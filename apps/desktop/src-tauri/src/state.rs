use chrono::Utc;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_clipboard_manager::ClipboardExt;
use tokio::sync::Mutex as AsyncMutex;
use tokio::task::{AbortHandle, JoinHandle};
use uuid::Uuid;

use ultravox_core::{
    AppConfig, AudioBackend, AudioInputConfig, AudioRecording, ConfigManager, CpalAudioBackend,
    CustomDictionary, DownloadManager, ModelCatalog, RecordingHistory, RecordingRow,
    RecordingStatus,
};

#[cfg(target_os = "macos")]
use ultravox_macos_bridge as bridge;
#[cfg(not(target_os = "macos"))]
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

use crate::events::{
    IndicatorHidePayload, IndicatorShowPayload, MeetingDetection, MeetingDetectionPendingPayload,
    RecordingAddedPayload, RecordingDeletedPayload, RecordingStartedPayload,
    RecordingStoppedPayload, SettingsChangedPayload, TranscriptionCompletedPayload,
    TranscriptionProgressPayload, UrlImportProgressPayload, INDICATOR_HIDE, INDICATOR_SHOW,
    MEETING_DETECTION_TTL_MS, MEETING_STATE_CHANGED, RECORDING_ADDED, RECORDING_DELETED,
    RECORDING_STARTED, RECORDING_STOPPED, SETTINGS_CHANGED, SHORTCUT_TRIGGERED,
    TRANSCRIPTION_COMPLETED, TRANSCRIPTION_PROGRESS, URL_IMPORT_PROGRESS,
};

/// A live recording session tracked by the desktop shell.
#[derive(Debug, Clone)]
pub struct RecordingSession {
    pub id: Uuid,
    pub recording: AudioRecording,
    pub started_at: Instant,
    pub keep_awake: KeepAwake,
}

#[derive(Debug, Clone)]
pub struct MeetingSession {
    pub id: Uuid,
    pub output_path: PathBuf,
    pub include_microphone: bool,
    pub started_at: Instant,
    pub stopping: bool,
    pub keep_awake: KeepAwake,
}

/// Keeps macOS from App Napping UltraVox while dictation records, a meeting
/// captures, or a transcription runs. With every window hidden, App Nap starved
/// the audio writer until the capture buffer overflowed and the recording was
/// rejected. A no-op on other platforms.
#[derive(Debug, Clone)]
pub struct KeepAwake(
    // Held only for its Drop, which ends the activity.
    #[cfg(target_os = "macos")]
    #[allow(dead_code)]
    std::sync::Arc<bridge::LatencyActivity>,
);

impl KeepAwake {
    pub fn begin(reason: &str) -> Self {
        #[cfg(target_os = "macos")]
        return Self(std::sync::Arc::new(bridge::LatencyActivity::begin(reason)));
        #[cfg(not(target_os = "macos"))]
        {
            let _ = reason;
            Self()
        }
    }
}

/// Handle for an in-flight transcription task so cancellation can target a
/// specific recording without disturbing other work.
#[derive(Debug)]
pub struct ActiveTranscription {
    pub recording_id: Uuid,
    pub abort_handle: AbortHandle,
}

pub(crate) const MEETING_DETECTION_TTL: Duration = Duration::from_millis(MEETING_DETECTION_TTL_MS);
const MAX_MEETING_DETECTION_ENTRIES: usize = 256;

#[derive(Debug, Clone)]
struct PendingMeetingDetection {
    detection: MeetingDetection,
    expires_at: Instant,
}
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(not(feature = "ultravox-pro"), allow(dead_code))]
enum MeetingDecision {
    InFlight,
    Accepted(String),
    Declined,
}

#[derive(Debug, Default)]
struct MeetingDetectionState {
    seen: HashMap<(crate::events::MeetingProvider, String), Instant>,
    decisions: HashMap<String, (Instant, MeetingDecision)>,
    pending: Option<PendingMeetingDetection>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MeetingDetectionRegistration {
    Prompt,
    Disabled,
    Active,
    Duplicate,
    Pending,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(not(feature = "ultravox-pro"), allow(dead_code))]
pub(crate) enum MeetingDeclineResult {
    Declined,
    AlreadyDeclined,
    AlreadyAccepted,
    InFlight,
    NotFound,
}

#[derive(Debug, Clone)]
#[cfg_attr(not(feature = "ultravox-pro"), allow(dead_code))]
pub(crate) enum MeetingAcceptClaim {
    Claimed(MeetingDetection, Instant),
    AlreadyAccepted(String),
    AlreadyDeclined,
    InFlight,
    Failed(String),
}

#[cfg(target_os = "macos")]
struct NativeIndicatorGuard;

#[cfg(target_os = "macos")]
impl Drop for NativeIndicatorGuard {
    fn drop(&mut self) {
        bridge::clear_insertion_target();
        bridge::hide_indicator();
    }
}

/// Shared application state managed by Tauri and accessible from commands.
///
/// Wraps the core configuration, download, history, and model managers with
/// coarse-grained locking plus a live recording session and event helpers.
/// This is sufficient for the Phase 1 shell; future milestones will refine
/// the concurrency model.
#[derive(Debug)]
pub struct AppState {
    pub app: AppHandle,
    pub config: Mutex<ConfigManager>,
    pub downloads: Mutex<DownloadManager>,
    pub history: Mutex<RecordingHistory>,
    pub catalog: ModelCatalog,
    pub activity_transition: AsyncMutex<()>,
    pub session: AsyncMutex<Option<RecordingSession>>,
    pub active_transcription: AsyncMutex<Option<ActiveTranscription>>,
    /// Serializes dropped-file imports so each file waits for the previous
    /// transcription to start (and finish) in drop order.
    pub file_import: AsyncMutex<()>,
    pub meeting_session: AsyncMutex<Option<MeetingSession>>,
    meeting_detection: AsyncMutex<MeetingDetectionState>,
    pub audio: AsyncMutex<CpalAudioBackend>,
    /// Cancellation handle for the currently running Retex refresh, if any.
    pub retex_scan_cancel: Mutex<Option<std::sync::Arc<std::sync::atomic::AtomicBool>>>,
    pub telemetry: crate::telemetry::Telemetry,
}

fn recording_id_for(recording: &AudioRecording) -> Uuid {
    Uuid::parse_str(&recording.id)
        .ok()
        .or_else(|| {
            recording
                .output_path
                .file_stem()
                .and_then(|s| s.to_str())
                .and_then(|s| Uuid::parse_str(s).ok())
        })
        .unwrap_or_else(Uuid::new_v4)
}

fn cancellation_may_discard_audio(error: &ultravox_core::audio::AudioError) -> bool {
    // Explicit cancellation does not require a valid recording or transcript.
    // CPAL has already dropped the stream before reporting missing samples.
    matches!(error, ultravox_core::audio::AudioError::SilentInput)
}

fn require_transcription_text(
    text: String,
    audio_path: &std::path::Path,
) -> Result<String, String> {
    if text.trim().is_empty() {
        return Err(format!(
            "transcription returned empty text: {}",
            ultravox_core::phonon::explain_empty(audio_path)
        ));
    }
    Ok(text)
}

fn fallback_row(
    recording: &AudioRecording,
    id: Uuid,
    language: String,
    transcription: String,
    status: RecordingStatus,
    progress: f32,
) -> RecordingRow {
    let file_name = recording
        .output_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("recording.wav")
        .to_string();
    let duration_seconds = recording.duration_ms.unwrap_or(0) as f64 / 1000.0;
    let mut row = RecordingRow {
        id,
        timestamp: Utc::now(),
        file_name,
        title: String::new(),
        preview: String::new(),
        transcription,
        language,
        duration_seconds,
        status,
        progress,
        source_file_url: Some(recording.output_path.to_string_lossy().to_string()),
    };
    row.refresh_display();
    row
}

/// FluidAudio's cache folders for the English Parakeet v2 model that Phonon-2 replaces.
pub(crate) fn superseded_fluid_audio_dirs() -> Vec<PathBuf> {
    #[cfg(target_os = "macos")]
    if let Some(home) = std::env::var_os("HOME") {
        return vec![PathBuf::from(home)
            .join("Library/Application Support/FluidAudio/Models/parakeet-tdt-0.6b-v2")];
    }
    Vec::new()
}

#[cfg(not(target_os = "macos"))]
struct WhisperOptions {
    language: String,
    translate: bool,
    suppress_blank: bool,
    show_timestamps: bool,
    temperature: f32,
    no_speech_threshold: f32,
    initial_prompt: String,
    use_beam_search: bool,
    beam_size: i32,
}

#[cfg(not(target_os = "macos"))]
fn transcribe_with_whisper(
    audio_path: &std::path::Path,
    model_path: &std::path::Path,
    options: WhisperOptions,
) -> Result<String, String> {
    let input =
        ultravox_core::prepare_transcription_audio(audio_path).map_err(|e| e.to_string())?;
    let audio_path = input.path();
    if !model_path.is_file() {
        return Err(format!(
            "Whisper model is not downloaded: {}",
            model_path.display()
        ));
    }
    let mut reader = hound::WavReader::open(audio_path)
        .map_err(|error| format!("failed to read recorded audio: {error}"))?;
    let spec = reader.spec();
    if spec.channels != 1 || spec.sample_rate != 16_000 {
        return Err(format!(
            "Whisper requires 16 kHz mono WAV input; received {} Hz / {} channels",
            spec.sample_rate, spec.channels
        ));
    }
    let samples = match (spec.sample_format, spec.bits_per_sample) {
        (hound::SampleFormat::Int, 16) => reader
            .samples::<i16>()
            .map(|sample| {
                sample
                    .map(|value| value as f32 / i16::MAX as f32)
                    .map_err(|error| error.to_string())
            })
            .collect::<Result<Vec<_>, _>>()?,
        (hound::SampleFormat::Float, 32) => reader
            .samples::<f32>()
            .map(|sample| sample.map_err(|error| error.to_string()))
            .collect::<Result<Vec<_>, _>>()?,
        _ => {
            return Err(format!(
                "Whisper requires 16-bit PCM or 32-bit float WAV input; received {}-bit {:?}",
                spec.bits_per_sample, spec.sample_format
            ))
        }
    };
    let context = WhisperContext::new_with_params(model_path, WhisperContextParameters::default())
        .map_err(|error| format!("failed to load Whisper model: {error}"))?;
    let mut state = context
        .create_state()
        .map_err(|error| format!("failed to initialize Whisper: {error}"))?;
    let strategy = if options.use_beam_search {
        SamplingStrategy::BeamSearch {
            beam_size: options.beam_size.max(1),
            patience: -1.0,
        }
    } else {
        SamplingStrategy::Greedy { best_of: 1 }
    };
    let mut params = FullParams::new(strategy);
    params.set_n_threads(
        std::thread::available_parallelism()
            .map(|threads| threads.get().min(8) as i32)
            .unwrap_or(4),
    );
    params.set_translate(options.translate);
    params.set_suppress_blank(options.suppress_blank);
    params.set_no_timestamps(!options.show_timestamps);
    params.set_temperature(options.temperature);
    params.set_no_speech_thold(options.no_speech_threshold);
    params.set_print_progress(false);
    params.set_print_realtime(false);
    params.set_print_timestamps(false);
    let detect_language = options.language.is_empty() || options.language == "auto";
    params.set_language((!detect_language).then_some(options.language.as_str()));
    params.set_detect_language(detect_language);
    if !options.initial_prompt.is_empty() {
        params.set_initial_prompt(&options.initial_prompt);
    }
    state
        .full(params, &samples)
        .map_err(|error| format!("Whisper transcription failed: {error}"))?;
    let mut text = String::new();
    for segment in state.as_iter() {
        text.push_str(
            &segment
                .to_str_lossy()
                .map_err(|error| format!("failed to read Whisper output: {error}"))?,
        );
    }
    if text.trim().is_empty() {
        return Err(format!(
            "transcription returned empty text: {}",
            ultravox_core::phonon::explain_empty(audio_path)
        ));
    }
    Ok(text.trim().to_string())
}

#[cfg(all(test, not(target_os = "macos")))]
mod whisper_integration_test {
    #[test]
    #[ignore = "requires ULTRAVOX_TEST_WHISPER_MODEL"]
    fn transcribes_the_real_jfk_fixture() {
        let model = std::env::var("ULTRAVOX_TEST_WHISPER_MODEL")
            .expect("ULTRAVOX_TEST_WHISPER_MODEL must point to ggml-base.en.bin");
        let audio = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../test/fixtures/jfk-short.wav");
        let text = super::transcribe_with_whisper(
            &audio,
            std::path::Path::new(&model),
            super::WhisperOptions {
                language: "en".to_string(),
                translate: false,
                suppress_blank: true,
                show_timestamps: false,
                temperature: 0.0,
                no_speech_threshold: 0.6,
                initial_prompt: String::new(),
                use_beam_search: false,
                beam_size: 5,
            },
        )
        .unwrap()
        .to_ascii_lowercase();
        assert!(
            text.contains("fellow") && text.contains("americans"),
            "{text}"
        );
    }
}

/// All desktop-owned stores share this root. Mirror builds require an explicit
/// existing sandbox and reject production roots (including symlink aliases).
pub(crate) fn data_dir(app: &AppHandle) -> Result<PathBuf, String> {
    let production = app.path().app_data_dir().map_err(|e| e.to_string())?;
    let override_dir = std::env::var_os("ULTRAVOX_DATA_DIR").map(PathBuf::from);
    if cfg!(feature = "mirror-debug") {
        let path = override_dir
            .ok_or("mirror-debug requires ULTRAVOX_DATA_DIR pointing to an existing sandbox")?;
        let path = path.canonicalize().map_err(|e| e.to_string())?;
        let production = production.canonicalize().map_err(|e| e.to_string())?;
        if path.starts_with(&production) || production.starts_with(&path) {
            return Err("mirror data directory overlaps production".into());
        }
        return Ok(path);
    }
    Ok(override_dir.unwrap_or(production))
}

impl AppState {
    /// Why replacing the app right now would lose the user's work, if it would.
    /// Updates wait for `None`: a relaunch mid-recording drops the audio.
    pub async fn busy_reason(&self) -> Option<&'static str> {
        if self.session.lock().await.is_some() {
            return Some("UltraVox is recording");
        }
        if self.meeting_session.lock().await.is_some() {
            return Some("a meeting or lecture is being recorded");
        }
        if self.active_transcription.lock().await.is_some() {
            return Some("a recording is being transcribed");
        }
        if self.file_import.try_lock().is_err() {
            return Some("a file is being imported");
        }
        None
    }

    /// Initialize the state from the Tauri app handle.
    pub fn new(app: AppHandle) -> Result<Self, String> {
        let app_dir = data_dir(&app)?;
        let config = ConfigManager::new(&app_dir).map_err(|e| e.to_string())?;
        let history = RecordingHistory::new(app_dir.clone()).map_err(|e| e.to_string())?;
        let models_dir = config
            .get()
            .models_directory
            .clone()
            .unwrap_or_else(|| app_dir.join("models"));
        let _ = std::fs::remove_dir_all(app_dir.join("recordings").join(".imports"));
        let telemetry = crate::telemetry::Telemetry::new(app.clone())?;
        Ok(Self {
            app: app.clone(),
            config: Mutex::new(config),
            downloads: Mutex::new(DownloadManager::with_models_dir(models_dir)),
            history: Mutex::new(history),
            catalog: ModelCatalog::default(),
            activity_transition: AsyncMutex::new(()),
            session: AsyncMutex::new(None),
            active_transcription: AsyncMutex::new(None),
            file_import: AsyncMutex::new(()),
            meeting_session: AsyncMutex::new(None),
            meeting_detection: AsyncMutex::new(MeetingDetectionState::default()),
            audio: AsyncMutex::new(CpalAudioBackend::new()),
            retex_scan_cancel: Mutex::new(None),
            telemetry,
        })
    }
    pub fn record_telemetry_usage(&self, counters: crate::telemetry::UsageCounters) {
        let app = self.app.clone();
        tauri::async_runtime::spawn(async move {
            let state = app.state::<AppState>();
            let _ = state.telemetry.usage(counters).await;
        });
    }

    /// Path to the configured directory for cached model files.
    pub fn models_dir(&self) -> Result<PathBuf, String> {
        if let Some(path) = self
            .config
            .lock()
            .map_err(|e| e.to_string())?
            .get()
            .models_directory
            .clone()
        {
            return Ok(path);
        }
        let app_dir = data_dir(&self.app)?;
        Ok(app_dir.join("models"))
    }

    /// Path to the default directory for recorded audio files.
    pub fn recordings_dir(&self) -> Result<PathBuf, String> {
        let app_dir = data_dir(&self.app)?;
        Ok(app_dir.join("recordings"))
    }

    /// Emit `recording-started`.
    pub fn emit_recording_started(&self, recording: &AudioRecording) -> Result<(), String> {
        self.app
            .emit(RECORDING_STARTED, RecordingStartedPayload::from(recording))
            .map_err(|e| e.to_string())
    }

    /// Emit `recording-stopped`.
    pub fn emit_recording_stopped(&self, recording: &AudioRecording) -> Result<(), String> {
        self.app
            .emit(RECORDING_STOPPED, RecordingStoppedPayload::from(recording))
            .map_err(|e| e.to_string())
    }

    /// Emit `transcription-progress`.
    pub fn emit_transcription_progress(
        &self,
        recording_id: impl Into<String>,
        progress: f32,
        status: impl Into<String>,
    ) -> Result<(), String> {
        self.app
            .emit(
                TRANSCRIPTION_PROGRESS,
                TranscriptionProgressPayload::new(recording_id, progress, status),
            )
            .map_err(|e| e.to_string())
    }

    /// Emit `transcription-completed`.
    pub fn emit_transcription_completed(
        &self,
        recording_id: impl Into<String>,
        text: impl Into<String>,
        language: Option<impl Into<String>>,
    ) -> Result<(), String> {
        self.app
            .emit(
                TRANSCRIPTION_COMPLETED,
                TranscriptionCompletedPayload::new(recording_id, text, language),
            )
            .map_err(|e| e.to_string())
    }

    /// Emit `shortcut-triggered`.
    pub fn emit_shortcut_triggered(&self, shortcut: impl Into<String>) -> Result<(), String> {
        self.app
            .emit(
                SHORTCUT_TRIGGERED,
                crate::events::ShortcutTriggeredPayload::new(shortcut),
            )
            .map_err(|e| e.to_string())
    }

    /// Emit `indicator-show`.
    pub fn emit_indicator_show(&self, x: f64, y: f64) -> Result<(), String> {
        self.app
            .emit(INDICATOR_SHOW, IndicatorShowPayload { x, y })
            .map_err(|e| e.to_string())
    }

    /// Emit `indicator-hide`.
    pub fn emit_indicator_hide(&self) -> Result<(), String> {
        self.app
            .emit(INDICATOR_HIDE, IndicatorHidePayload)
            .map_err(|e| e.to_string())
    }

    /// Emit `settings-changed`.
    pub fn emit_settings_changed(&self, config: &AppConfig) -> Result<(), String> {
        self.app
            .emit(SETTINGS_CHANGED, SettingsChangedPayload::from(config))
            .map_err(|e| e.to_string())
    }

    /// Emit `recording-added`.
    pub fn emit_recording_added(&self, row: &ultravox_core::RecordingRow) -> Result<(), String> {
        self.app
            .emit(RECORDING_ADDED, RecordingAddedPayload::from(row))
            .map_err(|e| e.to_string())
    }

    /// Emit `recording-deleted`.
    pub fn emit_recording_deleted(&self, id: Uuid) -> Result<(), String> {
        self.app
            .emit(RECORDING_DELETED, RecordingDeletedPayload::from(id))
            .map_err(|e| e.to_string())
    }

    pub fn emit_meeting_state_changed(&self, active: bool) -> Result<(), String> {
        self.app
            .emit(MEETING_STATE_CHANGED, active)
            .map_err(|e| e.to_string())
    }

    fn prune_meeting_detection_state(state: &mut MeetingDetectionState, now: Instant) -> bool {
        state
            .seen
            .retain(|_, seen_at| now.duration_since(*seen_at) <= MEETING_DETECTION_TTL);
        state
            .decisions
            .retain(|_, (decided_at, _)| now.duration_since(*decided_at) <= MEETING_DETECTION_TTL);
        while state.decisions.len() > MAX_MEETING_DETECTION_ENTRIES {
            let Some((oldest_id, _)) = state
                .decisions
                .iter()
                .min_by_key(|(_, (decided_at, _))| *decided_at)
            else {
                break;
            };
            let oldest_id = oldest_id.clone();
            state.decisions.remove(&oldest_id);
        }
        let expired = state
            .pending
            .as_ref()
            .is_some_and(|pending| pending.expires_at <= now);
        if expired {
            if let Some(pending) = state.pending.take() {
                state.decisions.insert(
                    pending.detection.detection_id,
                    (now, MeetingDecision::Declined),
                );
            }
        }
        expired
    }

    #[cfg_attr(not(feature = "ultravox-pro"), allow(dead_code))]
    fn release_meeting_accept_state(
        state: &mut MeetingDetectionState,
        detection: MeetingDetection,
        expires_at: Instant,
        restore_allowed: bool,
        now: Instant,
    ) -> bool {
        let detection_id = detection.detection_id.clone();
        let restored = restore_allowed && expires_at > now && state.pending.is_none();
        if restored {
            state.pending = Some(PendingMeetingDetection {
                detection,
                expires_at,
            });
            state.decisions.remove(&detection_id);
        } else {
            state
                .decisions
                .insert(detection_id, (now, MeetingDecision::Declined));
        }
        restored
    }
    pub(crate) async fn register_meeting_detection(
        &self,
        detection: MeetingDetection,
    ) -> Result<
        (
            MeetingDetectionRegistration,
            Option<MeetingDetectionPendingPayload>,
        ),
        String,
    > {
        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| error.to_string())?
            .as_millis() as u64;
        detection.validate(now_ms)?;

        let enabled = self
            .config
            .lock()
            .map_err(|error| error.to_string())?
            .get()
            .meeting_detection_enabled;
        if !enabled {
            return Ok((MeetingDetectionRegistration::Disabled, None));
        }
        if self.meeting_session.lock().await.is_some() || self.session.lock().await.is_some() {
            return Ok((MeetingDetectionRegistration::Active, None));
        }

        let now = Instant::now();
        let mut state = self.meeting_detection.lock().await;
        if Self::prune_meeting_detection_state(&mut state, now) {
            crate::close_meeting_reminder(&self.app);
        }
        if state
            .pending
            .as_ref()
            .is_some_and(|pending| pending.detection.detection_id == detection.detection_id)
            || state.decisions.contains_key(&detection.detection_id)
        {
            return Ok((MeetingDetectionRegistration::Duplicate, None));
        }
        let key = (detection.provider, detection.meeting_key.clone());
        if state.seen.contains_key(&key) {
            return Ok((MeetingDetectionRegistration::Duplicate, None));
        }
        if state.seen.len() >= MAX_MEETING_DETECTION_ENTRIES {
            if let Some((oldest_key, _)) = state.seen.iter().min_by_key(|(_, seen_at)| *seen_at) {
                let oldest_key = oldest_key.clone();
                state.seen.remove(&oldest_key);
            }
        }
        state.seen.insert(key, now);
        if state.pending.is_some() {
            return Ok((MeetingDetectionRegistration::Pending, None));
        }

        let expires_at = now
            .checked_add(MEETING_DETECTION_TTL)
            .ok_or_else(|| "meeting detection expiry overflow".to_string())?;
        let expires_at_ms = detection
            .detected_at_ms
            .checked_add(MEETING_DETECTION_TTL_MS)
            .ok_or_else(|| "meeting detection expiry overflow".to_string())?;
        let payload = MeetingDetectionPendingPayload::from_detection(&detection, expires_at_ms);
        state.pending = Some(PendingMeetingDetection {
            detection,
            expires_at,
        });
        Ok((MeetingDetectionRegistration::Prompt, Some(payload)))
    }

    #[cfg_attr(not(feature = "ultravox-pro"), allow(dead_code))]
    pub(crate) async fn claim_meeting_accept(&self, detection_id: &str) -> MeetingAcceptClaim {
        let now = Instant::now();
        let mut state = self.meeting_detection.lock().await;
        if Self::prune_meeting_detection_state(&mut state, now) {
            crate::close_meeting_reminder(&self.app);
        }
        if let Some(pending) = state.pending.take() {
            if pending.detection.detection_id == detection_id {
                state
                    .decisions
                    .insert(detection_id.to_string(), (now, MeetingDecision::InFlight));
                return MeetingAcceptClaim::Claimed(pending.detection, pending.expires_at);
            }
            state.pending = Some(pending);
            return MeetingAcceptClaim::Failed("meeting detection is not pending".to_string());
        }
        match state
            .decisions
            .get(detection_id)
            .map(|(_, decision)| decision)
        {
            Some(MeetingDecision::Accepted(recording_id)) => {
                MeetingAcceptClaim::AlreadyAccepted(recording_id.clone())
            }
            Some(MeetingDecision::Declined) => MeetingAcceptClaim::AlreadyDeclined,
            Some(MeetingDecision::InFlight) => MeetingAcceptClaim::InFlight,
            None => MeetingAcceptClaim::Failed("meeting detection is not pending".to_string()),
        }
    }

    #[cfg_attr(not(feature = "ultravox-pro"), allow(dead_code))]
    pub(crate) async fn complete_meeting_accept(&self, detection_id: &str, recording_id: String) {
        self.meeting_detection.lock().await.decisions.insert(
            detection_id.to_string(),
            (Instant::now(), MeetingDecision::Accepted(recording_id)),
        );
    }

    #[cfg_attr(not(feature = "ultravox-pro"), allow(dead_code))]
    pub(crate) async fn release_meeting_accept(
        &self,
        detection: MeetingDetection,
        expires_at: Instant,
        restore_allowed: bool,
        _error: String,
    ) -> bool {
        let now = Instant::now();
        let mut state = self.meeting_detection.lock().await;
        Self::release_meeting_accept_state(&mut state, detection, expires_at, restore_allowed, now)
    }

    pub(crate) async fn rollback_meeting_detection(&self, detection: &MeetingDetection) {
        let mut state = self.meeting_detection.lock().await;
        state.pending = state
            .pending
            .take()
            .filter(|pending| pending.detection.detection_id != detection.detection_id);
        state.decisions.remove(&detection.detection_id);
        state
            .seen
            .remove(&(detection.provider, detection.meeting_key.clone()));
    }

    #[cfg_attr(not(feature = "ultravox-pro"), allow(dead_code))]
    pub(crate) async fn pending_meeting_detection(&self) -> Option<MeetingDetectionPendingPayload> {
        let now = Instant::now();
        let mut state = self.meeting_detection.lock().await;
        if Self::prune_meeting_detection_state(&mut state, now) {
            crate::close_meeting_reminder(&self.app);
        }
        state.pending.as_ref().map(|pending| {
            MeetingDetectionPendingPayload::from_detection(
                &pending.detection,
                pending
                    .detection
                    .detected_at_ms
                    .saturating_add(MEETING_DETECTION_TTL_MS),
            )
        })
    }

    #[cfg_attr(not(feature = "ultravox-pro"), allow(dead_code))]
    pub(crate) async fn decline_meeting_detection(
        &self,
        detection_id: &str,
    ) -> MeetingDeclineResult {
        let now = Instant::now();
        let mut state = self.meeting_detection.lock().await;
        if Self::prune_meeting_detection_state(&mut state, now) {
            crate::close_meeting_reminder(&self.app);
        }
        if let Some(pending) = state.pending.as_ref() {
            if pending.detection.detection_id == detection_id {
                state.pending = None;
                state
                    .decisions
                    .insert(detection_id.to_string(), (now, MeetingDecision::Declined));
                return MeetingDeclineResult::Declined;
            }
            return MeetingDeclineResult::NotFound;
        }
        match state
            .decisions
            .get(detection_id)
            .map(|(_, decision)| decision)
        {
            Some(MeetingDecision::Declined) => MeetingDeclineResult::AlreadyDeclined,
            Some(MeetingDecision::Accepted(_)) => MeetingDeclineResult::AlreadyAccepted,
            Some(MeetingDecision::InFlight) => MeetingDeclineResult::InFlight,
            None => MeetingDeclineResult::NotFound,
        }
    }

    pub(crate) async fn expire_meeting_detection(&self, detection_id: &str) -> bool {
        let now = Instant::now();
        let mut state = self.meeting_detection.lock().await;
        let expired = state.pending.as_ref().is_some_and(|pending| {
            pending.detection.detection_id == detection_id && pending.expires_at <= now
        });
        if expired {
            state.pending = None;
            state
                .decisions
                .insert(detection_id.to_string(), (now, MeetingDecision::Declined));
        }
        expired
    }

    pub(crate) async fn clear_pending_meeting_detection(&self) {
        let now = Instant::now();
        let mut state = self.meeting_detection.lock().await;
        if let Some(pending) = state.pending.take() {
            state.decisions.insert(
                pending.detection.detection_id,
                (now, MeetingDecision::Declined),
            );
        }
    }

    pub fn emit_url_import_progress(
        &self,
        progress: f32,
        status: impl Into<String>,
    ) -> Result<(), String> {
        self.app
            .emit(
                URL_IMPORT_PROGRESS,
                UrlImportProgressPayload::new(progress, status),
            )
            .map_err(|e| e.to_string())
    }

    pub async fn begin_recording(&self) -> Result<String, String> {
        self.begin_with_id(Uuid::new_v4()).await
    }

    /// Begin a recording after capturing the focused insertion target.
    ///
    /// The UI microphone button uses this path so that a transcription with
    /// auto-paste knows where to insert. The global shortcut flow captures the
    /// target itself before showing the indicator, so it should call
    /// [`begin_recording`] directly to avoid re-capturing the indicator window.
    #[cfg(target_os = "macos")]
    pub async fn begin_recording_with_target(&self) -> Result<String, String> {
        // Capture the focused insertion target before the recording starts so a
        // later auto-paste knows where to insert. A missing target is not
        // fatal here; the transcription flow surfaces a paste failure later.
        let _ = bridge::capture_insertion_target();
        self.begin_recording().await
    }

    #[cfg(not(target_os = "macos"))]
    pub async fn begin_recording_with_target(&self) -> Result<String, String> {
        self.begin_recording().await
    }

    /// Begin a recording with a client-provided identity.
    ///
    /// Idempotent: if the same recording is already active, or its
    /// transcription is still active, returns its ID without restarting. A
    /// different active ID, or any ID that already has a history row, is
    /// reported as an error so callers cannot reuse or overwrite a completed
    /// lifecycle.
    pub async fn begin_with_id(&self, id: Uuid) -> Result<String, String> {
        let _transition = self.activity_transition.lock().await;
        if self.meeting_session.lock().await.is_some() {
            return Err("stop meeting mode before starting dictation".to_string());
        }
        // Active recording session takes precedence.
        {
            let session = self.session.lock().await;
            if let Some(active) = session.as_ref() {
                if active.id == id {
                    return Ok(id.to_string());
                }
                return Err("recording already in progress".to_string());
            }
        }

        // Same ID currently being transcribed is idempotent.
        {
            let active = self.active_transcription.lock().await;
            if let Some(task) = active.as_ref() {
                if task.recording_id == id {
                    return Ok(id.to_string());
                }
                return Err("another transcription is active".to_string());
            }
        }

        // Reject IDs that already have a history row of any status.
        {
            let history = self.history.lock().map_err(|e| e.to_string())?;
            if history.get(id).map_err(|e| e.to_string())?.is_some() {
                return Err(format!("recording {id} already exists in history"));
            }
        }

        #[cfg(target_os = "macos")]
        bridge::microphone_recording_readiness().map_err(str::to_string)?;

        let recordings_dir = self.recordings_dir()?;
        std::fs::create_dir_all(&recordings_dir).map_err(|e| e.to_string())?;
        let output_path = recordings_dir.join(format!("{id}.wav"));
        let selected_device = self
            .config
            .lock()
            .map_err(|error| error.to_string())?
            .get()
            .audio_input_device_id
            .clone();
        let mut input_config = AudioInputConfig::default();
        input_config.device_id = selected_device;

        let keep_awake = KeepAwake::begin("UltraVox dictation");
        let mut audio = self.audio.lock().await;
        // The audio lock may have waited; only a fresh native Authorized permits capture.
        #[cfg(target_os = "macos")]
        bridge::microphone_recording_readiness().map_err(str::to_string)?;
        let recording = match audio.start_recording(input_config, output_path.clone()).await {
            Ok(recording) => recording,
            Err(error) => {
                // A failed or timed-out start must not leave a stray WAV behind.
                if !matches!(error, ultravox_core::audio::AudioError::AlreadyRecording) {
                    let _ = std::fs::remove_file(&output_path);
                }
                return Err(error.to_string());
            }
        };

        let mut session = self.session.lock().await;
        *session = Some(RecordingSession {
            id,
            recording: recording.clone(),
            started_at: Instant::now(),
            keep_awake,
        });
        drop(session);
        self.clear_pending_meeting_detection().await;
        crate::close_meeting_reminder(&self.app);
        self.emit_recording_started(&recording)?;
        self.record_telemetry_usage(crate::telemetry::UsageCounters {
            recordings_started: 1,
            ..crate::telemetry::UsageCounters::default()
        });
        Ok(id.to_string())
    }

    pub async fn finish_recording(&self, allow_auto_paste: bool) -> Result<AudioRecording, String> {
        let _transition = self.activity_transition.lock().await;
        // Hold the active-transcription lock throughout the finish path. This
        // reserves the slot so a later recording cannot stop and overwrite it,
        // and lets us take the recording session, release it, and still be safe.
        let mut active = self.active_transcription.lock().await;

        let mut session_guard = self.session.lock().await;
        let session = session_guard.take().ok_or("not recording")?;
        if active
            .as_ref()
            .is_some_and(|a| a.recording_id != session.id)
        {
            // Restore the session so the recording/audio is not orphaned.
            *session_guard = Some(session);
            return Err("another transcription is still active".to_string());
        }
        drop(session_guard);

        let mut audio = self.audio.lock().await;
        let mut recording = match audio.stop_recording().await {
            Ok(recording) => recording,
            Err(error) => {
                drop(audio);
                let fallback = session.recording.clone();
                let _ = std::fs::remove_file(&fallback.output_path);
                self.emit_recording_stopped(&fallback)
                    .map_err(|emit_error| {
                        format!("{error}; failed to notify recording stop: {emit_error}")
                    })?;
                self.record_telemetry_usage(crate::telemetry::UsageCounters {
                    recordings_failed: 1,
                    ..crate::telemetry::UsageCounters::default()
                });
                return Err(error.to_string());
            }
        };
        drop(audio);
        recording.duration_ms = Some(session.started_at.elapsed().as_millis() as u64);

        let language = {
            let cfg = self.config.lock().map_err(|e| e.to_string())?;
            cfg.get().whisper_language.0.clone()
        };

        let mut row = RecordingRow {
            id: session.id,
            timestamp: Utc::now(),
            file_name: recording
                .output_path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("recording.wav")
                .to_string(),
            title: String::new(),
            preview: String::new(),
            transcription: String::new(),
            language,
            duration_seconds: recording.duration_ms.unwrap_or(0) as f64 / 1000.0,
            status: RecordingStatus::Pending,
            progress: 0.0,
            source_file_url: Some(recording.output_path.to_string_lossy().to_string()),
        };
        row.refresh_display();
        {
            let mut history = self.history.lock().map_err(|e| e.to_string())?;
            history.insert(&row).map_err(|e| e.to_string())?;
            self.emit_recording_added(&row)?;
            self.emit_recording_stopped(&recording)?;
        }
        self.record_telemetry_usage(crate::telemetry::UsageCounters {
            recordings_completed: 1,
            ..crate::telemetry::UsageCounters::default()
        });

        let app = self.app.clone();
        let recording_for_task = recording.clone();
        let transcription_task: JoinHandle<Result<String, String>> = tokio::spawn(async move {
            let state = app.state::<AppState>();
            state
                .run_transcription_flow(&recording_for_task, allow_auto_paste)
                .await
        });

        let abort_handle = transcription_task.abort_handle();
        *active = Some(ActiveTranscription {
            recording_id: session.id,
            abort_handle,
        });
        let recording_id = session.id;
        drop(active);

        let app = self.app.clone();
        tokio::spawn(async move {
            let result = transcription_task.await;
            let state = app.state::<AppState>();
            let mut active = state.active_transcription.lock().await;
            if active
                .as_ref()
                .is_some_and(|a| a.recording_id == recording_id)
            {
                active.take();
            }
            if let Err(error) = result {
                // Aborted tasks are expected on cancellation; avoid logging them
                // as unexpected failures.
                if !error.is_cancelled() {
                    eprintln!("transcription task failed: {error}");
                }
            }
        });

        Ok(recording)
    }

    pub async fn queue_managed_audio(
        &self,
        recording: AudioRecording,
        allow_auto_paste: bool,
        allow_active_meeting: bool,
    ) -> Result<String, String> {
        let _transition = self.activity_transition.lock().await;
        let id = recording_id_for(&recording);
        if !recording.output_path.is_file() {
            return Err(format!(
                "audio file is missing: {}",
                recording.output_path.display()
            ));
        }

        let mut active = self.active_transcription.lock().await;
        if active.is_some() {
            return Err("wait for the active transcription to finish".to_string());
        }
        if self.session.lock().await.is_some() {
            return Err("stop dictation before importing audio".to_string());
        }
        if !allow_active_meeting && self.meeting_session.lock().await.is_some() {
            return Err("stop meeting mode before importing audio".to_string());
        }

        let language = {
            let cfg = self.config.lock().map_err(|e| e.to_string())?;
            cfg.get().whisper_language.0.clone()
        };
        let mut row = RecordingRow {
            id,
            timestamp: Utc::now(),
            file_name: recording
                .output_path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("recording.wav")
                .to_string(),
            title: String::new(),
            preview: String::new(),
            transcription: String::new(),
            language,
            duration_seconds: recording.duration_ms.unwrap_or(0) as f64 / 1_000.0,
            status: RecordingStatus::Pending,
            progress: 0.0,
            source_file_url: Some(recording.output_path.to_string_lossy().to_string()),
        };
        row.refresh_display();
        {
            let mut history = self.history.lock().map_err(|e| e.to_string())?;
            if history.get(id).map_err(|e| e.to_string())?.is_some() {
                return Err(format!("recording {id} already exists in history"));
            }
            history.insert(&row).map_err(|e| e.to_string())?;
        }
        self.emit_recording_added(&row)?;

        let app = self.app.clone();
        let recording_for_task = recording.clone();
        let transcription_task: JoinHandle<Result<String, String>> = tokio::spawn(async move {
            let state = app.state::<AppState>();
            state
                .run_transcription_flow(&recording_for_task, allow_auto_paste)
                .await
        });
        let abort_handle = transcription_task.abort_handle();
        *active = Some(ActiveTranscription {
            recording_id: id,
            abort_handle,
        });
        drop(active);

        let app = self.app.clone();
        tokio::spawn(async move {
            let result = transcription_task.await;
            let state = app.state::<AppState>();
            let mut active = state.active_transcription.lock().await;
            if active.as_ref().is_some_and(|task| task.recording_id == id) {
                active.take();
            }
            if let Err(error) = result {
                if !error.is_cancelled() {
                    eprintln!("managed audio transcription task failed: {error}");
                }
            }
        });

        Ok(id.to_string())
    }

    pub async fn cancel_active_recording(&self) -> Result<Option<Uuid>, String> {
        let mut session_guard = self.session.lock().await;
        let Some(session) = session_guard.take() else {
            return Ok(None);
        };
        drop(session_guard);

        let mut audio = self.audio.lock().await;
        let result = audio.stop_recording().await;
        drop(audio);
        let recording = match result {
            Ok(recording) => recording,
            Err(error) => {
                let fallback = session.recording.clone();
                let _ = std::fs::remove_file(&fallback.output_path);
                self.emit_recording_stopped(&fallback)
                    .map_err(|emit_error| {
                        format!("{error}; failed to notify recording stop: {emit_error}")
                    })?;
                return Err(error.to_string());
            }
        };
        let _ = std::fs::remove_file(&recording.output_path);
        self.emit_recording_stopped(&recording)?;
        Ok(Some(session.id))
    }

    /// Cancel a recording or transcription by recording ID.
    ///
    /// If the ID matches an active recording session, audio capture is stopped
    /// and the partial file is removed. If the ID matches an active
    /// transcription task, the native engine is asked to cancel; the Rust slot
    /// remains reserved until the native task actually unwinds.
    ///
    /// Returns `true` when the ID was found and cancelled, `false` when there
    /// was nothing active for the ID.
    pub async fn cancel_recording_or_transcription(&self, id: Uuid) -> Result<bool, String> {
        // Cancel an active recording session.
        {
            let mut session_guard = self.session.lock().await;
            if let Some(active) = session_guard.as_ref() {
                if active.id == id {
                    let active = session_guard
                        .take()
                        .ok_or_else(|| "active recording disappeared".to_string())?;
                    drop(session_guard);
                    let mut audio = self.audio.lock().await;
                    let result = audio.stop_recording().await;
                    drop(audio);
                    let recording =
                        match result {
                            Ok(recording) => recording,
                            Err(error) => {
                                let fallback = active.recording.clone();
                                let _ = std::fs::remove_file(&fallback.output_path);
                                self.emit_recording_stopped(&fallback).map_err(|emit_error| {
                                format!("{error}; failed to notify recording stop: {emit_error}")
                            })?;
                                if cancellation_may_discard_audio(&error) {
                                    return Ok(true);
                                }
                                return Err(error.to_string());
                            }
                        };
                    let _ = std::fs::remove_file(&recording.output_path);
                    self.emit_recording_stopped(&recording)?;
                    return Ok(true);
                }
                return Err("another recording is active".to_string());
            }
        }

        // Cancel an active transcription task. The task is aborted first so
        // the Rust side cannot continue after a successful cancellation, then
        // the native engine is asked to cancel the same recording identity.
        // The native call may fail because the Swift actor has not claimed the
        // job yet, but the abort handle already stops the Rust task from using
        // any result, closing the handoff race.
        let mut cancelled = false;
        {
            let active = self.active_transcription.lock().await;
            if let Some(task) = active.as_ref() {
                if task.recording_id == id {
                    task.abort_handle.abort();
                    ultravox_core::phonon::cancel(&id.to_string());
                    #[cfg(target_os = "macos")]
                    {
                        let _ = bridge::cancel_transcription(&id.to_string());
                    }
                    cancelled = true;
                } else {
                    return Err("another transcription is active".to_string());
                }
            }
        }

        // Mark the history row as cancelled if it exists and is not terminal.
        {
            let mut history = self.history.lock().map_err(|e| e.to_string())?;
            if let Some(mut row) = history.get(id).map_err(|e| e.to_string())? {
                if row.status == RecordingStatus::Completed
                    || row.status == RecordingStatus::Failed
                    || row.status == RecordingStatus::Cancelled
                {
                    return Ok(false);
                }
                row.status = RecordingStatus::Cancelled;
                row.progress = 0.0;
                row.refresh_display();
                history.insert(&row).map_err(|e| e.to_string())?;
                self.emit_recording_added(&row)?;
                self.emit_transcription_progress(id.to_string(), 0.0, "cancelled")?;
                self.emit_transcription_completed(id.to_string(), "", None::<String>)?;
                return Ok(true);
            }
        }

        Ok(cancelled)
    }

    pub async fn retry_transcription(&self, id: Uuid) -> Result<String, String> {
        let _transition = self.activity_transition.lock().await;
        if self.meeting_session.lock().await.is_some() {
            return Err("stop meeting mode before retrying a transcription".to_string());
        }
        let mut active = self.active_transcription.lock().await;
        if active.is_some() {
            return Err("wait for the active transcription to finish before retrying".to_string());
        }

        let recording = {
            let mut history = self.history.lock().map_err(|error| error.to_string())?;
            let row = history
                .get(id)
                .map_err(|error| error.to_string())?
                .ok_or_else(|| "recording not found".to_string())?;
            if row.status != RecordingStatus::Failed {
                return Err("only failed transcriptions can be retried".to_string());
            }
            let source = row
                .source_file_url
                .as_deref()
                .ok_or_else(|| "recording has no source audio file".to_string())?;
            let output_path = PathBuf::from(source);
            if !output_path.is_file() {
                return Err(format!(
                    "recording audio is missing: {}",
                    output_path.display()
                ));
            }
            let recording = AudioRecording {
                id: id.to_string(),
                output_path,
                start_time_ms: 0,
                duration_ms: Some((row.duration_seconds.max(0.0) * 1_000.0).round() as u64),
            };
            history.retry(id).map_err(|error| error.to_string())?;
            let pending = history
                .get(id)
                .map_err(|error| error.to_string())?
                .ok_or_else(|| "recording not found after retry".to_string())?;
            self.emit_recording_added(&pending)?;
            recording
        };

        let app = self.app.clone();
        let recording_for_task = recording.clone();
        let transcription_task: JoinHandle<Result<String, String>> = tokio::spawn(async move {
            let state = app.state::<AppState>();
            state
                .run_transcription_flow(&recording_for_task, false)
                .await
        });
        let abort_handle = transcription_task.abort_handle();
        *active = Some(ActiveTranscription {
            recording_id: id,
            abort_handle,
        });
        drop(active);

        let app = self.app.clone();
        tokio::spawn(async move {
            let result = transcription_task.await;
            let state = app.state::<AppState>();
            let mut active = state.active_transcription.lock().await;
            if active.as_ref().is_some_and(|task| task.recording_id == id) {
                active.take();
            }
            if let Err(error) = result {
                if !error.is_cancelled() {
                    eprintln!("retry transcription task failed: {error}");
                }
            }
        });

        Ok(id.to_string())
    }

    /// Startup work for the speech models: finish the upgrade to Phonon-2 (download it for
    /// English users, remove the English models it replaces) and load the selected model.
    pub fn warm_transcription_model(&self) {
        // `models_dir()` takes the config lock itself, so resolve it before holding the lock here.
        let models_dir = match self.models_dir() {
            Ok(dir) => dir,
            Err(error) => {
                eprintln!("model warmup skipped: {error}");
                return;
            }
        };
        let (english, version, directory) = match self.config.lock() {
            Ok(config) => {
                let config = config.get();
                (
                    config.model_language != "multilingual"
                        && config.fluid_audio_model_version != "v3",
                    config.fluid_audio_model_version.clone(),
                    config.models_directory.clone(),
                )
            }
            Err(error) => {
                eprintln!("model warmup skipped: config lock poisoned: {error}");
                return;
            }
        };
        #[cfg(not(target_os = "macos"))]
        let _ = (&version, &directory);
        tauri::async_runtime::spawn(async move {
            let started = Instant::now();
            if english && !ultravox_core::phonon::is_installed(&models_dir) {
                // Existing installs keep working on their old English model until this finishes.
                if let Err(error) = ultravox_core::phonon::install(models_dir.clone()).await {
                    eprintln!("Phonon-2 download failed: {error}");
                }
            }
            // Load Phonon-2 first: the old English models are only removed once it has proven
            // it runs on this machine. Multilingual users never read them, so theirs go now.
            let mut phonon_ready = false;
            if english && ultravox_core::phonon::is_installed(&models_dir) {
                let loaded = tokio::task::spawn_blocking({
                    let models_dir = models_dir.clone();
                    move || ultravox_core::phonon::prepare(&models_dir)
                })
                .await;
                match loaded {
                    Ok(Ok(())) => {
                        phonon_ready = true;
                        eprintln!("Phonon-2 ready in {:.2}s", started.elapsed().as_secs_f64());
                    }
                    Ok(Err(error)) => eprintln!(
                        "Phonon-2 failed to load, keeping the previous English model: {error}"
                    ),
                    Err(error) => eprintln!("Phonon-2 warmup task failed: {error}"),
                }
            }
            if phonon_ready || !english {
                let removed = tokio::task::spawn_blocking({
                    let models_dir = models_dir.clone();
                    move || {
                        ultravox_core::phonon::cleanup_superseded(
                            &models_dir,
                            &superseded_fluid_audio_dirs(),
                        )
                    }
                })
                .await
                .unwrap_or_default();
                for path in removed {
                    eprintln!("removed superseded model {}", path.display());
                }
            }
            if english {
                return;
            }
            #[cfg(target_os = "macos")]
            {
                if !bridge::is_model_downloaded(&version, directory.as_deref()) {
                    eprintln!("Parakeet warmup skipped: selected model is not downloaded");
                    return;
                }
                let warmup = tokio::task::spawn_blocking(move || {
                    bridge::prepare_model(&version, directory.as_deref())
                })
                .await;
                match warmup {
                    Ok(true) => eprintln!(
                        "Parakeet model ready in {:.2}s",
                        started.elapsed().as_secs_f64()
                    ),
                    Ok(false) => eprintln!("Parakeet model warmup failed"),
                    Err(error) => eprintln!("Parakeet model warmup task failed: {error}"),
                }
            }
        });
    }

    /// Runs the full transcription flow for a completed recording: update
    /// history to `Transcribing`, call the native transcription bridge, update
    /// history with the result, emit completion events, and optionally paste.
    ///
    /// Pipeline: audio -> history -> transcribe -> paste/events.
    pub async fn run_transcription_flow(
        &self,
        recording: &AudioRecording,
        allow_auto_paste: bool,
    ) -> Result<String, String> {
        let _keep_awake = KeepAwake::begin("UltraVox transcription");
        #[cfg(target_os = "macos")]
        let _indicator_guard = NativeIndicatorGuard;
        #[cfg(target_os = "macos")]
        bridge::set_indicator_state("transcribing");

        let id = recording_id_for(recording);
        let visible_id = id.to_string();
        let audio_path = recording.output_path.clone();

        // Compile the manual and permissioned Retex vocabulary once per job.
        // Settings writes validate both sources, so a hand-edited invalid file
        // degrades to no dictionary rather than blocking transcription.
        let (language, auto_copy, auto_paste, add_space, dictionary) = {
            let cfg = self.config.lock().map_err(|e| e.to_string())?;
            let cfg = cfg.get();
            (
                cfg.whisper_language.0.clone(),
                cfg.auto_copy_to_clipboard,
                cfg.auto_paste_transcription,
                cfg.add_space_after_sentence,
                CustomDictionary::from_sources(&cfg.custom_dictionary, &cfg.retex_dictionary)
                    .unwrap_or_default(),
            )
        };

        // Move the recording row from Pending to Transcribing. If the row cannot
        // be found, create a failed row so the failure is visible instead of silent.
        // If the recording has already been cancelled (e.g. a cancel raced ahead of
        // this task), preserve the cancelled state and do no work.
        {
            let mut history = self.history.lock().map_err(|e| e.to_string())?;
            if let Some(mut row) = history.get(id).map_err(|e| e.to_string())? {
                if row.status == RecordingStatus::Cancelled {
                    self.emit_transcription_progress(&visible_id, 0.0, "cancelled")?;
                    return Err("transcription cancelled".to_string());
                }
                row.status = RecordingStatus::Transcribing;
                row.progress = 0.1;
                row.language = language.clone();
                row.refresh_display();
                history.insert(&row).map_err(|e| e.to_string())?;
                self.emit_recording_added(&row)?;
            } else {
                let row = fallback_row(
                    recording,
                    id,
                    language.clone(),
                    "recording row not found for transcription".to_string(),
                    RecordingStatus::Failed,
                    1.0,
                );
                history.insert(&row).map_err(|e| e.to_string())?;
                self.emit_recording_added(&row)?;
                self.emit_transcription_progress(&visible_id, 1.0, "failed")?;
                self.emit_transcription_completed(
                    &visible_id,
                    row.transcription.clone(),
                    Some(language.clone()),
                )?;
                return Err(row.transcription);
            }
        }
        self.emit_transcription_progress(&visible_id, 0.1, "transcribing")?;

        // Native engines are CPU-bound, so keep them off the async runtime.
        let models_dir_for_phonon = self.models_dir()?;
        #[cfg(target_os = "macos")]
        let (fluid_version, models_directory, recording_id_for_task) = {
            let cfg = self.config.lock().map_err(|e| e.to_string())?;
            (
                cfg.get().fluid_audio_model_version.clone(),
                cfg.get().models_directory.clone(),
                id.to_string(),
            )
        };
        #[cfg(not(target_os = "macos"))]
        let recording_id_for_task = id.to_string();
        #[cfg(not(target_os = "macos"))]
        let (model_path, whisper_options, english_selected) = {
            let models_dir = self.models_dir()?;
            let cfg = self.config.lock().map_err(|e| e.to_string())?;
            let config = cfg.get();
            // English is Phonon-2; the Whisper base.en file is only a fallback while an
            // upgraded install has not downloaded Phonon-2 yet.
            let filename = if config.model_language == "multilingual" {
                "ggml-base.bin"
            } else {
                "ggml-base.en.bin"
            };
            (
                config
                    .selected_whisper_model_path
                    .clone()
                    .unwrap_or_else(|| models_dir.join(filename)),
                WhisperOptions {
                    language: config.whisper_language.0.clone(),
                    translate: config.translate_to_english,
                    suppress_blank: config.suppress_blank_audio,
                    show_timestamps: config.show_timestamps,
                    temperature: config.temperature as f32,
                    no_speech_threshold: config.no_speech_threshold as f32,
                    initial_prompt: dictionary.initial_prompt(&config.initial_prompt),
                    use_beam_search: config.use_beam_search,
                    beam_size: config.beam_size as i32,
                },
                config.model_language != "multilingual",
            )
        };
        #[cfg(target_os = "macos")]
        let english = fluid_version != "v3";
        #[cfg(not(target_os = "macos"))]
        let english = english_selected;
        let text = tokio::task::spawn_blocking(move || {
            // Validate before model selection/fallback, retaining derived audio until decoding ends.
            let input = ultravox_core::prepare_transcription_audio(&audio_path)
                .map_err(|e| e.to_string())?;
            let audio_path = input.path();
            #[cfg(target_os = "macos")]
            let path = audio_path.to_string_lossy();
            if english {
                let mut phonon_error = None;
                if ultravox_core::phonon::is_installed(&models_dir_for_phonon) {
                    match ultravox_core::phonon::transcribe_file(
                        &models_dir_for_phonon,
                        &audio_path,
                        Some(&recording_id_for_task),
                    ) {
                        Ok(text) => return Ok(text),
                        Err(ultravox_core::phonon::PhononError::Cancelled) => {
                            return Err("transcription cancelled".to_string())
                        }
                        Err(error) => phonon_error = Some(error.to_string()),
                    }
                }
                // Phonon-2 is still downloading, or could not run on this machine: use the
                // previous English model if it is still on disk (it is only removed after
                // Phonon-2 has loaded successfully).
                #[cfg(target_os = "macos")]
                let legacy_present = bridge::is_model_downloaded("v2", models_directory.as_deref());
                #[cfg(not(target_os = "macos"))]
                let legacy_present = model_path.is_file();
                if !legacy_present {
                    return Err(phonon_error.unwrap_or_else(|| {
                        "Phonon-2 is still downloading; try again in a minute".to_string()
                    }));
                }
                if let Some(error) = phonon_error {
                    eprintln!("Phonon-2 failed ({error}); using the previous English model");
                }
            }
            #[cfg(target_os = "macos")]
            {
                bridge::transcribe_file_with_version_for_recording_in_directory(
                    &path,
                    &fluid_version,
                    &recording_id_for_task,
                    models_directory.as_deref(),
                )
                .map_err(|_| "transcription failed".to_string())
                .and_then(|text| {
                    if text.trim().is_empty() {
                        Err(format!(
                            "transcription returned empty text: {}",
                            ultravox_core::phonon::explain_empty(audio_path)
                        ))
                    } else {
                        Ok(text)
                    }
                })
            }
            #[cfg(not(target_os = "macos"))]
            {
                transcribe_with_whisper(&audio_path, &model_path, whisper_options)
            }
        })
        .await
        .map_err(|e| e.to_string())?;

        // Route empty backend or post-processed output through the same failure
        // path as decode errors, before Completed/history/clipboard side effects.
        let text = text.and_then(|text| {
            let text = require_transcription_text(text, &recording.output_path)?;
            let processed_text =
                require_transcription_text(dictionary.apply(&text), &recording.output_path)?;
            Ok(
                if add_space && processed_text.ends_with(|c: char| c.is_ascii_punctuation()) {
                    format!("{processed_text} ")
                } else {
                    processed_text
                },
            )
        });

        match text {
            Ok(final_text) => {
                // Update history with the completed transcription. Do not
                // overwrite a row that has already been cancelled or marked as
                // failed by a different path.
                {
                    let mut history = self.history.lock().map_err(|e| e.to_string())?;
                    if let Some(mut row) = history.get(id).map_err(|e| e.to_string())? {
                        if row.status == RecordingStatus::Cancelled
                            || row.status == RecordingStatus::Failed
                        {
                            return Err(
                                "transcription result discarded: already terminal".to_string()
                            );
                        }
                        row.transcription = final_text.clone();
                        row.status = RecordingStatus::Completed;
                        row.progress = 1.0;
                        row.language = language.clone();
                        row.refresh_display();
                        history.insert(&row).map_err(|e| e.to_string())?;
                        self.emit_recording_added(&row)?;
                    } else {
                        let row = fallback_row(
                            recording,
                            id,
                            language.clone(),
                            "recording row disappeared during transcription".to_string(),
                            RecordingStatus::Failed,
                            1.0,
                        );
                        history.insert(&row).map_err(|e| e.to_string())?;
                        self.emit_recording_added(&row)?;
                        self.emit_transcription_progress(&visible_id, 1.0, "failed")?;
                        self.emit_transcription_completed(
                            &visible_id,
                            row.transcription.clone(),
                            Some(language.clone()),
                        )?;
                        self.record_telemetry_usage(crate::telemetry::UsageCounters {
                            transcriptions_failed: 1,
                            ..crate::telemetry::UsageCounters::default()
                        });
                        return Err(row.transcription);
                    }
                }
                self.emit_transcription_progress(&visible_id, 1.0, "completed")?;
                self.emit_transcription_completed(
                    &visible_id,
                    final_text.clone(),
                    Some(language.clone()),
                )?;
                self.record_telemetry_usage(crate::telemetry::UsageCounters {
                    transcriptions_completed: 1,
                    ..crate::telemetry::UsageCounters::default()
                });

                // Auto-copy to the system clipboard if enabled.
                if auto_copy {
                    if let Err(e) = self.app.clipboard().write_text(final_text.clone()) {
                        eprintln!("failed to copy transcript to clipboard: {e}");
                    }
                }

                // Insert into the element captured when the shortcut started.
                // Failed delivery is surfaced instead of silently reporting a
                // successful end-to-end transcription.
                #[cfg(target_os = "macos")]
                if auto_paste && allow_auto_paste && bridge::paste_text(&final_text) <= 0 {
                    // Keep the transcript recoverable: the indicator tells the
                    // user to paste it manually, so it must be on the clipboard.
                    if !auto_copy {
                        if let Err(e) = self.app.clipboard().write_text(final_text.clone()) {
                            eprintln!("failed to copy transcript after paste failure: {e}");
                        }
                    }
                    let message = if bridge::is_accessibility_trusted(false) {
                        "transcription completed, but UltraVox could not insert it into the original text field; select the field and try again"
                    } else {
                        "transcription completed, but UltraVox could not insert it; enable UltraVox in System Settings > Privacy & Security > Accessibility"
                    };
                    eprintln!("paste in place failed for recording {id}; transcript left on the clipboard");
                    bridge::set_indicator_state("paste-failed");
                    tokio::time::sleep(Duration::from_millis(1_600)).await;
                    return Err(message.to_string());
                }
                #[cfg(target_os = "macos")]
                if auto_paste && allow_auto_paste {
                    bridge::set_indicator_state("inserted");
                }

                Ok(final_text)
            }
            Err(reason) => {
                let failure_text = reason.clone();
                {
                    let mut history = self.history.lock().map_err(|e| e.to_string())?;
                    if let Some(mut row) = history.get(id).map_err(|e| e.to_string())? {
                        if row.status == RecordingStatus::Cancelled
                            || row.status == RecordingStatus::Completed
                        {
                            return Err(
                                "transcription result discarded: already terminal".to_string()
                            );
                        }
                        row.transcription = failure_text.clone();
                        row.status = RecordingStatus::Failed;
                        row.progress = 1.0;
                        row.language = language.clone();
                        row.refresh_display();
                        history.insert(&row).map_err(|e| e.to_string())?;
                        self.emit_recording_added(&row)?;
                    } else {
                        let row = fallback_row(
                            recording,
                            id,
                            language.clone(),
                            failure_text.clone(),
                            RecordingStatus::Failed,
                            1.0,
                        );
                        history.insert(&row).map_err(|e| e.to_string())?;
                        self.emit_recording_added(&row)?;
                    }
                }
                self.emit_transcription_progress(&visible_id, 1.0, "failed")?;
                self.emit_transcription_completed(
                    &visible_id,
                    failure_text.clone(),
                    Some(language.clone()),
                )?;
                self.record_telemetry_usage(crate::telemetry::UsageCounters {
                    transcriptions_failed: 1,
                    ..crate::telemetry::UsageCounters::default()
                });
                #[cfg(target_os = "macos")]
                {
                    bridge::set_indicator_state("failed");
                    tokio::time::sleep(Duration::from_millis(1_600)).await;
                }
                Err(failure_text)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::time::{sleep, Duration};
    use uuid::Uuid;

    fn meeting_detection(id: &str, key: char) -> MeetingDetection {
        MeetingDetection {
            version: 1,
            detection_id: id.to_string(),
            provider: crate::events::MeetingProvider::GoogleMeet,
            meeting_key: key.to_string().repeat(64),
            detected_at_ms: 0,
        }
    }

    #[test]
    fn explicit_cancel_can_discard_missing_audio_but_not_other_failures() {
        use ultravox_core::audio::AudioError;
        assert!(cancellation_may_discard_audio(&AudioError::SilentInput));
        assert!(!cancellation_may_discard_audio(&AudioError::PermissionDenied));
        assert!(!cancellation_may_discard_audio(&AudioError::DeviceUnavailable));
    }

    #[test]
    fn completion_rejects_trim_empty_transcripts() {
        for text in ["", " ", "\n\t"] {
            let error =
                require_transcription_text(text.into(), std::path::Path::new("missing.wav"))
                    .unwrap_err();
            assert!(error.contains("transcription returned empty text"));
        }
        assert_eq!(
            require_transcription_text("speech".into(), std::path::Path::new("missing.wav"))
                .unwrap(),
            "speech"
        );
    }

    #[test]
    fn pruning_an_expired_prompt_records_a_terminal_decision() {
        let now = Instant::now();
        let detection = meeting_detection("expired", 'a');
        let mut state = MeetingDetectionState {
            pending: Some(PendingMeetingDetection {
                detection,
                expires_at: now.checked_sub(Duration::from_millis(1)).unwrap(),
            }),
            ..MeetingDetectionState::default()
        };

        assert!(AppState::prune_meeting_detection_state(&mut state, now));
        assert!(state.pending.is_none());
        assert!(matches!(
            state.decisions.get("expired"),
            Some((_, MeetingDecision::Declined))
        ));
    }

    #[test]
    fn failed_accept_never_overwrites_a_newer_or_expired_prompt() {
        let now = Instant::now();
        let newer = meeting_detection("newer", 'b');
        let mut superseded = MeetingDetectionState {
            pending: Some(PendingMeetingDetection {
                detection: newer,
                expires_at: now + Duration::from_secs(30),
            }),
            ..MeetingDetectionState::default()
        };

        assert!(!AppState::release_meeting_accept_state(
            &mut superseded,
            meeting_detection("old", 'a'),
            now + Duration::from_secs(30),
            true,
            now,
        ));
        assert_eq!(
            superseded
                .pending
                .as_ref()
                .map(|pending| pending.detection.detection_id.as_str()),
            Some("newer")
        );
        assert!(matches!(
            superseded.decisions.get("old"),
            Some((_, MeetingDecision::Declined))
        ));

        let mut expired = MeetingDetectionState::default();
        assert!(!AppState::release_meeting_accept_state(
            &mut expired,
            meeting_detection("expired", 'c'),
            now.checked_sub(Duration::from_millis(1)).unwrap(),
            true,
            now,
        ));
        assert!(expired.pending.is_none());
    }

    #[test]
    fn failed_accept_restores_the_current_unexpired_prompt() {
        let now = Instant::now();
        let mut state = MeetingDetectionState::default();
        state
            .decisions
            .insert("current".to_string(), (now, MeetingDecision::InFlight));

        assert!(AppState::release_meeting_accept_state(
            &mut state,
            meeting_detection("current", 'd'),
            now + Duration::from_secs(30),
            true,
            now,
        ));
        assert_eq!(
            state
                .pending
                .as_ref()
                .map(|pending| pending.detection.detection_id.as_str()),
            Some("current")
        );
        assert!(!state.decisions.contains_key("current"));
    }

    #[test]
    fn failed_accept_is_terminal_while_other_activity_blocks_meeting_mode() {
        let now = Instant::now();
        let mut state = MeetingDetectionState::default();
        state
            .decisions
            .insert("blocked".to_string(), (now, MeetingDecision::InFlight));

        assert!(!AppState::release_meeting_accept_state(
            &mut state,
            meeting_detection("blocked", 'e'),
            now + Duration::from_secs(30),
            false,
            now,
        ));
        assert!(state.pending.is_none());
        assert!(matches!(
            state.decisions.get("blocked"),
            Some((_, MeetingDecision::Declined))
        ));
    }

    #[tokio::test]
    async fn active_transcription_abort_handle_cancels_underlying_task() {
        // A transcription task stores an abort handle so that cancellation can
        // stop the Rust task even if the native engine has not yet claimed the
        // work. This test would not compile on the old ActiveTranscription
        // struct and would fail if the abort did not actually cancel the task.
        let task = tokio::spawn(async {
            sleep(Duration::from_secs(3600)).await;
            "finished"
        });
        let handle = task.abort_handle();
        let active = ActiveTranscription {
            recording_id: Uuid::new_v4(),
            abort_handle: handle,
        };
        active.abort_handle.abort();
        let result = task.await;
        assert!(result.is_err(), "expected the task to be aborted");
        assert!(
            result.unwrap_err().is_cancelled(),
            "expected the task to be cancelled, not panic"
        );
    }
}
