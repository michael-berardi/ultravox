//! End-to-end Phonon-2 regression tests for short, quiet and cut-off audio.
//!
//! They run the real model, so they need Phonon-2 installed (the desktop app's models directory,
//! or `ULTRAVOX_MODELS_DIR`) and print a SKIPPED line otherwise. The source is the public
//! 3-second `speech.wav` fixture; variants are generated here, so no extra audio is committed.
//! A private voice note can be checked with `ULTRAVOX_PRIVATE_NOTE=<wav>` and
//! `ULTRAVOX_PRIVATE_NOTE_EXPECT=<text>`; that audio is never copied into the repo.

use std::path::{Path, PathBuf};
use ultravox_core::phonon;

const SR: u32 = 16_000;

fn models_dir() -> Option<PathBuf> {
    let dir = std::env::var_os("ULTRAVOX_MODELS_DIR")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME").map(|h| {
                PathBuf::from(h).join("Library/Application Support/com.imploselabs.ultravox/models")
            })
        })?;
    phonon::is_installed(&dir).then_some(dir)
}

fn speech() -> Vec<f32> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/speech.wav");
    let mut reader = hound::WavReader::open(path).expect("speech fixture");
    assert_eq!(reader.spec().sample_rate, SR);
    let scale = (1u32 << (reader.spec().bits_per_sample - 1)) as f32;
    reader
        .samples::<i32>()
        .map(|s| s.unwrap() as f32 / scale)
        .collect()
}

fn gain(db: f32) -> f32 {
    10f32.powf(db / 20.0)
}

fn write_fixture(name: &str, samples: &[f32]) -> PathBuf {
    let wav = std::env::temp_dir().join(format!(
        "ultravox-phonon-edges-{}-{name}.wav",
        std::process::id()
    ));
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: SR,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut writer = hound::WavWriter::create(&wav, spec).unwrap();
    for s in samples {
        writer
            .write_sample((s.clamp(-1.0, 1.0) * 32767.0).round() as i16)
            .unwrap();
    }
    writer.finalize().unwrap();
    wav
}

fn transcribe(models: &Path, name: &str, samples: &[f32]) -> (String, PathBuf) {
    let wav = write_fixture(name, samples);
    let text = phonon::transcribe_file(models, &wav, None).expect("transcribe");
    (text, wav)
}

fn check(models: &Path, name: &str, samples: &[f32], expect: &str) {
    let (text, wav) = transcribe(models, name, samples);
    let _ = std::fs::remove_file(&wav);
    assert!(
        text.to_lowercase().contains(expect),
        "{name}: expected to contain {expect:?}, got {text:?}"
    );
}

#[test]
fn quiet_short_and_cut_off_audio_still_transcribes() {
    let Some(models) = models_dir() else {
        eprintln!("SKIPPED: Phonon-2 is not installed");
        return;
    };
    let audio = speech();
    let quiet = |db: f32, a: &[f32]| a.iter().map(|v| v * gain(db)).collect::<Vec<_>>();

    check(&models, "full", &audio, "fellow americans");
    check(
        &models,
        "quiet-30",
        &quiet(-30.0, &audio),
        "fellow americans",
    );
    check(
        &models,
        "quiet-40",
        &quiet(-40.0, &audio),
        "fellow americans",
    );
    // Recording stopped right after the last word: no silence after (or before) the speech.
    let words_end = audio.len() * 62 / 100;
    let cut = &audio[..words_end];
    for (name, samples) in [
        ("cut-at-words", cut.to_vec()),
        ("cut-quiet-30", quiet(-30.0, cut)),
        ("cut-quiet-40", quiet(-40.0, cut)),
        (
            "cut-both-ends-quiet",
            quiet(-35.0, &audio[SR as usize / 3..words_end]),
        ),
        (
            "clipped",
            audio.iter().map(|v| (v * 8.0).clamp(-1.0, 1.0)).collect(),
        ),
    ] {
        let (text, wav) = transcribe(&models, name, &samples);
        let _ = std::fs::remove_file(&wav);
        assert!(
            !text.trim().is_empty(),
            "{name}: empty transcript for audible speech"
        );
    }
}

#[test]
fn true_silence_returns_a_named_error_before_model_loading() {
    let missing_models =
        std::env::temp_dir().join(format!("ultravox-no-model-{}", uuid::Uuid::new_v4()));
    // Synthetic PCM fixtures: no microphone or model is needed to reject silence.
    let dither: Vec<f32> = (0..SR as usize * 3)
        .map(|i| ((i * 7919 % 255) as f32 / 255.0 - 0.5) * 0.0001)
        .collect();
    for (name, samples) in [("silence", vec![0.0; SR as usize * 3]), ("dither", dither)] {
        let wav = write_fixture(name, &samples);
        let error = phonon::transcribe_file(&missing_models, &wav, None).unwrap_err();
        assert!(matches!(error, phonon::PhononError::Audio(_)), "{error}");
        assert!(error.to_string().contains("no speech"), "{error}");
        assert!(error.to_string().contains("silent"), "{error}");
        assert!(phonon::explain_empty(&wav).contains("silent"));
        std::fs::remove_file(&wav).unwrap();
    }
}

#[test]
fn private_quiet_note_matches_expected_transcript() {
    let (Some(note), Ok(expect)) = (
        std::env::var_os("ULTRAVOX_PRIVATE_NOTE"),
        std::env::var("ULTRAVOX_PRIVATE_NOTE_EXPECT"),
    ) else {
        eprintln!("SKIPPED: set ULTRAVOX_PRIVATE_NOTE and ULTRAVOX_PRIVATE_NOTE_EXPECT to run");
        return;
    };
    let Some(models) = models_dir() else {
        eprintln!("SKIPPED: Phonon-2 is not installed");
        return;
    };
    let text = phonon::transcribe_file(&models, Path::new(&note), None).expect("transcribe");
    assert_eq!(text, expect);
}
