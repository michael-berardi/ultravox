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

const AUDIO_CHUNK_QUEUE_CAPACITY: usize = 32;

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
        for frame in samples.chunks_exact(channels) {
            let mono = frame.iter().copied().sum::<f32>() / channels as f32;
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
    current_level: Arc<AtomicU32>,
}

impl CpalAudioBackend {
    /// Create a new cpal-backed audio backend using the default host.
    pub fn new() -> Self {
        Self {
            recording: None,
            current_level: Arc::new(AtomicU32::new(0)),
        }
    }

    /// Return the current microphone input level as a normalized value in [0, 1].
    ///
    /// The value is lock-free and updated from the live CPAL input callback. It
    /// is reset to 0 at recording lifecycle boundaries and reports 0 when the
    /// backend is not recording.
    pub fn current_input_level(&self) -> f32 {
        if self.recording.is_none() {
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
    ) -> Result<AudioRecording, AudioError> {
        let sample_rate = config.sample_rate;
        let channels = config.channels;

        let host = cpal::default_host();
        let device = if let Some(device_id) = &config.device_id {
            let name_target = device_id.strip_prefix("cpal:").unwrap_or(device_id);
            host.input_devices()
                .map_err(|_| AudioError::DeviceUnavailable)?
                .find(|d| d.name().ok().as_deref() == Some(name_target))
                .ok_or(AudioError::DeviceUnavailable)?
        } else {
            host.default_input_device()
                .ok_or(AudioError::DeviceUnavailable)?
        };

        let supported_configs = device
            .supported_input_configs()
            .map_err(|_| AudioError::DeviceUnavailable)?;
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
        let selected_config = selected_config
            .or_else(|| device.default_input_config().ok())
            .ok_or(AudioError::DeviceUnavailable)?;

        let actual_sample_rate = selected_config.sample_rate().0;
        let actual_channels = selected_config.channels();

        let (sample_tx, sample_rx) =
            std::sync::mpsc::sync_channel::<Vec<f32>>(AUDIO_CHUNK_QUEUE_CAPACITY);
        let overflowed_chunks = Arc::new(AtomicU32::new(0));
        let err_fn = |err| eprintln!("audio stream error: {}", err);
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
        .map_err(|e| AudioError::Format(format!("build stream: {e}")))?;

        stream
            .play()
            .map_err(|e| AudioError::Format(format!("play stream: {e}")))?;

        // Drop our copy of sample_tx so the only remaining sender is the stream
        // callback; the receiver can then detect end-of-stream by disconnect.
        drop(sample_tx);

        let spec = hound::WavSpec {
            channels: 1,
            sample_rate,
            bits_per_sample: 32,
            sample_format: hound::SampleFormat::Float,
        };
        let mut writer = hound::WavWriter::create(&output_path, spec)
            .map_err(|e| AudioError::Format(format!("wav writer: {e}")))?;
        let mut resampler = StreamingResampler::new(actual_sample_rate, sample_rate)?;
        let mut frames = 0u64;
        let mut max_abs = 0.0f32;
        let mut process_chunk = |chunk: Vec<f32>| -> Result<(), AudioError> {
            resampler.push_interleaved(&chunk, actual_channels, |sample| {
                let sample = sample.clamp(-1.0, 1.0);
                max_abs = max_abs.max(sample.abs());
                writer
                    .write_sample(sample)
                    .map_err(|_| AudioError::Format("wav write failed".to_string()))?;
                frames += 1;
                Ok(())
            })
        };

        loop {
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
        while let Ok(chunk) = sample_rx.try_recv() {
            process_chunk(chunk)?;
        }
        drop(process_chunk);
        resampler.finish(|sample| {
            let sample = sample.clamp(-1.0, 1.0);
            max_abs = max_abs.max(sample.abs());
            writer
                .write_sample(sample)
                .map_err(|_| AudioError::Format("wav write failed".to_string()))?;
            frames += 1;
            Ok(())
        })?;

        // Silence can come from a muted or unavailable input as well as an OS
        // privacy decision. Permission is checked explicitly before recording,
        // so never infer it from sample content.
        if frames == 0 || max_abs < 1e-8 {
            return Err(AudioError::SilentInput);
        }
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

#[allow(dead_code)]
#[derive(Debug)]
struct CpalRecordingState {
    id: String,
    output_path: PathBuf,
    start_time_ms: u64,
    stop_tx: std::sync::mpsc::Sender<()>,
    handle: tokio::task::JoinHandle<Result<AudioRecording, AudioError>>,
}

#[async_trait]
impl AudioBackend for CpalAudioBackend {
    async fn list_devices(&self) -> Result<Vec<AudioDeviceInfo>, AudioError> {
        let host = cpal::default_host();
        let mut devices = Vec::new();
        let default = host.default_input_device();
        let Ok(list) = host.input_devices() else {
            return Ok(vec![AudioDeviceInfo {
                id: "default".to_string(),
                name: "System Default".to_string(),
                is_default: true,
            }]);
        };
        for device in list {
            let name = device.name().unwrap_or_else(|_| "Unknown".to_string());
            let id = format!("cpal:{}", name);
            let is_default = default.as_ref().and_then(|d| d.name().ok()).as_ref() == Some(&name);
            devices.push(AudioDeviceInfo {
                id,
                name,
                is_default,
            });
        }
        if devices.is_empty() {
            devices.push(AudioDeviceInfo {
                id: "default".to_string(),
                name: "System Default".to_string(),
                is_default: true,
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
        self.store_level(0.0);
        let level_shared = Arc::clone(&self.current_level);
        let (stop_tx, stop_rx) = std::sync::mpsc::channel::<()>();
        let output_path_clone = output_path.clone();
        let id_for_thread = id.clone();
        let (result_tx, result_rx) =
            tokio::sync::oneshot::channel::<Result<AudioRecording, AudioError>>();

        // The cpal Stream is not Send on macOS CoreAudio, so device setup, stream
        // creation, sample collection, and WAV writing all happen on a dedicated
        // OS thread. Communication back to the async caller uses tokio oneshot.
        std::thread::spawn(move || {
            let result = Self::record_on_thread(
                config,
                output_path_clone,
                id_for_thread,
                start_time_ms,
                stop_rx,
                level_shared,
            );
            let _ = result_tx.send(result);
        });

        let handle: tokio::task::JoinHandle<Result<AudioRecording, AudioError>> =
            tokio::spawn(async move {
                result_rx.await.unwrap_or_else(|_| {
                    Err(AudioError::Format(
                        "recorder thread dropped result".to_string(),
                    ))
                })
            });

        self.recording = Some(CpalRecordingState {
            id: id.clone(),
            output_path: output_path.clone(),
            start_time_ms,
            stop_tx,
            handle,
        });

        Ok(AudioRecording {
            id,
            output_path,
            start_time_ms,
            duration_ms: None,
        })
    }

    async fn stop_recording(&mut self) -> Result<AudioRecording, AudioError> {
        let state = self.recording.take().ok_or(AudioError::NotRecording)?;
        let _ = state.stop_tx.send(());
        let result = state
            .handle
            .await
            .map_err(|e| AudioError::Format(format!("recorder task: {e}")))??;
        self.store_level(0.0);

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
        self.recording.is_some()
    }
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
