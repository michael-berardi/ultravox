use chrono::Utc;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_clipboard_manager::ClipboardExt;
use tokio::sync::Mutex as AsyncMutex;
use tokio::task::{AbortHandle, JoinError, JoinHandle};
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
    IndicatorHidePayload, IndicatorShowPayload, RecordingAddedPayload, RecordingDeletedPayload,
    RecordingStartedPayload, RecordingStoppedPayload, SettingsChangedPayload,
    TranscriptionCompletedPayload, TranscriptionProgressPayload, UrlImportProgressPayload,
    INDICATOR_HIDE, INDICATOR_SHOW, RECORDING_ADDED, RECORDING_DELETED, RECORDING_STARTED,
    RECORDING_STOPPED, SETTINGS_CHANGED, SHORTCUT_TRIGGERED, TRANSCRIPTION_COMPLETED,
    TRANSCRIPTION_PROGRESS, URL_IMPORT_PROGRESS,
};

/// A live recording session tracked by the desktop shell.
#[derive(Debug, Clone)]
pub struct RecordingSession {
    pub id: Uuid,
    pub recording: AudioRecording,
    pub started_at: Instant,
}

/// Handle for an in-flight transcription task so cancellation can target a
/// specific recording without disturbing other work.
#[derive(Debug)]
pub struct ActiveTranscription {
    pub recording_id: Uuid,
    pub abort_handle: AbortHandle,
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
    pub audio: AsyncMutex<CpalAudioBackend>,
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

fn transcription_task_failure(result: Result<Result<String, String>, JoinError>) -> Option<String> {
    match result {
        Ok(Ok(_)) => None,
        Ok(Err(reason)) => Some(reason),
        Err(error) if error.is_cancelled() => None,
        Err(error) => Some(format!("transcription task stopped unexpectedly: {error}")),
    }
}

fn mark_incomplete_recording_failed(
    history: &mut RecordingHistory,
    id: Uuid,
    reason: &str,
) -> Result<Option<RecordingRow>, String> {
    let Some(mut row) = history.get(id).map_err(|error| error.to_string())? else {
        return Ok(None);
    };
    if !matches!(
        row.status,
        RecordingStatus::Pending | RecordingStatus::Converting | RecordingStatus::Transcribing
    ) {
        return Ok(None);
    }
    row.transcription = reason.to_string();
    row.status = RecordingStatus::Failed;
    row.progress = 1.0;
    row.refresh_display();
    history.insert(&row).map_err(|error| error.to_string())?;
    Ok(Some(row))
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

#[cfg(target_os = "macos")]
struct NativeIndicatorGuard;

#[cfg(target_os = "macos")]
impl Drop for NativeIndicatorGuard {
    fn drop(&mut self) {
        bridge::clear_insertion_target();
        bridge::hide_indicator();
    }
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
            audio: AsyncMutex::new(CpalAudioBackend::new()),
        })
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

    fn handle_transcription_task_result(
        &self,
        id: Uuid,
        result: Result<Result<String, String>, JoinError>,
        context: &str,
    ) {
        let Some(reason) = transcription_task_failure(result) else {
            return;
        };

        let update = self
            .history
            .lock()
            .map_err(|error| error.to_string())
            .and_then(|mut history| mark_incomplete_recording_failed(&mut history, id, &reason));
        match update {
            Ok(Some(row)) => {
                if let Err(error) = self
                    .emit_recording_added(&row)
                    .and_then(|_| self.emit_transcription_progress(&id.to_string(), 1.0, "failed"))
                    .and_then(|_| {
                        self.emit_transcription_completed(
                            &id.to_string(),
                            reason.clone(),
                            Some(row.language.clone()),
                        )
                    })
                {
                    eprintln!("{context}: {reason}; failed to emit failure state: {error}");
                } else {
                    eprintln!("{context}: {reason}");
                }
            }
            Ok(None) => eprintln!("{context}: {reason}"),
            Err(error) => eprintln!("{context}: {reason}; failed to persist failure: {error}"),
        }
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
        self.emit_recording_started(&recording)?;
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
            drop(active);
            state.handle_transcription_task_result(
                recording_id,
                result,
                "transcription task failed",
            );
        });

        Ok(recording)
    }

    pub async fn queue_managed_audio(
        &self,
        recording: AudioRecording,
        allow_auto_paste: bool,
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
            drop(active);
            state.handle_transcription_task_result(
                id,
                result,
                "managed audio transcription task failed",
            );
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
            drop(active);
            state.handle_transcription_task_result(id, result, "retry transcription task failed");
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
    /// Pipeline: audio -> history -> transcribe -> paste/events.
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

        // Parse the bounded manual dictionary before starting work, then keep
        // the compiled matcher in memory for this transcription.
        let (language, auto_copy, auto_paste, add_space, dictionary) = {
            let cfg = self.config.lock().map_err(|e| e.to_string())?;
            let cfg = cfg.get();
            (
                cfg.whisper_language.0.clone(),
                cfg.auto_copy_to_clipboard,
                cfg.auto_paste_transcription,
                cfg.add_space_after_sentence,
                CustomDictionary::parse(&cfg.custom_dictionary)
                    .map_err(|error| error.to_string())?,
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
        #[cfg(target_os = "macos")]
        let (path, fluid_version, models_directory, recording_id_for_task) = {
            let cfg = self.config.lock().map_err(|e| e.to_string())?;
            (
                audio_path.to_string_lossy().to_string(),
                cfg.get().fluid_audio_model_version.clone(),
                cfg.get().models_directory.clone(),
                id.to_string(),
            )
        };
        #[cfg(not(target_os = "macos"))]
        let (model_path, whisper_options) = {
            let models_dir = self.models_dir()?;
            let cfg = self.config.lock().map_err(|e| e.to_string())?;
            let config = cfg.get();
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
                    initial_prompt: dictionary.combined_initial_prompt(&config.initial_prompt),
                    use_beam_search: config.use_beam_search,
                    beam_size: config.beam_size as i32,
                },
            )
        };
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
                transcribe_with_whisper(&audio_path, &model_path, whisper_options)
            }
        })
        .await
        .map_err(|e| e.to_string())?;

        match text {
            Ok(text) => {
                // Apply local vocabulary corrections before the transcript is
                // written to history, copied, emitted, or pasted.
                let text = dictionary.apply(&text);
                let final_text = if add_space
                    && !text.is_empty()
                    && text.ends_with(|c: char| c.is_ascii_punctuation())
                {
                    format!("{text} ")
                } else {
                    text
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
                        return Err(row.transcription);
                    }
                }
                self.emit_transcription_progress(&visible_id, 1.0, "completed")?;
                self.emit_transcription_completed(
                    &visible_id,
                    final_text.clone(),
                    Some(language.clone()),
                )?;
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

    fn row(id: Uuid, status: RecordingStatus) -> RecordingRow {
        let mut row = RecordingRow {
            id,
            timestamp: Utc::now(),
            file_name: "recording.wav".to_string(),
            title: String::new(),
            preview: String::new(),
            transcription: String::new(),
            language: "en".to_string(),
            duration_seconds: 1.0,
            status,
            progress: 0.0,
            source_file_url: None,
        };
        row.refresh_display();
        row
    }

    #[test]
    fn watcher_surfaces_inner_task_errors() {
        assert_eq!(
            transcription_task_failure(Ok(Err("invalid dictionary".to_string()))),
            Some("invalid dictionary".to_string())
        );
        assert_eq!(transcription_task_failure(Ok(Ok("done".to_string()))), None);
    }

    #[test]
    fn inner_task_errors_fail_pending_and_transcribing_rows() {
        let mut history = RecordingHistory::new_in_memory().unwrap();
        for status in [RecordingStatus::Pending, RecordingStatus::Transcribing] {
            let id = Uuid::new_v4();
            history.insert(&row(id, status)).unwrap();
            let changed = mark_incomplete_recording_failed(
                &mut history,
                id,
                "dictionary line 1 contains an invalid field",
            )
            .unwrap();
            assert!(changed.is_some());
            let persisted = history.get(id).unwrap().unwrap();
            assert_eq!(persisted.status, RecordingStatus::Failed);
            assert_eq!(persisted.progress, 1.0);
            assert_eq!(
                persisted.transcription,
                "dictionary line 1 contains an invalid field"
            );
        }
    }

    #[test]
    fn task_error_handler_does_not_overwrite_terminal_rows() {
        let mut history = RecordingHistory::new_in_memory().unwrap();
        let id = Uuid::new_v4();
        let mut completed = row(id, RecordingStatus::Completed);
        completed.transcription = "finished".to_string();
        history.insert(&completed).unwrap();

        assert!(
            mark_incomplete_recording_failed(&mut history, id, "late error")
                .unwrap()
                .is_none()
        );
        let persisted = history.get(id).unwrap().unwrap();
        assert_eq!(persisted.status, RecordingStatus::Completed);
        assert_eq!(persisted.transcription, "finished");
    }
}
