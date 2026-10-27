use async_trait::async_trait;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use thiserror::Error;
use uuid::Uuid;

/// Errors from the audio recording subsystem.
#[derive(Debug, Error)]
pub enum AudioError {
    #[error("permission denied")]
    PermissionDenied,
    #[error("no audio was captured; check that the microphone is selected and not muted")]
    SilentInput,
    #[error("device unavailable")]
    DeviceUnavailable,
    #[error("audio device error: {0}")]
    Device(String),
    #[error("audio startup error: {0}")]
    Startup(String),
    #[error("audio stream error: {0}")]
    Stream(String),
    #[error("recording already in progress")]
    AlreadyRecording,
    #[error("not recording")]
    NotRecording,
    #[error("audio format error: {0}")]
    Format(String),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

/// Configuration for an audio input capture session.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AudioInputConfig {
    pub sample_rate: u32,
    pub channels: u16,
    pub sample_format: SampleFormat,
    pub device_id: Option<String>,
}

impl Default for AudioInputConfig {
    fn default() -> Self {
        Self {
            sample_rate: 16_000,
            channels: 1,
            sample_format: SampleFormat::I16,
            device_id: None,
        }
    }
}

/// Audio sample format.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SampleFormat {
    I16,
    F32,
}

/// Handle to an active recording session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioRecording {
    pub id: String,
    pub output_path: PathBuf,
    pub start_time_ms: u64,
    pub duration_ms: Option<u64>,
}

/// Abstraction over a platform audio recorder.
#[async_trait]
pub trait AudioBackend: Send + Sync {
    /// List available input devices.
    async fn list_devices(&self) -> Result<Vec<AudioDeviceInfo>, AudioError>;

    /// Start recording to the given path with the supplied configuration.
    async fn start_recording(
        &mut self,
        config: AudioInputConfig,
        output_path: PathBuf,
    ) -> Result<AudioRecording, AudioError>;

    /// Stop the current recording and return the completed recording metadata.
    async fn stop_recording(&mut self) -> Result<AudioRecording, AudioError>;

    /// Whether the backend is currently recording.
    fn is_recording(&self) -> bool;
}

/// Metadata for a discovered audio input device.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioDeviceInfo {
    pub id: String,
    pub name: String,
    pub is_default: bool,
}

/// A stub audio backend that records no real audio but satisfies the trait.
#[derive(Debug, Default)]
pub struct StubAudioBackend {
    recording: Option<AudioRecording>,
}

#[async_trait]
impl AudioBackend for StubAudioBackend {
    async fn list_devices(&self) -> Result<Vec<AudioDeviceInfo>, AudioError> {
        Ok(vec![AudioDeviceInfo {
            id: "default".to_string(),
            name: "System Default".to_string(),
            is_default: true,
        }])
    }

    async fn start_recording(
        &mut self,
        _config: AudioInputConfig,
        output_path: PathBuf,
    ) -> Result<AudioRecording, AudioError> {
        if self.recording.is_some() {
            return Err(AudioError::AlreadyRecording);
        }
        let recording = AudioRecording {
            id: "stub-recording".to_string(),
            output_path,
            start_time_ms: 0,
            duration_ms: None,
        };
        self.recording = Some(recording.clone());
        Ok(recording)
    }

    async fn stop_recording(&mut self) -> Result<AudioRecording, AudioError> {
        match self.recording.take() {
            Some(mut rec) => {
                rec.duration_ms = Some(0);
                Ok(rec)
            }
            None => Err(AudioError::NotRecording),
        }
    }

    fn is_recording(&self) -> bool {
        self.recording.is_some()
    }
}

/// Convert our sample-format enum to a cpal sample format.
fn cpal_sample_format(format: SampleFormat) -> cpal::SampleFormat {
    match format {
        SampleFormat::I16 => cpal::SampleFormat::I16,
        SampleFormat::F32 => cpal::SampleFormat::F32,
    }
}

/// Derive a recording ID from the output path file stem when it is a valid UUID.
/// Falls back to a freshly generated UUID so callers always get a usable ID.
fn recording_id_from_output_path(output_path: &PathBuf) -> String {
    output_path
        .file_stem()
        .and_then(|s| s.to_str())
        .and_then(|s| Uuid::parse_str(s).ok())
        .unwrap_or_else(Uuid::new_v4)
        .to_string()
}

/// Mix interleaved multi-channel samples down to mono.
#[cfg(test)]
fn mix_to_mono(samples: &[f32], channels: u16) -> Vec<f32> {
    if channels <= 1 {
        return samples.to_vec();
    }
    let channels = channels as usize;
    let mut mono = Vec::with_capacity(samples.len() / channels);
    for chunk in samples.chunks_exact(channels) {
        let sum: f32 = chunk.iter().sum();
        mono.push(sum / channels as f32);
    }
    mono
}

/// Resample a mono signal to the target sample rate using linear interpolation.
#[cfg(test)]
fn resample_linear(samples: &[f32], source_rate: u32, target_rate: u32) -> Vec<f32> {
    if source_rate == target_rate || samples.is_empty() {
        return samples.to_vec();
    }
    let ratio = source_rate as f64 / target_rate as f64;
    let out_len = (samples.len() as f64 / ratio) as usize;
    let mut out = Vec::with_capacity(out_len);
    for i in 0..out_len {
        let src_pos = i as f64 * ratio;
        let src_idx = src_pos as usize;
        let frac = (src_pos - src_idx as f64) as f32;
        let s0 = samples[src_idx];
        let s1 = samples.get(src_idx + 1).copied().unwrap_or(s0);
        out.push(s0 + (s1 - s0) * frac);
    }
    out
}

/// Chunks the capture callback may queue ahead of the WAV writer. CoreAudio
/// delivers roughly 10 ms per chunk, so this absorbs about ten seconds of
/// writer stall (a few MB) before any audio is dropped; 32 chunks overflowed
/// whenever macOS throttled a hidden UltraVox.
const AUDIO_CHUNK_QUEUE_CAPACITY: usize = 1024;

/// Incremental mono linear resampler. It retains only the source frames needed
/// for the next interpolation point instead of the complete recording.
struct StreamingResampler {
    source_rate: u32,
    target_rate: u32,
    input_frames: u64,
    output_frames: u64,
    buffer_start: u64,
    buffer: VecDeque<f32>,
}

impl StreamingResampler {
    fn new(source_rate: u32, target_rate: u32) -> Result<Self, AudioError> {
        if source_rate == 0 || target_rate == 0 {
            return Err(AudioError::Format(
                "sample rates must be greater than zero".to_string(),
            ));
        }
        Ok(Self {
            source_rate,
            target_rate,
            input_frames: 0,
            output_frames: 0,
            buffer_start: 0,
            buffer: VecDeque::new(),
        })
    }

    fn desired_output_frames(&self) -> u64 {
        ((self.input_frames as u128 * self.target_rate as u128) / self.source_rate as u128)
            .min(u64::MAX as u128) as u64
    }

    fn push_interleaved<F>(
        &mut self,
        samples: &[f32],
        channels: u16,
        mut emit: F,
    ) -> Result<(), AudioError>
    where
        F: FnMut(f32) -> Result<(), AudioError>,
    {
        if channels == 0 {
            return Err(AudioError::Format(
                "audio input reported zero channels".to_string(),
            ));
        }
        let channels = channels as usize;
        if samples.len() % channels != 0 {
            return Err(AudioError::Format("incomplete audio frame".into()));
        }
        if samples.iter().any(|sample| !sample.is_finite()) {
            return Err(AudioError::Format(
                "audio contains non-finite samples".into(),
            ));
        }
        for frame in samples.chunks_exact(channels) {
            let mono = frame.iter().copied().sum::<f32>() / channels as f32;
            if !mono.is_finite() {
                return Err(AudioError::Format(
                    "resampler mix contains non-finite samples".into(),
                ));
            }
            self.buffer.push_back(mono);
            self.input_frames += 1;
            self.flush_ready(false, &mut emit)?;
        }
        Ok(())
    }

    fn finish<F>(&mut self, mut emit: F) -> Result<(), AudioError>
    where
        F: FnMut(f32) -> Result<(), AudioError>,
    {
        self.flush_ready(true, &mut emit)
    }

    fn flush_ready<F>(&mut self, finalizing: bool, emit: &mut F) -> Result<(), AudioError>
    where
        F: FnMut(f32) -> Result<(), AudioError>,
    {
        let desired = self.desired_output_frames();
        while self.output_frames < desired {
            let source_position =
                self.output_frames as f64 * self.source_rate as f64 / self.target_rate as f64;
            let source_index = source_position.floor() as u64;
            let local_index = source_index.saturating_sub(self.buffer_start) as usize;
            let Some(first) = self.buffer.get(local_index).copied() else {
                break;
            };
            let second = self.buffer.get(local_index + 1).copied();
            if second.is_none() && !finalizing {
                break;
            }
            let fraction = (source_position - source_index as f64) as f32;
            let sample = first + (second.unwrap_or(first) - first) * fraction;
            if !sample.is_finite() {
                return Err(AudioError::Format(
                    "resampler output contains non-finite samples".into(),
                ));
            }
            emit(sample)?;
            self.output_frames += 1;
            self.prune_consumed_frames();
        }
        Ok(())
    }

    fn prune_consumed_frames(&mut self) {
        let next_source_index = (self.output_frames as f64 * self.source_rate as f64
            / self.target_rate as f64)
            .floor() as u64;
        while self.buffer_start < next_source_index && self.buffer.len() > 1 {
            self.buffer.pop_front();
            self.buffer_start += 1;
        }
    }

    #[cfg(test)]
    fn buffered_frames(&self) -> usize {
        self.buffer.len()
    }
}

fn queue_audio_chunk(
    sender: &std::sync::mpsc::SyncSender<Vec<f32>>,
    chunk: Vec<f32>,
    overflowed_chunks: &AtomicU32,
) {
    if matches!(
        sender.try_send(chunk),
        Err(std::sync::mpsc::TrySendError::Full(_))
    ) {
        overflowed_chunks.fetch_add(1, Ordering::Relaxed);
    }
}

/// Compute a normalized microphone level in [0, 1] from interleaved f32 samples.
///
/// Samples are mixed down to mono per frame, then converted from RMS to a
/// decibel meter. The -60 dB floor suppresses room noise while the -3 dB
/// ceiling leaves normal speech in the visually useful middle of the range.
fn compute_audio_level(samples: &[f32], channels: u16) -> f32 {
    if samples.is_empty() || channels == 0 {
        return 0.0;
    }
    let channels = channels as usize;
    let frames = samples.len() / channels;
    if frames == 0 {
        return 0.0;
    }
    let mut sum_sq = 0.0f64;
    for chunk in samples.chunks_exact(channels) {
        let avg = chunk.iter().map(|sample| *sample as f64).sum::<f64>() / channels as f64;
        sum_sq += avg * avg;
    }
    let rms = (sum_sq / frames as f64).sqrt() as f32;

    const FLOOR_RMS: f32 = 1e-3;
    const FLOOR_DB: f32 = -60.0;
    const CEILING_DB: f32 = -3.0;
    if rms <= FLOOR_RMS {
        return 0.0;
    }
    let decibels = 20.0 * rms.log10();
    ((decibels - FLOOR_DB) / (CEILING_DB - FLOOR_DB)).clamp(0.0, 1.0)
}

#[cfg(test)]
fn validate_captured_audio(samples: &[f32]) -> Result<(), AudioError> {
    if samples.is_empty() || samples.iter().all(|sample| sample.abs() < 1e-8) {
        return Err(AudioError::SilentInput);
    }
    Ok(())
}

#[derive(Debug, Default)]
pub struct CpalAudioBackend {
    recording: Option<CpalRecordingState>,
    pending: Option<RecorderWorker>,
    current_level: Arc<AtomicU32>,
}

impl CpalAudioBackend {
    /// Create a new cpal-backed audio backend using the default host.
    pub fn new() -> Self {
        Self {
            recording: None,
            pending: None,
            current_level: Arc::new(AtomicU32::new(0)),
        }
    }

    /// Return the current microphone input level as a normalized value in [0, 1].
    ///
    /// The value is lock-free and updated from the live CPAL input callback. It
    /// is reset to 0 at recording lifecycle boundaries and reports 0 when the
    /// backend is not recording.
    pub fn current_input_level(&self) -> f32 {
        if !self.is_recording() {
            return 0.0;
        }
        f32::from_bits(self.current_level.load(Ordering::Relaxed))
    }

    fn store_level(&self, value: f32) {
        self.current_level.store(value.to_bits(), Ordering::Relaxed);
    }

    fn now_ms() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64
    }

    /// Runs the full recording lifecycle on a dedicated OS thread so the cpal
    /// `Stream` (which is `!Send` on macOS CoreAudio) never crosses a thread
    /// boundary. The caller passes the stop channel receiver and gets back the
    /// finalized recording metadata (or an error) via a tokio oneshot.
    fn record_on_thread(
        config: AudioInputConfig,
        output_path: PathBuf,
        id: String,
        start_time_ms: u64,
        stop_rx: std::sync::mpsc::Receiver<()>,
        level_shared: Arc<AtomicU32>,
        ready_tx: tokio::sync::oneshot::Sender<()>,
    ) -> Result<AudioRecording, AudioError> {
        let sample_rate = config.sample_rate;
        let channels = config.channels;

        check_startup_cancelled(&stop_rx)?;
        if sample_rate == 0 || channels == 0 {
            return Err(AudioError::Format(
                "sample rate and channels must be nonzero".into(),
            ));
        }
        let host = cpal::default_host();
        let device = if let Some(device_id) = &config.device_id {
            let name_target = device_id.strip_prefix("cpal:").unwrap_or(device_id);
            let devices = host
                .input_devices()
                .map_err(|e| AudioError::Device(format!("enumerate input devices: {e}")))?;
            let mut selected = None;
            for device in devices {
                // A device whose name cannot be read cannot be the one requested.
                let Ok(name) = device.name() else { continue };
                if name == name_target {
                    selected = Some(device);
                    break;
                }
            }
            selected.ok_or_else(|| {
                AudioError::Device(format!("input device not found: {name_target}"))
            })?
        } else {
            host.default_input_device()
                .ok_or_else(|| AudioError::Device("no default input device".into()))?
        };
        check_startup_cancelled(&stop_rx)?;
        let supported_configs = device
            .supported_input_configs()
            .map_err(|e| AudioError::Device(format!("query supported input configs: {e}")))?;
        let preferred_format = cpal_sample_format(config.sample_format);
        let mut selected_config = None;
        for cfg in supported_configs {
            let supports_rate =
                cfg.min_sample_rate().0 <= sample_rate && cfg.max_sample_rate().0 >= sample_rate;
            if cfg.channels() == channels && supports_rate {
                let candidate = cfg.with_sample_rate(cpal::SampleRate(sample_rate));
                if candidate.sample_format() == preferred_format {
                    selected_config = Some(candidate);
                    break;
                }
                if selected_config.is_none() {
                    selected_config = Some(candidate);
                }
            }
        }
        let selected_config = match selected_config {
            Some(config) => config,
            None => device
                .default_input_config()
                .map_err(|e| AudioError::Device(format!("query default input config: {e}")))?,
        };
        check_startup_cancelled(&stop_rx)?;

        let actual_sample_rate = selected_config.sample_rate().0;
        let actual_channels = selected_config.channels();

        let (sample_tx, sample_rx) =
            std::sync::mpsc::sync_channel::<Vec<f32>>(AUDIO_CHUNK_QUEUE_CAPACITY);
        let overflowed_chunks = Arc::new(AtomicU32::new(0));
        // Error callbacks are not the realtime sample callback. Retain the
        // backend's actual cause, including errors delivered during play().
        let (stream_error_tx, stream_error_rx) = std::sync::mpsc::channel();
        let err_fn = move |err: cpal::StreamError| {
            let _ = stream_error_tx.send(err);
        };
        let sample_tx_clone = sample_tx.clone();
        let overflowed_chunks_clone = Arc::clone(&overflowed_chunks);

        let stream = match selected_config.sample_format() {
            cpal::SampleFormat::F32 => {
                let cfg = selected_config.config();
                let level_shared = Arc::clone(&level_shared);
                device.build_input_stream(
                    &cfg,
                    move |data: &[f32], _: &cpal::InputCallbackInfo| {
                        let level = compute_audio_level(data, actual_channels);
                        level_shared.store(level.to_bits(), Ordering::Relaxed);
                        queue_audio_chunk(
                            &sample_tx_clone,
                            data.to_vec(),
                            &overflowed_chunks_clone,
                        );
                    },
                    err_fn,
                    None,
                )
            }
            cpal::SampleFormat::I16 => {
                let cfg = selected_config.config();
                let level_shared = Arc::clone(&level_shared);
                device.build_input_stream(
                    &cfg,
                    move |data: &[i16], _: &cpal::InputCallbackInfo| {
                        let chunk: Vec<f32> =
                            data.iter().map(|&s| s as f32 / i16::MAX as f32).collect();
                        let level = compute_audio_level(&chunk, actual_channels);
                        level_shared.store(level.to_bits(), Ordering::Relaxed);
                        queue_audio_chunk(&sample_tx_clone, chunk, &overflowed_chunks_clone);
                    },
                    err_fn,
                    None,
                )
            }
            cpal::SampleFormat::U16 => {
                let cfg = selected_config.config();
                let level_shared = Arc::clone(&level_shared);
                device.build_input_stream(
                    &cfg,
                    move |data: &[u16], _: &cpal::InputCallbackInfo| {
                        let chunk: Vec<f32> = data
                            .iter()
                            .map(|&s| (s as f32 - 32768.0) / 32768.0)
                            .collect();
                        let level = compute_audio_level(&chunk, actual_channels);
                        level_shared.store(level.to_bits(), Ordering::Relaxed);
                        queue_audio_chunk(&sample_tx_clone, chunk, &overflowed_chunks_clone);
                    },
                    err_fn,
                    None,
                )
            }
            _ => {
                return Err(AudioError::Format(
                    "unsupported cpal sample format".to_string(),
                ))
            }
        }
        .map_err(|e| AudioError::Stream(format!("build input stream: {e}")))?;

        // Drop our copy of sample_tx so the only remaining sender is the stream
        // callback; the receiver can then detect end-of-stream by disconnect.
        drop(sample_tx);

        let spec = hound::WavSpec {
            channels: 1,
            sample_rate,
            bits_per_sample: 32,
            sample_format: hound::SampleFormat::Float,
        };
        // Declared before the writer so the writer is closed before removal.
        let mut output_guard = StartupOutputGuard::new(output_path.clone());
        let mut writer = hound::WavWriter::create(&output_path, spec)
            .map_err(|e| AudioError::Format(format!("wav writer: {e}")))?;
        let mut resampler = StreamingResampler::new(actual_sample_rate, sample_rate)?;
        check_startup_cancelled(&stop_rx)?;
        stream
            .play()
            .map_err(|e| AudioError::Stream(format!("play stream: {e}")))?;
        check_startup_cancelled(&stop_rx)?;
        check_stream_error(&stream_error_rx)?;
        ready_tx
            .send(())
            .map_err(|_| AudioError::Startup("startup cancelled".into()))?;
        // Recording has started: from here the WAV belongs to the caller, which
        // decides what to do with it on stop.
        output_guard.keep();

        let summary = capture_to_wav(
            stream,
            &stop_rx,
            &sample_rx,
            &stream_error_rx,
            actual_channels,
            &mut resampler,
            &mut writer,
        )?;
        let CaptureSummary {
            frames,
            max_abs,
            stream_failure,
        } = summary;

        require_captured_audio(frames, max_abs, stream_failure)?;
        let dropped = overflowed_chunks.load(Ordering::Relaxed);
        if dropped > 0 {
            return Err(AudioError::Format(format!(
                "audio capture buffer overflowed; dropped {dropped} chunks"
            )));
        }
        writer
            .finalize()
            .map_err(|e| AudioError::Format(format!("wav finalize: {e}")))?;

        let duration_ms = frames * 1000 / sample_rate.max(1) as u64;

        Ok(AudioRecording {
            id,
            output_path,
            start_time_ms,
            duration_ms: Some(duration_ms),
        })
    }
}

/// Removes the output WAV when startup fails, is cancelled, or times out.
struct StartupOutputGuard {
    path: PathBuf,
    keep: bool,
}

impl StartupOutputGuard {
    fn new(path: PathBuf) -> Self {
        Self { path, keep: false }
    }

    fn keep(&mut self) {
        self.keep = true;
    }
}

impl Drop for StartupOutputGuard {
    fn drop(&mut self) {
        if !self.keep {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

struct CaptureSummary {
    frames: u64,
    max_abs: f32,
    /// Stream error that ended capture after it had started, if any.
    stream_failure: Option<AudioError>,
}

/// Silence can come from a muted or unavailable input as well as an OS privacy
/// decision. Permission is checked explicitly before recording, so never infer
/// it from sample content. A stream failure only fails the recording when
/// nothing usable was captured before it; then it is the more useful cause.
fn require_captured_audio(
    frames: u64,
    max_abs: f32,
    stream_failure: Option<AudioError>,
) -> Result<(), AudioError> {
    if frames == 0 || max_abs < 1e-8 {
        return Err(stream_failure.unwrap_or(AudioError::SilentInput));
    }
    Ok(())
}

/// Captures until stopped or until the stream fails, then drains and flushes
/// every sample already delivered. A stream error after startup is not fatal
/// here: it is reported in the summary so the caller can keep what was recorded.
fn capture_to_wav<W, S>(
    stream: S,
    stop_rx: &std::sync::mpsc::Receiver<()>,
    sample_rx: &std::sync::mpsc::Receiver<Vec<f32>>,
    stream_error_rx: &std::sync::mpsc::Receiver<cpal::StreamError>,
    channels: u16,
    resampler: &mut StreamingResampler,
    writer: &mut hound::WavWriter<W>,
) -> Result<CaptureSummary, AudioError>
where
    W: std::io::Write + std::io::Seek,
{
    let mut frames = 0u64;
    let mut max_abs = 0.0f32;
    let mut stream_failure = None;
    let mut process_chunk = |chunk: Vec<f32>| -> Result<(), AudioError> {
        resampler.push_interleaved(&chunk, channels, |sample| {
            let sample = sample.clamp(-1.0, 1.0);
            max_abs = max_abs.max(sample.abs());
            writer
                .write_sample(sample)
                .map_err(|e| AudioError::Format(format!("wav write: {e}")))?;
            frames += 1;
            Ok(())
        })
    };

    loop {
        if let Err(error) = check_stream_error(stream_error_rx) {
            stream_failure = Some(error);
            break;
        }
        match stop_rx.try_recv() {
            Ok(()) | Err(std::sync::mpsc::TryRecvError::Disconnected) => break,
            Err(std::sync::mpsc::TryRecvError::Empty) => {}
        }
        match sample_rx.recv_timeout(std::time::Duration::from_millis(10)) {
            Ok(chunk) => process_chunk(chunk)?,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
    // Drop the stream first to stop the audio callback and disconnect the
    // channel sender. Any samples already in flight are delivered before
    // the sender is dropped, so we can then drain the channel completely.
    drop(stream);
    if stream_failure.is_none() {
        stream_failure = check_stream_error(stream_error_rx).err();
    }
    while let Ok(chunk) = sample_rx.try_recv() {
        process_chunk(chunk)?;
    }
    drop(process_chunk);
    resampler.finish(|sample| {
        let sample = sample.clamp(-1.0, 1.0);
        max_abs = max_abs.max(sample.abs());
        writer
            .write_sample(sample)
            .map_err(|e| AudioError::Format(format!("wav write: {e}")))?;
        frames += 1;
        Ok(())
    })?;
    Ok(CaptureSummary {
        frames,
        max_abs,
        stream_failure,
    })
}

const STARTUP_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

fn check_startup_cancelled(stop_rx: &std::sync::mpsc::Receiver<()>) -> Result<(), AudioError> {
    match stop_rx.try_recv() {
        Err(std::sync::mpsc::TryRecvError::Empty) => Ok(()),
        _ => Err(AudioError::Startup("startup cancelled".into())),
    }
}

fn check_stream_error(
    errors: &std::sync::mpsc::Receiver<cpal::StreamError>,
) -> Result<(), AudioError> {
    match errors.try_recv() {
        Ok(error) => Err(AudioError::Stream(error.to_string())),
        Err(_) => Ok(()),
    }
}

/// Dropping an in-flight start future must request shutdown, not detach capture.
struct StopOnDrop(Option<std::sync::mpsc::Sender<()>>);
impl Drop for StopOnDrop {
    fn drop(&mut self) {
        if let Some(sender) = &self.0 {
            let _ = sender.send(());
        }
    }
}

struct ResetLevel(Arc<AtomicU32>);
impl Drop for ResetLevel {
    fn drop(&mut self) {
        self.0.store(0, Ordering::Relaxed);
    }
}

#[derive(Debug)]
struct RecorderWorker {
    stop_tx: std::sync::mpsc::Sender<()>,
    // This supervisor owns and joins the native thread even if its task handle
    // is dropped. A timed-out start retains this worker until cleanup finishes.
    handle: tokio::task::JoinHandle<Result<AudioRecording, AudioError>>,
}
impl Drop for RecorderWorker {
    fn drop(&mut self) {
        let _ = self.stop_tx.send(());
    }
}

#[derive(Debug)]
struct CpalRecordingState {
    worker: RecorderWorker,
}

impl CpalAudioBackend {
    // A mockable orchestration boundary: tests never enumerate/open a device.
    async fn start_worker<F>(
        &mut self,
        output_path: PathBuf,
        timeout: std::time::Duration,
        run: F,
    ) -> Result<AudioRecording, AudioError>
    where
        F: FnOnce(
                std::sync::mpsc::Receiver<()>,
                Arc<AtomicU32>,
                tokio::sync::oneshot::Sender<()>,
            ) -> Result<AudioRecording, AudioError>
            + Send
            + 'static,
    {
        if self.recording.is_some() {
            return Err(AudioError::AlreadyRecording);
        }
        if let Some(pending) = &self.pending {
            if !pending.handle.is_finished() {
                return Err(AudioError::Startup(
                    "previous recorder is still shutting down".into(),
                ));
            }
        }
        self.pending.take();
        self.store_level(0.0);
        let level = Arc::clone(&self.current_level);
        let (stop_tx, stop_rx) = std::sync::mpsc::channel();
        let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
        let (result_tx, result_rx) = tokio::sync::oneshot::channel();
        let native_thread = std::thread::Builder::new()
            .name("ultravox-recorder".into())
            .spawn(move || {
                let _reset = ResetLevel(Arc::clone(&level));
                let result = run(stop_rx, level, ready_tx);
                let _ = result_tx.send(result);
            })
            .map_err(|e| AudioError::Startup(format!("spawn recorder thread: {e}")))?;
        let handle = tokio::spawn(async move {
            let result = result_rx.await;
            // The result sender has exited its recording lifecycle (or panicked).
            // Joining now never waits for a running device operation.
            if let Err(panic) = native_thread.join() {
                let cause = panic
                    .downcast_ref::<String>()
                    .map(String::as_str)
                    .or_else(|| panic.downcast_ref::<&str>().copied())
                    .unwrap_or("non-string panic");
                return Err(AudioError::Startup(format!(
                    "recorder thread panicked: {cause}"
                )));
            }
            result.map_err(|e| AudioError::Startup(format!("recorder result channel: {e}")))?
        });
        let mut cancel = StopOnDrop(Some(stop_tx.clone()));
        self.pending = Some(RecorderWorker { stop_tx, handle });
        let worker = self.pending.as_mut().expect("pending recorder installed");
        let startup = async {
            tokio::select! {
                biased;
                result = &mut worker.handle => {
                    match result.map_err(|e| AudioError::Startup(format!("recorder supervisor: {e}")))? {
                        Err(error) => Err(error),
                        Ok(_) => Err(AudioError::Startup("recorder exited before startup completed".into())),
                    }
                }
                ready = ready_rx => {
                    if ready.is_ok() {
                        Ok(())
                    } else {
                        match (&mut worker.handle).await
                            .map_err(|e| AudioError::Startup(format!("recorder supervisor: {e}")))? {
                            Err(error) => Err(error),
                            Ok(_) => Err(AudioError::Startup("recorder exited without readiness".into())),
                        }
                    }
                }
            }
        };
        match tokio::time::timeout(timeout, startup).await {
            Ok(Ok(())) => {
                cancel.0.take();
                self.recording = Some(CpalRecordingState {
                    worker: self.pending.take().expect("ready recorder installed"),
                });
                Ok(AudioRecording {
                    id: recording_id_from_output_path(&output_path),
                    output_path,
                    start_time_ms: Self::now_ms(),
                    duration_ms: None,
                })
            }
            Ok(Err(error)) => {
                self.pending.take();
                self.store_level(0.0);
                Err(error)
            }
            Err(_) => {
                self.store_level(0.0);
                Err(AudioError::Startup(format!(
                    "recorder readiness timed out after {} ms",
                    timeout.as_millis()
                )))
            }
        }
    }
}

#[async_trait]
impl AudioBackend for CpalAudioBackend {
    async fn list_devices(&self) -> Result<Vec<AudioDeviceInfo>, AudioError> {
        let host = cpal::default_host();
        let mut devices = Vec::new();
        let default_name = host
            .default_input_device()
            .and_then(|device| device.name().ok());
        let list = host
            .input_devices()
            .map_err(|e| AudioError::Device(format!("enumerate input devices: {e}")))?;
        for device in list {
            // Skip devices whose name cannot be read instead of failing the list.
            let Ok(name) = device.name() else { continue };
            let id = format!("cpal:{name}");
            let is_default = default_name.as_ref() == Some(&name);
            devices.push(AudioDeviceInfo {
                id,
                name,
                is_default,
            });
        }
        Ok(devices)
    }

    async fn start_recording(
        &mut self,
        config: AudioInputConfig,
        output_path: PathBuf,
    ) -> Result<AudioRecording, AudioError> {
        if self.recording.is_some() {
            return Err(AudioError::AlreadyRecording);
        }
        if let Some(parent) = output_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let id = recording_id_from_output_path(&output_path);
        let start_time_ms = Self::now_ms();
        let thread_output = output_path.clone();
        let thread_id = id.clone();
        let mut recording = self
            .start_worker(output_path, STARTUP_TIMEOUT, move |stop, level, ready| {
                Self::record_on_thread(
                    config,
                    thread_output,
                    thread_id,
                    start_time_ms,
                    stop,
                    level,
                    ready,
                )
            })
            .await?;
        // Match the ID/timestamp used by the WAV worker, including non-UUID paths.
        recording.start_time_ms = start_time_ms;
        recording.id = id;
        Ok(recording)
    }

    async fn stop_recording(&mut self) -> Result<AudioRecording, AudioError> {
        let mut state = self.recording.take().ok_or(AudioError::NotRecording)?;
        let _ = state.worker.stop_tx.send(());
        let result = (&mut state.worker.handle)
            .await
            .map_err(|e| AudioError::Startup(format!("recorder supervisor: {e}")));
        self.store_level(0.0);
        let result = result??;

        let metadata = std::fs::metadata(&result.output_path)?;
        if metadata.len() == 0 {
            return Err(AudioError::Format("recorded WAV file is empty".to_string()));
        }
        if let Some(duration_ms) = result.duration_ms {
            if duration_ms < 100 {
                return Err(AudioError::Format(
                    "recording too short (less than 100ms)".to_string(),
                ));
            }
        }
        Ok(result)
    }

    fn is_recording(&self) -> bool {
        self.recording
            .as_ref()
            .is_some_and(|state| !state.worker.handle.is_finished())
    }
}

/// Sample rate of WAV files produced by media imports, matching recorded audio.
pub const IMPORT_SAMPLE_RATE: u32 = 16_000;

/// Maximum size of a media file accepted for import (2 GB, matching the URL
/// download limit).
pub const IMPORT_MAX_BYTES: u64 = 2 * 1024 * 1024 * 1024;

/// Build the codec registry used for media imports: all enabled Symphonia
/// codecs plus the libopus-backed Opus decoder (WhatsApp voice notes, WebM).
fn import_codec_registry() -> symphonia::core::codecs::registry::CodecRegistry {
    let mut registry = symphonia::core::codecs::registry::CodecRegistry::new();
    symphonia::default::register_enabled_codecs(&mut registry);
    registry.register_audio_decoder::<symphonia_adapter_libopus::OpusDecoder>();
    registry
}

/// Telegram may omit Ogg's end-of-stream flag despite complete audio pages.
/// Permit that EOF only after verifying every physical page, checksum, sequence,
/// and packet boundary. A genuinely truncated/corrupt stream remains an error.
fn validate_complete_ogg(source: &std::path::Path) -> Result<bool, AudioError> {
    use std::io::{Read, Seek, SeekFrom};
    let invalid = |cause: &str| AudioError::Format(format!("invalid OGG: {cause}"));
    let mut file = std::fs::File::open(source)?;
    if file.metadata()?.len() > IMPORT_MAX_BYTES {
        return Err(invalid("input exceeds the 2 GB limit"));
    }
    let mut magic = [0u8; 4];
    if file.read(&mut magic)? != 4 || &magic != b"OggS" {
        return Ok(false);
    }
    file.seek(SeekFrom::Start(0))?;
    let mut table = [0u32; 256];
    for (index, entry) in table.iter_mut().enumerate() {
        let mut crc = (index as u32) << 24;
        for _ in 0..8 {
            crc = if crc & 0x8000_0000 != 0 {
                (crc << 1) ^ 0x04c1_1db7
            } else {
                crc << 1
            };
        }
        *entry = crc;
    }
    let mut streams = std::collections::HashMap::<u32, (u32, bool)>::new();
    loop {
        let mut header = [0u8; 27];
        if file.read(&mut header[..1])? == 0 {
            break;
        }
        file.read_exact(&mut header[1..])
            .map_err(|_| invalid("truncated page header"))?;
        if &header[..4] != b"OggS" || header[4] != 0 || header[5] & !7 != 0 {
            return Err(invalid("unsupported or corrupt page header"));
        }
        let mut lacing = vec![0u8; header[26] as usize];
        file.read_exact(&mut lacing)
            .map_err(|_| invalid("truncated segment table"))?;
        let mut payload = vec![0u8; lacing.iter().map(|v| *v as usize).sum()];
        file.read_exact(&mut payload)
            .map_err(|_| invalid("truncated page payload"))?;
        let serial = u32::from_le_bytes(header[14..18].try_into().unwrap());
        let sequence = u32::from_le_bytes(header[18..22].try_into().unwrap());
        let continued = header[5] & 1 != 0;
        if let Some((previous, unfinished)) = streams.get(&serial) {
            if sequence != previous.wrapping_add(1) || continued != *unfinished {
                return Err(invalid("missing page or broken packet continuation"));
            }
        } else if continued {
            return Err(invalid("input starts with an incomplete packet"));
        }
        let expected = u32::from_le_bytes(header[22..26].try_into().unwrap());
        header[22..26].fill(0);
        let mut crc = 0u32;
        for byte in header.iter().chain(lacing.iter()).chain(payload.iter()) {
            crc = (crc << 8) ^ table[((crc >> 24) as u8 ^ *byte) as usize];
        }
        if crc != expected {
            return Err(invalid("page checksum mismatch"));
        }
        streams.insert(serial, (sequence, lacing.last() == Some(&255)));
    }
    if streams.is_empty() || streams.values().any(|(_, unfinished)| *unfinished) {
        return Err(invalid("audio ends with an incomplete packet"));
    }
    Ok(true)
}

/// Decode a media file (wav, mp3, m4a/aac, flac, ogg, opus, webm, mp4 audio,
/// aiff, caf, alac, …) into a 16 kHz mono PCM16 WAV at `dest`, returning the
/// decoded duration in milliseconds. On failure, any partial `dest` file is
/// removed.
pub fn decode_media_file_to_wav(
    source: &std::path::Path,
    dest: &std::path::Path,
) -> Result<u64, AudioError> {
    use symphonia::core::codecs::audio::AudioDecoderOptions;
    use symphonia::core::errors::Error as SymphoniaError;
    use symphonia::core::formats::probe::Hint;
    use symphonia::core::formats::{FormatOptions, TrackType};
    use symphonia::core::io::MediaSourceStream;
    use symphonia::core::meta::MetadataOptions;

    let mut created_destination = false;
    let result = (|| {
        // Symphonia can return partial PCM for a damaged WAV. Validate the
        // complete RIFF/sample data before creating any derived output.
        crate::audio_input::validate_wav_if_riff(source)?;
        let complete_ogg = validate_complete_ogg(source)?;
        let file = std::fs::File::open(source)?;
        let mss = MediaSourceStream::new(Box::new(file), Default::default());
        let mut hint = Hint::new();
        if let Some(extension) = source.extension().and_then(|ext| ext.to_str()) {
            hint.with_extension(extension);
        }
        let mut format = symphonia::default::get_probe()
            .probe(
                &hint,
                mss,
                FormatOptions::default(),
                MetadataOptions::default(),
            )
            .map_err(|e| {
                AudioError::Format(format!("unsupported or unreadable media file: {e}"))
            })?;
        let track = format
            .default_track(TrackType::Audio)
            .ok_or_else(|| AudioError::Format("media file has no audio track".to_string()))?;
        let track_id = track.id;
        let codec_params = track
            .codec_params
            .as_ref()
            .ok_or_else(|| AudioError::Format("audio track is missing codec parameters".into()))?
            .audio()
            .ok_or_else(|| AudioError::Format("track is not a supported audio codec".into()))?;
        let source_rate = codec_params
            .sample_rate
            .ok_or_else(|| AudioError::Format("audio track has no sample rate".to_string()))?;
        let channels = codec_params
            .channels
            .as_ref()
            .map(|c| c.count() as u16)
            .filter(|c| *c > 0)
            .ok_or_else(|| AudioError::Format("audio track has no channels".to_string()))?;
        let mut decoder = import_codec_registry()
            .make_audio_decoder(codec_params, &AudioDecoderOptions::default())
            .map_err(|e| AudioError::Format(format!("no decoder for this audio codec: {e}")))?;

        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: IMPORT_SAMPLE_RATE,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        // Never overwrite the source, a hard-link alias, or an existing
        // caller-owned destination. Only newly created derived audio is ours.
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let output = options
            .open(dest)
            .map_err(|e| AudioError::Format(format!("cannot create new normalized WAV: {e}")))?;
        created_destination = true;
        let mut writer = hound::WavWriter::new(std::io::BufWriter::new(output), spec)
            .map_err(|e| AudioError::Format(format!("wav writer: {e}")))?;
        let mut resampler = StreamingResampler::new(source_rate, IMPORT_SAMPLE_RATE)?;
        let mut frames = 0u64;
        let mut interleaved: Vec<f32> = Vec::new();

        loop {
            let packet = match format.next_packet() {
                Ok(Some(packet)) => packet,
                Ok(None) => break,
                Err(SymphoniaError::ResetRequired) => {
                    return Err(AudioError::Format(
                        "chained audio streams are not supported".to_string(),
                    ));
                }
                // Complete checksum-verified Telegram Ogg pages may have no
                // EOS flag. Symphonia reports EOF after their final packet.
                Err(SymphoniaError::IoError(e))
                    if complete_ogg
                        && frames > 0
                        && e.kind() == std::io::ErrorKind::UnexpectedEof =>
                {
                    break
                }
                // No broad salvage: truncated WAV/pages/packets are errors.
                Err(e) => {
                    return Err(AudioError::Format(format!("could not read media: {e}")));
                }
            };
            if packet.track_id != track_id {
                continue;
            }
            let decoded = match decoder.decode_ref(&packet.as_packet_ref()) {
                Ok(decoded) => decoded,
                Err(e) => {
                    return Err(AudioError::Format(format!("audio decode failed: {e}")));
                }
            };
            if decoded.frames() == 0 {
                continue;
            }
            let spec = decoded.spec();
            if spec.rate() != source_rate || spec.channels().count() as u16 != channels {
                return Err(AudioError::Format(
                    "media file changes audio format mid-stream".to_string(),
                ));
            }
            interleaved.clear();
            decoded.copy_to_vec_interleaved(&mut interleaved);
            if interleaved.iter().any(|sample| !sample.is_finite()) {
                return Err(AudioError::Format(
                    "audio contains non-finite samples".into(),
                ));
            }
            resampler.push_interleaved(&interleaved, channels, |sample| {
                writer
                    .write_sample((sample.clamp(-1.0, 1.0) * 32767.0).round() as i16)
                    .map_err(|_| AudioError::Format("wav write failed".to_string()))?;
                frames += 1;
                Ok(())
            })?;
        }

        resampler.finish(|sample| {
            writer
                .write_sample((sample.clamp(-1.0, 1.0) * 32767.0).round() as i16)
                .map_err(|_| AudioError::Format("wav write failed".to_string()))?;
            frames += 1;
            Ok(())
        })?;
        writer
            .finalize()
            .map_err(|e| AudioError::Format(format!("wav finalize: {e}")))?;
        if frames == 0 {
            return Err(AudioError::Format(
                "media file decoded to no audio samples".to_string(),
            ));
        }
        Ok(frames.saturating_mul(1_000) / IMPORT_SAMPLE_RATE as u64)
    })();

    if result.is_err() && created_destination {
        let _ = std::fs::remove_file(dest);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn stub_backend_recording_lifecycle() {
        let mut backend = StubAudioBackend::default();
        assert!(!backend.is_recording());
        let devices = backend.list_devices().await.unwrap();
        assert_eq!(devices.len(), 1);
        let rec = backend
            .start_recording(AudioInputConfig::default(), PathBuf::from("/tmp/test.wav"))
            .await
            .unwrap();
        assert!(backend.is_recording());
        assert_eq!(rec.output_path, PathBuf::from("/tmp/test.wav"));
        let finished = backend.stop_recording().await.unwrap();
        assert!(!backend.is_recording());
        assert!(finished.duration_ms.is_some());
    }

    #[tokio::test]
    async fn double_start_rejected() {
        let mut backend = StubAudioBackend::default();
        backend
            .start_recording(AudioInputConfig::default(), PathBuf::from("/tmp/test.wav"))
            .await
            .unwrap();
        let result = backend
            .start_recording(AudioInputConfig::default(), PathBuf::from("/tmp/test2.wav"))
            .await;
        assert!(matches!(result, Err(AudioError::AlreadyRecording)));
    }

    #[test]
    fn recording_id_extracts_output_path_uuid() {
        let uuid = Uuid::new_v4();
        let path = PathBuf::from(format!("/tmp/recordings/{}.wav", uuid));
        assert_eq!(recording_id_from_output_path(&path), uuid.to_string());
    }

    #[test]
    fn audio_level_normalization() {
        // Silence and signals below the noise gate report 0.
        assert_eq!(compute_audio_level(&[], 1), 0.0);
        assert_eq!(compute_audio_level(&[0.0, 0.0, 0.0, 0.0], 1), 0.0);
        assert_eq!(compute_audio_level(&[1e-4; 1000], 1), 0.0);

        // Full-scale mono input reaches the top of the normalized range.
        assert!((compute_audio_level(&[1.0; 1000], 1) - 1.0).abs() < f32::EPSILON);

        // Half-scale input gets a compressed but visible level.
        let half = compute_audio_level(&[0.5; 1000], 1);
        assert!(half > 0.4 && half < 1.0);

        // Typical speech around -30 dB sits near the middle of the meter.
        let speech = compute_audio_level(&[0.03; 1000], 1);
        assert!(speech > 0.45 && speech < 0.6);

        // Opposite-phase stereo channels cancel to 0; in-phase double.
        let stereo_in_phase = compute_audio_level(&[1.0, 1.0].repeat(500), 2);
        assert!((stereo_in_phase - 1.0).abs() < 0.01);
        let stereo_out_of_phase = compute_audio_level(&[1.0, -1.0].repeat(500), 2);
        assert_eq!(stereo_out_of_phase, 0.0);
    }

    #[test]
    fn silent_capture_is_not_reported_as_permission_denied() {
        assert!(matches!(
            validate_captured_audio(&[0.0, 0.0, 0.0]),
            Err(AudioError::SilentInput)
        ));
        assert!(validate_captured_audio(&[0.0, 0.01, 0.0]).is_ok());
    }

    #[test]
    fn streaming_resampler_matches_batch_output_across_chunk_boundaries() {
        let stereo: Vec<f32> = (0..9_600)
            .flat_map(|index| {
                let value = ((index as f32) * 0.013).sin();
                [value, value * 0.5]
            })
            .collect();
        let mono = mix_to_mono(&stereo, 2);
        let expected = resample_linear(&mono, 48_000, 16_000);
        let mut actual = Vec::new();
        let mut streaming = StreamingResampler::new(48_000, 16_000).unwrap();
        for chunk in stereo.chunks(960) {
            streaming
                .push_interleaved(chunk, 2, |sample| {
                    actual.push(sample);
                    Ok(())
                })
                .unwrap();
        }
        streaming
            .finish(|sample| {
                actual.push(sample);
                Ok(())
            })
            .unwrap();

        assert_eq!(actual.len(), expected.len());
        for (actual, expected) in actual.iter().zip(expected.iter()) {
            assert!((actual - expected).abs() < 1e-6);
        }
    }

    #[test]
    fn streaming_resampler_rejects_nonfinite_input_mix_and_output() {
        for samples in [vec![f32::NAN], vec![f32::INFINITY], vec![f32::NEG_INFINITY]] {
            let mut resampler = StreamingResampler::new(16_000, 16_000).unwrap();
            let error = resampler
                .push_interleaved(&samples, 1, |_| panic!("must not emit"))
                .unwrap_err();
            assert!(error.to_string().contains("non-finite"));
        }
        let mut resampler = StreamingResampler::new(16_000, 16_000).unwrap();
        let error = resampler
            .push_interleaved(&[f32::MAX, f32::MAX], 2, |_| panic!("must not emit"))
            .unwrap_err();
        assert!(error.to_string().contains("resampler mix"));
        let mut resampler = StreamingResampler::new(16_000, 32_000).unwrap();
        let error = resampler
            .push_interleaved(&[f32::MAX, -f32::MAX], 1, |_| panic!("must not emit"))
            .unwrap_err();
        assert!(error.to_string().contains("resampler output"));
    }

    #[test]
    fn streaming_resampler_keeps_a_constant_size_buffer() {
        let mut streaming = StreamingResampler::new(48_000, 16_000).unwrap();
        let mut max_buffered = 0;
        let ten_seconds_of_stereo = vec![0.25f32; 48_000 * 2 * 10];
        for chunk in ten_seconds_of_stereo.chunks(960) {
            streaming.push_interleaved(chunk, 2, |_| Ok(())).unwrap();
            max_buffered = max_buffered.max(streaming.buffered_frames());
        }
        streaming.finish(|_| Ok(())).unwrap();

        assert!(
            max_buffered <= 4,
            "streaming buffer retained {max_buffered} source frames"
        );
    }

    #[tokio::test]
    #[ignore] // Requires a live audio input with microphone permission; run with --ignored.
    async fn cpal_backend_records_nonzero_samples_from_default_input() {
        let mut backend = CpalAudioBackend::new();
        let devices = backend.list_devices().await.unwrap();
        let default = devices
            .iter()
            .find(|d| d.is_default)
            .or_else(|| devices.first())
            .cloned()
            .expect("no audio input device available");

        let temp_dir = std::env::temp_dir().join("ultravox-cpal-nonzero-test");
        std::fs::create_dir_all(&temp_dir).expect("failed to create temp dir");
        let output_path = temp_dir.join(format!("{}.wav", Uuid::new_v4()));

        let mut config = AudioInputConfig::default();
        config.device_id = Some(default.id);

        let _ = backend
            .start_recording(config, output_path.clone())
            .await
            .expect("failed to start recording");
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        let recording = backend
            .stop_recording()
            .await
            .expect("failed to stop recording");

        let reader = hound::WavReader::open(&recording.output_path)
            .expect("recorded file is not a valid WAV");
        let samples: Vec<f32> = reader
            .into_samples::<f32>()
            .map(|s| s.expect("invalid sample"))
            .collect();
        assert!(!samples.is_empty(), "recorded WAV contains no samples");
        let max_abs = samples.iter().map(|s| s.abs()).fold(0.0f32, f32::max);
        assert!(
            max_abs > 1e-4,
            "recorded WAV is silent; max absolute sample was {max_abs}"
        );
        assert!(
            recording.duration_ms.unwrap_or(0) >= 1000,
            "recorded duration too short: {:?}",
            recording.duration_ms
        );
    }
}

#[cfg(test)]
mod startup_tests {
    use super::*;
    use std::time::Duration;

    fn mock_recording() -> AudioRecording {
        AudioRecording {
            id: "mock".into(),
            output_path: PathBuf::from("never-created.wav"),
            start_time_ms: 0,
            duration_ms: Some(1000),
        }
    }

    async fn wait_for_cleanup(backend: &CpalAudioBackend) {
        tokio::time::timeout(Duration::from_secs(1), async {
            while backend
                .pending
                .as_ref()
                .is_some_and(|worker| !worker.handle.is_finished())
            {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("mock recorder thread must be joined");
    }

    #[tokio::test]
    async fn startup_waits_for_readiness_and_then_owns_one_worker() {
        let mut backend = CpalAudioBackend::new();
        let (entered_tx, entered_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let starting = backend.start_worker(
            PathBuf::from("mock.wav"),
            Duration::from_secs(1),
            move |stop, level, ready| {
                entered_tx.send(()).unwrap();
                release_rx.recv().unwrap();
                level.store(0.5f32.to_bits(), Ordering::Relaxed);
                ready.send(()).unwrap();
                stop.recv().unwrap();
                Ok(mock_recording())
            },
        );
        tokio::pin!(starting);
        tokio::select! {
            result = &mut starting => panic!("start returned before readiness: {result:?}"),
            _ = entered_rx => {}
        }
        release_tx.send(()).unwrap();
        starting.await.unwrap();
        // The pinned future is consumed above; the next lexical scope releases
        // its borrow before inspecting the backend.
    }

    #[tokio::test]
    async fn startup_success_level_reset_and_duplicate_start() {
        let mut backend = CpalAudioBackend::new();
        backend
            .start_worker(
                PathBuf::from("mock.wav"),
                Duration::from_secs(1),
                |stop, level, ready| {
                    level.store(0.5f32.to_bits(), Ordering::Relaxed);
                    ready.send(()).unwrap();
                    stop.recv().unwrap();
                    Err(AudioError::Format(
                        "wav write: synthetic disk failure".into(),
                    ))
                },
            )
            .await
            .unwrap();
        assert!(backend.is_recording());
        assert_eq!(backend.current_input_level(), 0.5);
        let error = backend
            .start_worker(
                PathBuf::from("mock.wav"),
                Duration::from_secs(1),
                |_, _, _| panic!("second worker must not spawn"),
            )
            .await
            .unwrap_err();
        assert!(matches!(error, AudioError::AlreadyRecording));
        let error = backend.stop_recording().await.unwrap_err();
        assert_eq!(
            error.to_string(),
            "audio format error: wav write: synthetic disk failure"
        );
        assert!(!backend.is_recording());
        assert_eq!(backend.current_level.load(Ordering::Relaxed), 0);
    }

    #[tokio::test]
    async fn startup_errors_preserve_causes_without_phantom_recording() {
        let cases = [
            AudioError::Device("enumerate input devices: synthetic enumeration failure".into()),
            AudioError::Device("no default input device".into()),
            AudioError::Device("input device not found: Missing Mic".into()),
            AudioError::Device("read input device name: synthetic name failure".into()),
            AudioError::Device("query supported input configs: synthetic config failure".into()),
            AudioError::Device("query default input config: synthetic default failure".into()),
            AudioError::Stream("build input stream: synthetic build failure".into()),
            AudioError::Stream("play stream: synthetic play failure".into()),
            AudioError::Format("wav writer: synthetic writer failure".into()),
        ];
        for cause in cases {
            let expected = cause.to_string();
            let mut backend = CpalAudioBackend::new();
            let error = backend
                .start_worker(
                    PathBuf::from("mock.wav"),
                    Duration::from_secs(1),
                    move |_, _, _ready| Err(cause),
                )
                .await
                .unwrap_err();
            assert_eq!(error.to_string(), expected);
            assert!(!backend.is_recording());
            assert!(backend.pending.is_none());
            assert_eq!(backend.current_input_level(), 0.0);
        }
    }

    #[tokio::test]
    async fn startup_timeout_signals_stop_and_joins_worker() {
        let mut backend = CpalAudioBackend::new();
        let (cleaned_tx, cleaned_rx) = tokio::sync::oneshot::channel();
        let error = backend
            .start_worker(
                PathBuf::from("mock.wav"),
                Duration::from_millis(20),
                move |stop, level, _ready| {
                    level.store(0.8f32.to_bits(), Ordering::Relaxed);
                    stop.recv().unwrap();
                    cleaned_tx.send(()).unwrap();
                    Err(AudioError::Startup("startup cancelled".into()))
                },
            )
            .await
            .unwrap_err();
        assert!(error.to_string().contains("readiness timed out"));
        assert!(!backend.is_recording());
        assert_eq!(backend.current_input_level(), 0.0);
        tokio::time::timeout(Duration::from_secs(1), cleaned_rx)
            .await
            .unwrap()
            .unwrap();
        wait_for_cleanup(&backend).await;
        assert_eq!(backend.current_level.load(Ordering::Relaxed), 0);
        let error = backend
            .start_worker(
                PathBuf::from("mock.wav"),
                Duration::from_secs(1),
                |_, _, _| Err(AudioError::Device("retry reached worker".into())),
            )
            .await
            .unwrap_err();
        assert!(error.to_string().contains("retry reached worker"));
    }

    #[tokio::test]
    async fn dropping_start_future_cancels_and_joins_worker() {
        let mut backend = CpalAudioBackend::new();
        let (entered_tx, entered_rx) = tokio::sync::oneshot::channel();
        let (cleaned_tx, cleaned_rx) = tokio::sync::oneshot::channel();
        {
            let starting = backend.start_worker(
                PathBuf::from("mock.wav"),
                Duration::from_secs(1),
                move |stop, _, _ready| {
                    entered_tx.send(()).unwrap();
                    stop.recv().unwrap();
                    cleaned_tx.send(()).unwrap();
                    Err(AudioError::Startup("startup cancelled".into()))
                },
            );
            tokio::pin!(starting);
            tokio::select! {
                result = &mut starting => panic!("unexpected startup result: {result:?}"),
                _ = entered_rx => {}
            }
        }
        assert!(!backend.is_recording());
        tokio::time::timeout(Duration::from_secs(1), cleaned_rx)
            .await
            .unwrap()
            .unwrap();
        wait_for_cleanup(&backend).await;
        assert_eq!(backend.current_input_level(), 0.0);
    }

    #[tokio::test]
    async fn startup_panic_reports_actual_thread_cause() {
        let mut backend = CpalAudioBackend::new();
        let error = backend
            .start_worker(
                PathBuf::from("mock.wav"),
                Duration::from_secs(1),
                |_, _, _| panic!("synthetic thread failure"),
            )
            .await
            .unwrap_err();
        assert!(error
            .to_string()
            .contains("recorder thread panicked: synthetic thread failure"));
        assert!(!backend.is_recording());
        assert!(backend.pending.is_none());
    }

    #[tokio::test]
    async fn timed_out_worker_blocks_restart_until_cleanup_finishes() {
        let mut backend = CpalAudioBackend::new();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        backend
            .start_worker(
                PathBuf::from("mock.wav"),
                Duration::from_millis(20),
                move |stop, _, _ready| {
                    stop.recv().unwrap();
                    release_rx.recv().unwrap();
                    Err(AudioError::Startup("startup cancelled".into()))
                },
            )
            .await
            .unwrap_err();
        let error = backend
            .start_worker(
                PathBuf::from("mock.wav"),
                Duration::from_secs(1),
                |_, _, _| panic!("must not overlap the previous worker"),
            )
            .await
            .unwrap_err();
        assert!(error.to_string().contains("still shutting down"));
        assert!(!backend.is_recording());
        release_tx.send(()).unwrap();
        wait_for_cleanup(&backend).await;
    }

    #[tokio::test]
    async fn dropping_backend_signals_active_worker_shutdown() {
        let mut backend = CpalAudioBackend::new();
        let level = Arc::clone(&backend.current_level);
        let (cleaned_tx, cleaned_rx) = tokio::sync::oneshot::channel();
        backend
            .start_worker(
                PathBuf::from("mock.wav"),
                Duration::from_secs(1),
                move |stop, level, ready| {
                    level.store(0.5f32.to_bits(), Ordering::Relaxed);
                    ready.send(()).unwrap();
                    stop.recv().unwrap();
                    cleaned_tx.send(()).unwrap();
                    Ok(mock_recording())
                },
            )
            .await
            .unwrap();
        drop(backend);
        tokio::time::timeout(Duration::from_secs(1), cleaned_rx)
            .await
            .unwrap()
            .unwrap();
        tokio::time::timeout(Duration::from_secs(1), async {
            while level.load(Ordering::Relaxed) != 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
    }

    #[test]
    fn startup_cancel_check_treats_disconnect_as_cancellation() {
        let (sender, receiver) = std::sync::mpsc::channel();
        assert!(check_startup_cancelled(&receiver).is_ok());
        drop(sender);
        assert!(check_startup_cancelled(&receiver)
            .unwrap_err()
            .to_string()
            .contains("cancelled"));
    }

    #[test]
    fn stream_error_keeps_backend_cause_instead_of_silence_or_permission() {
        let (sender, receiver) = std::sync::mpsc::channel();
        sender.send(cpal::StreamError::DeviceNotAvailable).unwrap();
        let error = check_stream_error(&receiver).unwrap_err();
        assert!(matches!(error, AudioError::Stream(_)));
        assert!(error
            .to_string()
            .contains(&cpal::StreamError::DeviceNotAvailable.to_string()));
    }

    fn temp_wav_path() -> PathBuf {
        std::env::temp_dir().join(format!("uv-audio-test-{}.wav", Uuid::new_v4()))
    }

    // Synthetic: the stream fails after speech was delivered; the speech is
    // drained, finalized, and readable instead of being discarded.
    #[test]
    fn stream_error_after_start_keeps_and_finalizes_captured_audio() {
        let path = temp_wav_path();
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: 16_000,
            bits_per_sample: 32,
            sample_format: hound::SampleFormat::Float,
        };
        let mut writer = hound::WavWriter::create(&path, spec).unwrap();
        let mut resampler = StreamingResampler::new(16_000, 16_000).unwrap();
        let (_stop_tx, stop_rx) = std::sync::mpsc::channel::<()>();
        let (sample_tx, sample_rx) = std::sync::mpsc::sync_channel::<Vec<f32>>(8);
        let (error_tx, error_rx) = std::sync::mpsc::channel();
        sample_tx.send(vec![0.25; 800]).unwrap();
        sample_tx.send(vec![-0.25; 800]).unwrap();
        error_tx.send(cpal::StreamError::DeviceNotAvailable).unwrap();
        let summary = capture_to_wav(
            (),
            &stop_rx,
            &sample_rx,
            &error_rx,
            1,
            &mut resampler,
            &mut writer,
        )
        .unwrap();
        assert_eq!(summary.frames, 1600);
        assert!(summary.max_abs > 0.2);
        assert!(matches!(summary.stream_failure, Some(AudioError::Stream(_))));
        assert!(require_captured_audio(summary.frames, summary.max_abs, summary.stream_failure)
            .is_ok());
        writer.finalize().unwrap();
        let samples: Vec<f32> = hound::WavReader::open(&path)
            .unwrap()
            .samples::<f32>()
            .map(Result::unwrap)
            .collect();
        assert_eq!(samples.len(), 1600);
        assert_eq!(samples[0], 0.25);
        assert_eq!(samples[1599], -0.25);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn stream_error_with_nothing_captured_fails_with_the_stream_cause() {
        let path = temp_wav_path();
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: 16_000,
            bits_per_sample: 32,
            sample_format: hound::SampleFormat::Float,
        };
        let mut writer = hound::WavWriter::create(&path, spec).unwrap();
        let mut resampler = StreamingResampler::new(16_000, 16_000).unwrap();
        let (_stop_tx, stop_rx) = std::sync::mpsc::channel::<()>();
        let (_sample_tx, sample_rx) = std::sync::mpsc::sync_channel::<Vec<f32>>(8);
        let (error_tx, error_rx) = std::sync::mpsc::channel();
        error_tx.send(cpal::StreamError::DeviceNotAvailable).unwrap();
        let summary = capture_to_wav(
            (),
            &stop_rx,
            &sample_rx,
            &error_rx,
            1,
            &mut resampler,
            &mut writer,
        )
        .unwrap();
        assert_eq!(summary.frames, 0);
        let error =
            require_captured_audio(summary.frames, summary.max_abs, summary.stream_failure)
                .unwrap_err();
        assert!(matches!(error, AudioError::Stream(_)));
        assert!(matches!(
            require_captured_audio(0, 0.0, None),
            Err(AudioError::SilentInput)
        ));
        drop(writer);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn startup_output_guard_removes_file_unless_recording_started() {
        let removed = temp_wav_path();
        std::fs::write(&removed, b"RIFF").unwrap();
        drop(StartupOutputGuard::new(removed.clone()));
        assert!(!removed.exists());

        let kept = temp_wav_path();
        std::fs::write(&kept, b"RIFF").unwrap();
        let mut guard = StartupOutputGuard::new(kept.clone());
        guard.keep();
        drop(guard);
        assert!(kept.exists());
        std::fs::remove_file(kept).unwrap();
    }

    // Synthetic: a start that never becomes ready times out and is cancelled;
    // the worker must not leave its partially created WAV behind.
    #[tokio::test]
    async fn timed_out_start_removes_its_output_file() {
        let path = temp_wav_path();
        let worker_path = path.clone();
        let mut backend = CpalAudioBackend::new();
        let error = backend
            .start_worker(
                path.clone(),
                Duration::from_millis(20),
                move |stop, _, _ready| {
                    let _guard = StartupOutputGuard::new(worker_path.clone());
                    std::fs::write(&worker_path, b"RIFF").unwrap();
                    stop.recv().unwrap();
                    Err(AudioError::Startup("startup cancelled".into()))
                },
            )
            .await
            .unwrap_err();
        assert!(error.to_string().contains("readiness timed out"));
        wait_for_cleanup(&backend).await;
        assert!(!path.exists());
    }
}
