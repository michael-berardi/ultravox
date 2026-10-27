//! Phonon-2: the English speech model (Fermion Research, CC-BY-4.0, derived from NVIDIA
//! Parakeet TDT 0.6B v3). A 164 MB download that runs on the CPU on macOS (Apple silicon),
//! Windows and Linux (x86-64). It replaces the previous English models (Parakeet v2 on macOS,
//! Whisper `base.en` elsewhere); multilingual transcription is unchanged.

mod container;
mod engine;
mod install;
mod mel;
mod native;
mod segment;

pub use install::{cleanup_superseded, install, install_progress, is_installed, MODEL_DIR_NAME};

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum PhononError {
    #[error("Phonon-2 is not supported here: {0}")]
    Unsupported(String),
    #[error("Phonon-2 model problem: {0}")]
    Model(String),
    #[error("Phonon-2 native engine: {0}")]
    Native(String),
    #[error("Phonon-2 audio: {0}")]
    Audio(String),
    #[error("Phonon-2 download: {0}")]
    Download(String),
    #[error("transcription cancelled")]
    Cancelled,
}

type Loaded = Option<(PathBuf, Arc<Mutex<engine::Engine>>)>;

fn loaded() -> &'static Mutex<Loaded> {
    static ENGINE: OnceLock<Mutex<Loaded>> = OnceLock::new();
    ENGINE.get_or_init(|| Mutex::new(None))
}

fn cancelled() -> &'static Mutex<HashSet<String>> {
    static CANCELLED: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    CANCELLED.get_or_init(|| Mutex::new(HashSet::new()))
}

/// Ask a running (or about-to-run) transcription for `recording_id` to stop at its next window.
pub fn cancel(recording_id: &str) {
    if let Ok(mut set) = cancelled().lock() {
        set.insert(recording_id.to_string());
    }
}

fn is_cancelled(recording_id: Option<&str>) -> bool {
    recording_id
        .and_then(|id| cancelled().lock().ok().map(|set| set.contains(id)))
        .unwrap_or(false)
}

fn engine_for(models_dir: &Path) -> Result<Arc<Mutex<engine::Engine>>, PhononError> {
    let model_dir = models_dir.join(MODEL_DIR_NAME);
    let mut slot = loaded()
        .lock()
        .map_err(|_| PhononError::Native("engine lock poisoned".into()))?;
    if let Some((dir, engine)) = slot.as_ref() {
        if *dir == model_dir {
            return Ok(engine.clone());
        }
    }
    if !is_installed(models_dir) {
        return Err(PhononError::Model(format!(
            "Phonon-2 is not downloaded ({} is missing)",
            model_dir.display()
        )));
    }
    let engine = Arc::new(Mutex::new(engine::Engine::load(
        &model_dir,
        &models_dir.join(".runtime"),
    )?));
    *slot = Some((model_dir, engine.clone()));
    Ok(engine)
}

/// Load the engine now so the first dictation does not pay for it.
pub fn prepare(models_dir: &Path) -> Result<(), PhononError> {
    engine_for(models_dir).map(|_| ())
}

/// Normalize and validate media before reading engine-ready 16 kHz mono samples.
fn read_audio(path: &Path) -> Result<Vec<f32>, PhononError> {
    let input =
        crate::prepare_transcription_audio(path).map_err(|e| PhononError::Audio(e.to_string()))?;
    let mut reader = hound::WavReader::open(input.path())
        .map_err(|e| PhononError::Audio(format!("{}: {e}", path.display())))?;
    let spec = reader.spec();
    let channels = spec.channels.max(1) as usize;
    let samples: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Float => reader
            .samples::<f32>()
            .collect::<Result<_, _>>()
            .map_err(|e| PhononError::Audio(e.to_string()))?,
        hound::SampleFormat::Int => {
            let scale = (1u64 << (spec.bits_per_sample - 1)) as f32;
            reader
                .samples::<i32>()
                .map(|s| s.map(|v| v as f32 / scale))
                .collect::<Result<_, _>>()
                .map_err(|e| PhononError::Audio(e.to_string()))?
        }
    };
    let mono: Vec<f32> = samples
        .chunks_exact(channels)
        .map(|frame| frame.iter().sum::<f32>() / channels as f32)
        .collect();
    Ok(segment::resample(
        &mono,
        spec.sample_rate as usize,
        engine::SAMPLE_RATE,
    ))
}

/// Silence added around a window that decoded to nothing, tried in order. The decoder can drop
/// a whole utterance when speech runs right up to the first or last frame (a short voice note
/// that was cut off at the end of the recording), so the retry gives it room instead of changing
/// the level: gain does not matter, edge context does.
const EDGE_RETRIES_MS: [(usize, usize); 2] = [(0, 1000), (300, 1000)];

/// Decode one window; if it comes back empty although it holds sound, retry with silence padded
/// around it. A window that is still empty afterwards really holds nothing recognisable.
fn decode_window(engine: &engine::Engine, window: &[f32]) -> Result<String, PhononError> {
    let text = engine.decode(window)?;
    if !text.trim().is_empty() {
        return Ok(text);
    }
    for (head_ms, tail_ms) in EDGE_RETRIES_MS {
        let padded = segment::pad_edges(window, head_ms, tail_ms);
        let text = engine.decode(&padded)?;
        if !text.trim().is_empty() {
            return Ok(text);
        }
    }
    Ok(String::new())
}

/// Why `transcribe_file` returned no text for `wav`, in plain words, for error messages.
pub fn explain_empty(wav: &Path) -> String {
    match read_audio(wav) {
        Err(e) => e.to_string(),
        Ok(audio) if audio.is_empty() => "the audio has no samples".into(),
        Ok(audio) => {
            let peak = audio.iter().fold(0f32, |m, v| m.max(v.abs()));
            let secs = audio.len() as f32 / engine::SAMPLE_RATE as f32;
            if peak < 1e-4 {
                format!("the audio is silent ({secs:.2}s, peak below -80 dBFS)")
            } else {
                format!(
                    "no speech was recognised in {secs:.2}s of audio (peak {:.1} dBFS), including after retrying with silence padded around it",
                    20.0 * peak.log10()
                )
            }
        }
    }
}

/// Transcribe an audio file with Phonon-2. Media is normalized without changing the source.
/// Long audio is cut into 25-35 s windows at pauses.
pub fn transcribe_file(
    models_dir: &Path,
    wav: &Path,
    recording_id: Option<&str>,
) -> Result<String, PhononError> {
    if is_cancelled(recording_id) {
        return Err(PhononError::Cancelled);
    }
    let audio = read_audio(wav)?;
    if audio.is_empty() {
        return Err(PhononError::Audio(format!(
            "{}: no speech: the audio has no samples",
            wav.display()
        )));
    }
    if audio.iter().fold(0f32, |m, v| m.max(v.abs())) < 1e-4 {
        return Err(PhononError::Audio(format!(
            "{}: no speech: the audio is silent (peak below -80 dBFS)",
            wav.display()
        )));
    }
    let engine = engine_for(models_dir)?;
    let engine = engine
        .lock()
        .map_err(|_| PhononError::Native("engine lock poisoned".into()))?;
    let mut texts = Vec::new();
    for (start, end) in segment::plan(&audio) {
        if is_cancelled(recording_id) {
            return Err(PhononError::Cancelled);
        }
        let window = &audio[start..end];
        if window.iter().fold(0f32, |m, v| m.max(v.abs())) < 1e-4 {
            continue;
        }
        let text = decode_window(&engine, window)?;
        if !text.trim().is_empty() {
            texts.push(text);
        }
    }
    if let Some(id) = recording_id {
        if let Ok(mut set) = cancelled().lock() {
            set.remove(id);
        }
    }
    let text = texts.join(" ");
    if text.trim().is_empty() {
        return Err(PhononError::Audio(format!(
            "{}: no speech was recognised, including after edge-padding retries",
            wav.display()
        )));
    }
    Ok(text)
}
