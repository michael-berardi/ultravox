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
    DownloadManager, ModelCatalog, RecordingHistory, RecordingRow, RecordingStatus,
};

#[cfg(target_os = "macos")]
use ultravox_macos_bridge as bridge;

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
}

#[derive(Debug, Clone)]
pub struct MeetingSession {
    pub id: Uuid,
    pub output_path: PathBuf,
    pub started_at: Instant,
    pub stopping: bool,
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
pub(crate) enum MeetingDeclineResult {
    Declined,
    AlreadyDeclined,
    AlreadyAccepted,
    InFlight,
    NotFound,
}

#[derive(Debug, Clone)]
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
    pub meeting_session: AsyncMutex<Option<MeetingSession>>,
    meeting_detection: AsyncMutex<MeetingDetectionState>,
    pub audio: AsyncMutex<CpalAudioBackend>,
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

impl AppState {
    /// Initialize the state from the Tauri app handle.
    pub fn new(app: AppHandle) -> Result<Self, String> {
        let app_dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
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
            meeting_session: AsyncMutex::new(None),
            meeting_detection: AsyncMutex::new(MeetingDetectionState::default()),
            audio: AsyncMutex::new(CpalAudioBackend::new()),
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
        let app_dir = self.app.path().app_data_dir().map_err(|e| e.to_string())?;
        Ok(app_dir.join("models"))
    }

    /// Path to the default directory for recorded audio files.
    pub fn recordings_dir(&self) -> Result<PathBuf, String> {
        let app_dir = self.app.path().app_data_dir().map_err(|e| e.to_string())?;
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

    pub(crate) async fn complete_meeting_accept(&self, detection_id: &str, recording_id: String) {
        self.meeting_detection.lock().await.decisions.insert(
            detection_id.to_string(),
            (Instant::now(), MeetingDecision::Accepted(recording_id)),
        );
    }

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
        match bridge::microphone_authorization_status() {
            bridge::MicrophoneAuthorizationStatus::Authorized => {}
            bridge::MicrophoneAuthorizationStatus::NotDetermined => {
                let granted = tokio::task::spawn_blocking(bridge::request_microphone_access)
                    .await
                    .map_err(|error| format!("could not request microphone access: {error}"))?;
                if !granted {
                    return Err(
                        "Microphone access was not granted. Enable UltraVox in System Settings > Privacy & Security > Microphone."
                            .to_string(),
                    );
                }
            }
            bridge::MicrophoneAuthorizationStatus::Denied
            | bridge::MicrophoneAuthorizationStatus::Restricted => {
                return Err(
                    "Microphone access is disabled for UltraVox. Enable it in System Settings > Privacy & Security > Microphone."
                        .to_string(),
                );
            }
        }

        let recordings_dir = self.recordings_dir()?;
        std::fs::create_dir_all(&recordings_dir).map_err(|e| e.to_string())?;
        let output_path = recordings_dir.join(format!("{id}.wav"));

        let mut audio = self.audio.lock().await;
        let recording = audio
            .start_recording(AudioInputConfig::default(), output_path)
            .await
            .map_err(|e| e.to_string())?;

        let mut session = self.session.lock().await;
        *session = Some(RecordingSession {
            id,
            recording: recording.clone(),
            started_at: Instant::now(),
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

    pub fn warm_transcription_model(&self) {
        #[cfg(target_os = "macos")]
        {
            let (version, directory) = match self.config.lock() {
                Ok(config) => (
                    config.get().fluid_audio_model_version.clone(),
                    config.get().models_directory.clone(),
                ),
                Err(error) => {
                    eprintln!("Parakeet warmup skipped: config lock poisoned: {error}");
                    return;
                }
            };
            tauri::async_runtime::spawn(async move {
                if !bridge::is_model_downloaded(&version, directory.as_deref()) {
                    eprintln!("Parakeet warmup skipped: selected model is not downloaded");
                    return;
                }
                let started = Instant::now();
                let warmup = tokio::task::spawn_blocking(move || {
                    bridge::prepare_model(&version, directory.as_deref())
                })
                .await;
                match warmup {
                    Ok(true) => {
                        eprintln!(
                            "Parakeet model ready in {:.2}s",
                            started.elapsed().as_secs_f64()
                        );
                    }
                    Ok(false) => eprintln!("Parakeet model warmup failed"),
                    Err(error) => eprintln!("Parakeet model warmup task failed: {error}"),
                }
            });
        }
    }

    /// Runs the full transcription flow for a completed recording: update
    /// history to `Transcribing`, call the native transcription bridge, update
    /// history with the result, emit completion events, and optionally paste.
    ///
    /// This is the Rust-side wiring that matches the Swift baseline flow:
    /// audio -> history -> transcribe -> paste/events.
    pub async fn run_transcription_flow(
        &self,
        recording: &AudioRecording,
        allow_auto_paste: bool,
    ) -> Result<String, String> {
        #[cfg(target_os = "macos")]
        let _indicator_guard = NativeIndicatorGuard;
        #[cfg(target_os = "macos")]
        bridge::set_indicator_state("transcribing");

        let id = recording_id_for(recording);
        let visible_id = id.to_string();
        let audio_path = recording.output_path.clone();

        // Read config once for language and post-processing settings.
        let (language, auto_copy, auto_paste, add_space) = {
            let cfg = self.config.lock().map_err(|e| e.to_string())?;
            let cfg = cfg.get();
            (
                cfg.whisper_language.0.clone(),
                cfg.auto_copy_to_clipboard,
                cfg.auto_paste_transcription,
                cfg.add_space_after_sentence,
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

        // Transcribe on a blocking thread so a native engine (FluidAudio/CoreML)
        // does not block the async runtime. Use the configured FluidAudio model
        // version so multilingual v3 recordings are transcribed with v3. Pass the
        // recording identity so the native engine can cancel this specific job
        // without touching other work.
        let path = audio_path.to_string_lossy().to_string();
        let (fluid_version, models_directory) = {
            let cfg = self.config.lock().map_err(|e| e.to_string())?;
            (
                cfg.get().fluid_audio_model_version.clone(),
                cfg.get().models_directory.clone(),
            )
        };
        let recording_id_for_task = id.to_string();
        let text = tokio::task::spawn_blocking(move || {
            #[cfg(target_os = "macos")]
            {
                bridge::transcribe_file_with_version_for_recording_in_directory(
                    &path,
                    &fluid_version,
                    &recording_id_for_task,
                    models_directory.as_deref(),
                )
                .map_err(|_| "transcription failed".to_string())
            }
            #[cfg(not(target_os = "macos"))]
            {
                Ok(format!("TODO: placeholder transcription for {path}"))
            }
        })
        .await
        .map_err(|e| e.to_string())?;

        match text {
            Ok(text) => {
                let final_text = if add_space
                    && !text.is_empty()
                    && text.ends_with(|c: char| c.is_ascii_punctuation())
                {
                    format!("{text} ")
                } else {
                    text.clone()
                };

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
                    let message = if bridge::is_accessibility_trusted(false) {
                        "transcription completed, but UltraVox could not insert it into the original text field; select the field and try again"
                    } else {
                        "transcription completed, but UltraVox could not insert it; enable UltraVox in System Settings > Privacy & Security > Accessibility"
                    };
                    bridge::set_indicator_state("paste-failed");
                    tokio::time::sleep(Duration::from_millis(1_600)).await;
                    return Err(message.to_string());
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
