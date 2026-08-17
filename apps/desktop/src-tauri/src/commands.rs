use std::{
    env,
    io::ErrorKind,
    path::{Path, PathBuf},
    process::Stdio,
};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, State};
use tauri_plugin_clipboard_manager::ClipboardExt;
use uuid::Uuid;

use ultravox_core::{
    AppConfig, AudioBackend, AudioDeviceInfo, AudioInputConfig, AudioRecording, DownloadManager,
    DownloadProgress, ModelCatalog, ModelDownload, RecordingRow,
};

#[cfg(target_os = "macos")]
use ultravox_macos_bridge as bridge;

use crate::state::{AppState, MeetingSession};
use crate::update::{self, UpdateInfo, UpdatePreferences};

pub const APP_NAME: &str = "UltraVox";
pub const APP_IDENTIFIER: &str = "com.imploselabs.ultravox";
pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShortcutSettings {
    pub modifier_only_hotkey: String,
    pub key_combination: Option<String>,
    pub hold_to_record: bool,
    pub meeting_key_combination: String,
}

fn remove_managed_recording_file(state: &AppState, file_name: &str) -> Result<(), String> {
    let Some(file_name) = Path::new(file_name).file_name() else {
        return Ok(());
    };
    let path = state.recordings_dir()?.join(file_name);
    match std::fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("failed to delete {}: {error}", path.display())),
    }
}

fn recording_is_tracked(state: &AppState, id: Uuid) -> bool {
    state
        .history
        .lock()
        .ok()
        .and_then(|history| history.get(id).ok())
        .flatten()
        .is_some()
}

fn remove_all_recording_files(state: &AppState) -> Result<(), String> {
    let directory = state.recordings_dir()?;
    for entry in std::fs::read_dir(&directory)
        .map_err(|error| format!("failed to read {}: {error}", directory.display()))?
    {
        let entry = entry.map_err(|error| format!("failed to read recording entry: {error}"))?;
        let file_type = entry
            .file_type()
            .map_err(|error| format!("failed to inspect {}: {error}", entry.path().display()))?;
        if file_type.is_file() || file_type.is_symlink() {
            std::fs::remove_file(entry.path())
                .map_err(|error| format!("failed to delete {}: {error}", entry.path().display()))?;
        } else if file_type.is_dir() && entry.file_name() == ".imports" {
            std::fs::remove_dir_all(entry.path())
                .map_err(|error| format!("failed to delete {}: {error}", entry.path().display()))?;
        }
    }
    Ok(())
}

#[tauri::command]
pub fn copy_to_clipboard(state: State<AppState>, text: String) -> Result<(), String> {
    state
        .app
        .clipboard()
        .write_text(text)
        .map_err(|e| e.to_string())
}

#[derive(Debug, Serialize, Clone)]
pub struct AppInfo {
    pub name: String,
    pub version: String,
    pub identifier: String,
}

#[derive(Debug, Serialize, Clone)]
pub struct AppStatus {
    pub status: String,
    pub recording: bool,
    pub meeting: bool,
    pub transcription: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct PermissionStatus {
    pub microphone: PermissionState,
    pub accessibility: PermissionState,
    pub screen_recording: PermissionState,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionState {
    Granted,
    Denied,
    NotDetermined,
    Unavailable,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionKind {
    Microphone,
    Accessibility,
    ScreenRecording,
}

fn screen_recording_permission_error() -> String {
    "Screen Recording access is disabled for UltraVox. Enable UltraVox in System Settings > Privacy & Security > Screen Recording, then choose Recheck permissions."
        .to_string()
}

fn permission_status() -> PermissionStatus {
    #[cfg(target_os = "macos")]
    {
        let microphone = match bridge::microphone_authorization_status() {
            bridge::MicrophoneAuthorizationStatus::Authorized => PermissionState::Granted,
            bridge::MicrophoneAuthorizationStatus::NotDetermined => PermissionState::NotDetermined,
            bridge::MicrophoneAuthorizationStatus::Denied
            | bridge::MicrophoneAuthorizationStatus::Restricted => PermissionState::Denied,
        };
        return PermissionStatus {
            microphone,
            accessibility: if bridge::is_accessibility_trusted(false) {
                PermissionState::Granted
            } else {
                PermissionState::Denied
            },
            screen_recording: if bridge::screen_recording_authorized() {
                PermissionState::Granted
            } else {
                PermissionState::Denied
            },
        };
    }
    #[cfg(not(target_os = "macos"))]
    {
        PermissionStatus {
            microphone: PermissionState::Unavailable,
            accessibility: PermissionState::Unavailable,
            screen_recording: PermissionState::Unavailable,
        }
    }
}

#[tauri::command]
pub fn get_permission_status() -> PermissionStatus {
    permission_status()
}

#[tauri::command]
pub fn request_permission(kind: PermissionKind) -> Result<PermissionStatus, String> {
    #[cfg(target_os = "macos")]
    match kind {
        PermissionKind::Microphone => {
            let _ = bridge::request_microphone_access();
        }
        PermissionKind::Accessibility => {
            let _ = bridge::is_accessibility_trusted(true);
        }
        PermissionKind::ScreenRecording => {
            let _ = bridge::request_screen_recording_access();
        }
    }
    #[cfg(not(target_os = "macos"))]
    let _ = kind;
    Ok(permission_status())
}

fn permission_settings_pane(kind: &PermissionKind) -> &'static str {
    match kind {
        PermissionKind::Microphone => "Privacy_Microphone",
        PermissionKind::Accessibility => "Privacy_Accessibility",
        PermissionKind::ScreenRecording => "Privacy_ScreenCapture",
    }
}

#[tauri::command]
pub fn open_permission_settings(kind: PermissionKind) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        let pane = permission_settings_pane(&kind);
        std::process::Command::new("open")
            .arg(format!(
                "x-apple.systempreferences:com.apple.preference.security?{pane}"
            ))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|error| format!("failed to open macOS Privacy settings: {error}"))?;
        return Ok(());
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = kind;
        Err("macOS permission settings are unavailable on this platform".to_string())
    }
}
#[tauri::command]
pub fn get_update_preferences(state: State<'_, AppState>) -> Result<UpdatePreferences, String> {
    update::read_preferences(&state.app)
}

#[tauri::command]
pub fn set_update_preferences(
    state: State<'_, AppState>,
    preferences: UpdatePreferences,
) -> Result<(), String> {
    update::write_preferences(&state.app, &preferences)
}

#[tauri::command]
pub async fn check_for_update() -> Result<Option<UpdateInfo>, String> {
    update::check(APP_VERSION).await
}

#[tauri::command]
pub async fn install_update(app: AppHandle, info: UpdateInfo) -> Result<(), String> {
    update::install(app, info).await
}
#[tauri::command]
pub async fn get_app_telemetry_status(
    state: State<'_, AppState>,
) -> Result<crate::telemetry::TelemetryStatus, String> {
    Ok(state.telemetry.status().await)
}

#[tauri::command]
pub async fn set_app_telemetry_enabled(
    state: State<'_, AppState>,
    enabled: bool,
) -> Result<crate::telemetry::TelemetryStatus, String> {
    let status = state.telemetry.set_enabled(enabled).await?;
    if enabled {
        let app = state.app.clone();
        tauri::async_runtime::spawn(async move {
            let state = app.state::<AppState>();
            let _ = state.telemetry.launch().await;
            let _ = state.telemetry.heartbeat().await;
        });
    }
    Ok(status)
}

#[tauri::command]
pub async fn record_app_telemetry_usage(
    state: State<'_, AppState>,
    counters: crate::telemetry::UsageCounters,
) -> Result<(), String> {
    state.telemetry.usage(counters).await
}

#[tauri::command]
pub fn get_app_info() -> AppInfo {
    AppInfo {
        name: APP_NAME.to_string(),
        version: APP_VERSION.to_string(),
        identifier: APP_IDENTIFIER.to_string(),
    }
}

#[tauri::command]
pub async fn get_app_status(state: State<'_, AppState>) -> Result<AppStatus, String> {
    let recording = state.session.lock().await.is_some();
    let meeting = state.meeting_session.lock().await.is_some();
    Ok(AppStatus {
        status: "ready".to_string(),
        recording,
        meeting,
        transcription: "idle".to_string(),
    })
}

// These are no-op stubs in this first pass; implementations live in
// crates/ultravox-macos-bridge/native/UltraVoxMacOSBridge.

#[derive(Debug, Serialize, Clone)]
pub struct BridgeVersion {
    pub version: String,
}

#[derive(Debug, Serialize, Clone)]
pub struct CaretPosition {
    pub x: f64,
    pub y: f64,
    pub found: bool,
}

#[derive(Debug, Serialize, Clone)]
pub struct TranscriptionResult {
    pub text: String,
    pub success: bool,
}

#[cfg(target_os = "macos")]
#[tauri::command]
pub fn bridge_version() -> BridgeVersion {
    BridgeVersion {
        version: bridge::version(),
    }
}

#[cfg(not(target_os = "macos"))]
#[tauri::command]
pub fn bridge_version() -> BridgeVersion {
    BridgeVersion {
        version: "unavailable".to_string(),
    }
}

#[cfg(target_os = "macos")]
#[tauri::command]
pub fn get_caret_position() -> CaretPosition {
    let (x, y, found) = bridge::get_caret_position();
    CaretPosition {
        x,
        y,
        found: found != 0,
    }
}

#[cfg(not(target_os = "macos"))]
#[tauri::command]
pub fn get_caret_position() -> CaretPosition {
    CaretPosition {
        x: 0.0,
        y: 0.0,
        found: false,
    }
}

#[cfg(target_os = "macos")]
#[tauri::command]
pub fn paste_text(text: String) -> i32 {
    bridge::paste_text(&text)
}

#[cfg(not(target_os = "macos"))]
#[tauri::command]
pub fn paste_text(_text: String) -> i32 {
    0
}

#[cfg(target_os = "macos")]
#[tauri::command]
pub fn start_modifier_hotkey(modifier: String) -> i32 {
    bridge::start_modifier_hotkey(&modifier)
}

#[cfg(not(target_os = "macos"))]
#[tauri::command]
pub fn start_modifier_hotkey(_modifier: String) -> i32 {
    0
}

#[cfg(target_os = "macos")]
#[tauri::command]
pub fn stop_modifier_hotkey() -> i32 {
    bridge::stop_modifier_hotkey()
}

#[cfg(not(target_os = "macos"))]
#[tauri::command]
pub fn stop_modifier_hotkey() -> i32 {
    0
}

#[cfg(target_os = "macos")]
#[tauri::command]
pub fn start_key_combination_hotkey(combo: String, hold_to_record: bool) -> i32 {
    bridge::start_key_combination_hotkey(&combo, hold_to_record)
}

#[cfg(not(target_os = "macos"))]
#[tauri::command]
pub fn start_key_combination_hotkey(_combo: String, _hold_to_record: bool) -> i32 {
    0
}

#[cfg(target_os = "macos")]
#[tauri::command]
pub fn stop_key_combination_hotkey() -> i32 {
    bridge::stop_key_combination_hotkey()
}

#[cfg(not(target_os = "macos"))]
#[tauri::command]
pub fn stop_key_combination_hotkey() -> i32 {
    0
}

#[cfg(target_os = "macos")]
#[tauri::command]
pub fn show_indicator(state: State<AppState>, x: f64, y: f64) -> i32 {
    let _ = state.emit_indicator_show(x, y);
    bridge::show_indicator(x, y)
}

#[cfg(not(target_os = "macos"))]
#[tauri::command]
pub fn show_indicator(state: State<AppState>, x: f64, y: f64) -> i32 {
    let _ = state.emit_indicator_show(x, y);
    0
}

#[cfg(target_os = "macos")]
#[tauri::command]
pub fn hide_indicator(state: State<AppState>) -> i32 {
    let _ = state.emit_indicator_hide();
    bridge::hide_indicator()
}

#[cfg(not(target_os = "macos"))]
#[tauri::command]
pub fn hide_indicator(state: State<AppState>) -> i32 {
    let _ = state.emit_indicator_hide();
    0
}

#[cfg(target_os = "macos")]
#[tauri::command]
pub fn transcribe_file(path: String) -> TranscriptionResult {
    match bridge::transcribe_file(&path) {
        Ok(text) => TranscriptionResult {
            text,
            success: true,
        },
        Err(_) => TranscriptionResult {
            text: String::new(),
            success: false,
        },
    }
}

#[cfg(not(target_os = "macos"))]
#[tauri::command]
pub fn transcribe_file(_path: String) -> TranscriptionResult {
    TranscriptionResult {
        text: String::new(),
        success: false,
    }
}

// Core command stubs wired into the Tauri invoke handler. These implement the
// contract expected by the frontend so the app builds and the desktop shell is
// functional. Real implementations will replace the no-op bodies in later
// milestones.

#[tauri::command]
pub fn get_settings(state: State<AppState>) -> Result<AppConfig, String> {
    let manager = state
        .config
        .lock()
        .map_err(|e| format!("lock poisoned: {e}"))?;
    Ok(manager.get().clone())
}

#[tauri::command]
pub fn set_settings(state: State<AppState>, config: AppConfig) -> Result<(), String> {
    let models_dir = match config.models_directory.as_ref() {
        Some(directory) => directory.clone(),
        None => state
            .app
            .path()
            .app_data_dir()
            .map_err(|error| error.to_string())?
            .join("models"),
    };
    std::fs::create_dir_all(&models_dir).map_err(|error| {
        format!(
            "could not use models directory {}: {error}",
            models_dir.display()
        )
    })?;

    let mut manager = state
        .config
        .lock()
        .map_err(|error| format!("lock poisoned: {error}"))?;
    manager
        .set(config.clone())
        .map_err(|error| error.to_string())?;
    drop(manager);

    *state
        .downloads
        .lock()
        .map_err(|error| format!("lock poisoned: {error}"))? =
        DownloadManager::with_models_dir(models_dir);
    state.warm_transcription_model();
    state.emit_settings_changed(&config)?;
    Ok(())
}

#[tauri::command]
pub fn get_model_catalog(state: State<AppState>) -> Result<ModelCatalog, String> {
    Ok(state.catalog.clone())
}

#[tauri::command]
pub fn get_download_progress(state: State<AppState>, id: Uuid) -> Result<DownloadProgress, String> {
    let manager = state
        .downloads
        .lock()
        .map_err(|e| format!("lock poisoned: {e}"))?;
    manager
        .get(id)
        .ok_or_else(|| "download not found".to_string())
}

/// Prepare a model for use. For FluidAudio models this routes to the native
/// Swift bridge which downloads and loads the CoreML assets. For other models
/// it falls back to the HTTP download manager.
#[tauri::command]
pub async fn prepare_model(state: State<'_, AppState>, model_id: String) -> Result<bool, String> {
    let catalog = &state.catalog;
    let model = catalog
        .get(&model_id)
        .ok_or_else(|| "model not found".to_string())?;

    #[cfg(target_os = "macos")]
    if model.family == ultravox_core::ModelFamily::FluidAudio {
        let version = match model.version {
            ultravox_core::ModelVersion::V3 => "v3",
            _ => "v2",
        };
        let directory = state
            .config
            .lock()
            .map_err(|e| format!("lock poisoned: {e}"))?
            .get()
            .models_directory
            .clone();
        let was_downloaded = bridge::is_model_downloaded(version, directory.as_deref());
        let prepared = bridge::prepare_model(version, directory.as_deref());
        if !was_downloaded {
            state.record_telemetry_usage(crate::telemetry::UsageCounters {
                model_downloads_completed: u64::from(prepared),
                model_downloads_failed: u64::from(!prepared),
                ..crate::telemetry::UsageCounters::default()
            });
        }
        return Ok(prepared);
    }

    let request = ModelDownload {
        id: Uuid::new_v4(),
        model_id: model.id.clone(),
        url: model.url.clone(),
        destination: std::path::PathBuf::from(&model.filename),
    };
    let manager = state
        .downloads
        .lock()
        .map_err(|e| format!("lock poisoned: {e}"))?
        .clone();
    manager
        .start_download(request)
        .await
        .map_err(|e| e.to_string())?;
    Ok(true)
}

#[tauri::command]
pub async fn start_download(
    state: State<'_, AppState>,
    request: ModelDownload,
) -> Result<DownloadProgress, String> {
    let manager = state
        .downloads
        .lock()
        .map_err(|e| format!("lock poisoned: {e}"))?
        .clone();
    manager
        .start_download(request)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn is_model_downloaded(state: State<AppState>, model_id: String) -> Result<bool, String> {
    let catalog = &state.catalog;
    let model = catalog
        .get(&model_id)
        .ok_or_else(|| "model not found".to_string())?;

    #[cfg(target_os = "macos")]
    if model.family == ultravox_core::ModelFamily::FluidAudio {
        let version = match model.version {
            ultravox_core::ModelVersion::V3 => "v3",
            _ => "v2",
        };
        let directory = state
            .config
            .lock()
            .map_err(|e| format!("lock poisoned: {e}"))?
            .get()
            .models_directory
            .clone();
        return Ok(bridge::is_model_downloaded(version, directory.as_deref()));
    }

    let manager = state
        .downloads
        .lock()
        .map_err(|e| format!("lock poisoned: {e}"))?;
    Ok(manager.is_downloaded(&model.filename))
}

#[tauri::command]
pub fn get_model_progress(state: State<AppState>, model_id: String) -> Result<f64, String> {
    let model = state
        .catalog
        .get(&model_id)
        .ok_or_else(|| "model not found".to_string())?;
    let version = match model.version {
        ultravox_core::ModelVersion::V3 => "v3",
        _ => "v2",
    };
    #[cfg(target_os = "macos")]
    {
        Ok(bridge::model_progress(version))
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = version;
        Ok(0.0)
    }
}

#[tauri::command]
pub fn cancel_download(state: State<AppState>, id: Uuid) -> Result<(), String> {
    let manager = state
        .downloads
        .lock()
        .map_err(|e| format!("lock poisoned: {e}"))?;
    manager.cancel(id).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_downloads(state: State<AppState>) -> Result<Vec<DownloadProgress>, String> {
    let manager = state
        .downloads
        .lock()
        .map_err(|e| format!("lock poisoned: {e}"))?;
    Ok(manager.list())
}

#[tauri::command]
pub fn list_recordings(state: State<AppState>) -> Result<Vec<RecordingRow>, String> {
    let history = state
        .history
        .lock()
        .map_err(|e| format!("lock poisoned: {e}"))?;
    history.list(100, 0).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_recording(state: State<AppState>, id: Uuid) -> Result<Option<RecordingRow>, String> {
    let history = state
        .history
        .lock()
        .map_err(|e| format!("lock poisoned: {e}"))?;
    history.get(id).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn update_recording(state: State<AppState>, row: RecordingRow) -> Result<RecordingRow, String> {
    let mut history = state
        .history
        .lock()
        .map_err(|e| format!("lock poisoned: {e}"))?;
    history.insert(&row).map_err(|e| e.to_string())?;
    state.emit_recording_added(&row)?;
    Ok(row)
}

#[tauri::command]
pub async fn delete_recording(state: State<'_, AppState>, id: Uuid) -> Result<(), String> {
    let active_transcription = state.active_transcription.lock().await;
    if active_transcription
        .as_ref()
        .is_some_and(|active| active.recording_id == id)
    {
        return Err("cancel or wait for this transcription before deleting it".to_string());
    }
    let active_recording = state.session.lock().await;
    if active_recording
        .as_ref()
        .is_some_and(|session| session.id == id)
    {
        return Err("stop this recording before deleting it".to_string());
    }

    let mut history = state
        .history
        .lock()
        .map_err(|error| format!("lock poisoned: {error}"))?;
    if let Some(row) = history.get(id).map_err(|error| error.to_string())? {
        remove_managed_recording_file(state.inner(), &row.file_name)?;
    }
    history.delete(id).map_err(|error| error.to_string())?;
    state.emit_recording_deleted(id)?;
    Ok(())
}

#[tauri::command]
pub async fn delete_all_recordings(state: State<'_, AppState>) -> Result<usize, String> {
    let active_transcription = state.active_transcription.lock().await;
    if active_transcription.is_some() {
        return Err(
            "wait for the active transcription to finish before deleting history".to_string(),
        );
    }

    let active_recording = state.session.lock().await;
    if active_recording.is_some() {
        return Err("stop the active recording before deleting history".to_string());
    }
    if state.meeting_session.lock().await.is_some() {
        return Err("stop meeting mode before deleting history".to_string());
    }

    let mut history = state
        .history
        .lock()
        .map_err(|error| format!("lock poisoned: {error}"))?;
    let rows = history.list_all().map_err(|error| error.to_string())?;
    remove_all_recording_files(state.inner())?;
    let deleted = history.delete_all().map_err(|error| error.to_string())?;
    for row in rows {
        state.emit_recording_deleted(row.id)?;
    }

    Ok(deleted)
}

fn find_executable(name: &str) -> Option<PathBuf> {
    let mut candidates = vec![
        PathBuf::from("/opt/homebrew/bin").join(name),
        PathBuf::from("/usr/local/bin").join(name),
        PathBuf::from("/usr/bin").join(name),
    ];
    if let Some(path) = env::var_os("PATH") {
        candidates.extend(env::split_paths(&path).map(|directory| directory.join(name)));
    }
    candidates.into_iter().find(|path| path.is_file())
}

fn validate_remote_url(value: &str) -> Result<String, String> {
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed.len() > 4_096 {
        return Err("enter a valid HTTP or HTTPS URL".to_string());
    }
    let url = tauri::Url::parse(trimmed).map_err(|_| "enter a valid URL".to_string())?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return Err("only HTTP and HTTPS media URLs are supported".to_string());
    }
    Ok(url.to_string())
}

fn modifier_conflicts_with_combination(modifier: &str, combination: &str) -> bool {
    let modifier_family = match modifier.to_ascii_lowercase().as_str() {
        "leftoption" | "rightoption" => "option",
        "leftcommand" | "rightcommand" => "command",
        "leftcontrol" | "rightcontrol" => "control",
        "leftshift" | "rightshift" => "shift",
        _ => return false,
    };
    combination
        .split('+')
        .any(|part| part.trim().eq_ignore_ascii_case(modifier_family))
}

async fn probe_duration_ms(ffprobe: &Path, audio_path: &Path) -> Option<u64> {
    let output = tokio::process::Command::new(ffprobe)
        .args([
            "-v",
            "error",
            "-show_entries",
            "format=duration",
            "-of",
            "default=noprint_wrappers=1:nokey=1",
        ])
        .arg(audio_path)
        .output()
        .await
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let seconds = String::from_utf8_lossy(&output.stdout)
        .trim()
        .parse::<f64>()
        .ok()?;
    (seconds.is_finite() && seconds >= 0.0).then_some((seconds * 1_000.0).round() as u64)
}

#[tauri::command]
pub async fn import_url(state: State<'_, AppState>, url: String) -> Result<String, String> {
    let remote_url = validate_remote_url(&url)?;
    if state.session.lock().await.is_some() {
        return Err("stop dictation before importing a URL".to_string());
    }
    if state.meeting_session.lock().await.is_some() {
        return Err("stop meeting mode before importing a URL".to_string());
    }
    if state.active_transcription.lock().await.is_some() {
        return Err("wait for the active transcription to finish".to_string());
    }

    let yt_dlp = find_executable("yt-dlp").ok_or_else(|| {
        "yt-dlp is required for URL transcription. Install it with `brew install yt-dlp`."
            .to_string()
    })?;
    let ffmpeg = find_executable("ffmpeg").ok_or_else(|| {
        "ffmpeg is required for URL transcription. Install it with `brew install ffmpeg`."
            .to_string()
    })?;
    let ffprobe = find_executable("ffprobe");

    let id = Uuid::new_v4();
    let recordings_dir = state.recordings_dir()?;
    tokio::fs::create_dir_all(&recordings_dir)
        .await
        .map_err(|error| format!("could not create recordings directory: {error}"))?;
    let import_dir = recordings_dir.join(".imports").join(id.to_string());
    tokio::fs::create_dir_all(&import_dir)
        .await
        .map_err(|error| format!("could not create media import directory: {error}"))?;
    let output_template = import_dir.join("source.%(ext)s");
    let temporary_output_path = import_dir.join("source.wav");
    let output_path = recordings_dir.join(format!("{id}.wav"));

    if let Err(error) = state.emit_url_import_progress(0.05, "Checking media URL") {
        let _ = tokio::fs::remove_dir_all(&import_dir).await;
        return Err(error);
    }
    let mut download = tokio::process::Command::new(&yt_dlp);
    download
        .arg("--no-playlist")
        .arg("--newline")
        .arg("--max-filesize")
        .arg("2G")
        .arg("--socket-timeout")
        .arg("30")
        .arg("--extract-audio")
        .arg("--audio-format")
        .arg("wav")
        .arg("--audio-quality")
        .arg("0")
        .arg("--ffmpeg-location")
        .arg(&ffmpeg)
        .arg("--output")
        .arg(&output_template)
        .arg(&remote_url)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let download_result = download.output().await;
    let output = match download_result {
        Ok(output) => output,
        Err(error) => {
            let _ = tokio::fs::remove_dir_all(&import_dir).await;
            return Err(format!("could not launch yt-dlp: {error}"));
        }
    };

    if !output.status.success() {
        let _ = tokio::fs::remove_dir_all(&import_dir).await;
        let stderr = String::from_utf8_lossy(&output.stderr);
        let details = stderr
            .lines()
            .rev()
            .take(8)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect::<Vec<_>>()
            .join("\n");
        return Err(if details.is_empty() {
            "the media URL could not be downloaded".to_string()
        } else {
            format!("the media URL could not be downloaded: {details}")
        });
    }
    if !temporary_output_path.is_file() {
        let _ = tokio::fs::remove_dir_all(&import_dir).await;
        return Err("yt-dlp finished without creating an audio file".to_string());
    }
    if let Err(error) = tokio::fs::rename(&temporary_output_path, &output_path).await {
        let _ = tokio::fs::remove_dir_all(&import_dir).await;
        return Err(format!("could not finalize downloaded audio: {error}"));
    }
    let _ = tokio::fs::remove_dir_all(&import_dir).await;

    state.emit_url_import_progress(0.85, "Preparing downloaded audio")?;
    let duration_ms = match ffprobe {
        Some(path) => probe_duration_ms(&path, &output_path).await,
        None => None,
    };
    let recording = AudioRecording {
        id: id.to_string(),
        output_path: output_path.clone(),
        start_time_ms: 0,
        duration_ms,
    };
    if state.meeting_session.lock().await.is_some() {
        let _ = tokio::fs::remove_file(&output_path).await;
        return Err("meeting mode started before the download finished".to_string());
    }
    let result = state.queue_managed_audio(recording, false, false).await;
    if result.is_err() {
        if !recording_is_tracked(state.inner(), id) {
            let _ = tokio::fs::remove_file(output_path).await;
        }
    } else {
        state.emit_url_import_progress(1.0, "Queued for transcription")?;
    }
    result
}

async fn clear_meeting_session(state: &AppState, id: Uuid) -> Result<(), String> {
    let mut meeting = state.meeting_session.lock().await;

    if meeting.as_ref().is_some_and(|active| active.id == id) {
        meeting.take();
    }
    drop(meeting);
    state.emit_meeting_state_changed(false)
}

pub(crate) async fn discard_active_meeting(state: &AppState) {
    let _transition = state.activity_transition.lock().await;
    let session = state.meeting_session.lock().await.take();
    let Some(session) = session else { return };

    #[cfg(target_os = "macos")]
    if let Ok(Ok(audio_path)) = tokio::task::spawn_blocking(bridge::stop_meeting_capture).await {
        let _ = tokio::fs::remove_file(audio_path).await;
    }
    let _ = tokio::fs::remove_file(&session.output_path).await;
    let _ = tokio::fs::remove_file(session.output_path.with_extension("m4a")).await;
    let _ = state.emit_meeting_state_changed(false);
}

#[tauri::command]
pub async fn start_meeting(state: State<'_, AppState>) -> Result<String, String> {
    start_meeting_impl(state.inner()).await
}

pub(crate) async fn start_meeting_impl(state: &AppState) -> Result<String, String> {
    #[cfg(not(target_os = "macos"))]
    {
        let _ = state;
        return Err("meeting mode is only available on macOS".to_string());
    }

    #[cfg(target_os = "macos")]
    {
        let _transition = state.activity_transition.lock().await;
        if state.session.lock().await.is_some() {
            return Err("stop dictation before starting meeting mode".to_string());
        }
        if state.active_transcription.lock().await.is_some() {
            return Err("wait for the active transcription to finish".to_string());
        }
        let existing_meeting = state.meeting_session.lock().await.clone();
        if let Some(session) = existing_meeting {
            if session.stopping {
                return Err("meeting mode is stopping".to_string());
            }
            state.clear_pending_meeting_detection().await;
            crate::close_meeting_reminder(&state.app);
            return Ok(session.id.to_string());
        }
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
                    "Microphone access is disabled for UltraVox. Enable UltraVox in System Settings > Privacy & Security > Microphone."
                        .to_string(),
                );
            }
        }
        if !bridge::screen_recording_authorized() {
            return Err(screen_recording_permission_error());
        }
        let id = Uuid::new_v4();
        let recordings_dir = state.recordings_dir()?;
        tokio::fs::create_dir_all(&recordings_dir)
            .await
            .map_err(|error| format!("could not create recordings directory: {error}"))?;
        let output_path = recordings_dir.join(format!("{id}.mp4"));
        let native_path = output_path.clone();
        let capture_result =
            match tokio::task::spawn_blocking(move || bridge::start_meeting_capture(&native_path))
                .await
            {
                Ok(result) => result,
                Err(error) => {
                    state.record_telemetry_usage(crate::telemetry::UsageCounters {
                        recordings_failed: 1,
                        ..crate::telemetry::UsageCounters::default()
                    });
                    return Err(format!("meeting capture task failed: {error}"));
                }
            };
        if let Err(error) = capture_result {
            state.record_telemetry_usage(crate::telemetry::UsageCounters {
                recordings_failed: 1,
                ..crate::telemetry::UsageCounters::default()
            });
            let _ = tokio::fs::remove_file(&output_path).await;
            return Err(error);
        }

        let mut meeting = state.meeting_session.lock().await;
        *meeting = Some(MeetingSession {
            id,
            output_path,
            started_at: std::time::Instant::now(),
            stopping: false,
        });
        drop(meeting);
        state.emit_meeting_state_changed(true)?;
        state.record_telemetry_usage(crate::telemetry::UsageCounters {
            recordings_started: 1,
            ..crate::telemetry::UsageCounters::default()
        });
        state.clear_pending_meeting_detection().await;
        crate::close_meeting_reminder(&state.app);
        Ok(id.to_string())
    }
}
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MeetingDetectionDecision {
    Accept,
    Decline,
}

#[tauri::command]
pub async fn respond_meeting_detection(
    state: State<'_, AppState>,
    detection_id: String,
    decision: MeetingDetectionDecision,
) -> Result<String, String> {
    match decision {
        MeetingDetectionDecision::Accept => match state.claim_meeting_accept(&detection_id).await {
            crate::state::MeetingAcceptClaim::Claimed(detection, expires_at) => {
                match start_meeting_impl(state.inner()).await {
                    Ok(recording_id) => {
                        state
                            .complete_meeting_accept(&detection.detection_id, recording_id.clone())
                            .await;
                        crate::close_meeting_reminder(&state.app);
                        Ok(recording_id)
                    }
                    Err(error) => {
                        let _transition = state.activity_transition.lock().await;
                        let restore_allowed = state.session.lock().await.is_none()
                            && state.active_transcription.lock().await.is_none()
                            && state.meeting_session.lock().await.is_none();
                        let restored = state
                            .release_meeting_accept(
                                detection,
                                expires_at,
                                restore_allowed,
                                error.clone(),
                            )
                            .await;
                        if !restored && state.pending_meeting_detection().await.is_none() {
                            crate::close_meeting_reminder(&state.app);
                        }
                        Err(error)
                    }
                }
            }
            crate::state::MeetingAcceptClaim::AlreadyAccepted(recording_id) => {
                crate::close_meeting_reminder(&state.app);
                Ok(recording_id)
            }
            crate::state::MeetingAcceptClaim::AlreadyDeclined => {
                Err("meeting detection was declined".to_string())
            }
            crate::state::MeetingAcceptClaim::InFlight => {
                Err("meeting decision is already in progress".to_string())
            }
            crate::state::MeetingAcceptClaim::Failed(error) => Err(error),
        },
        MeetingDetectionDecision::Decline => {
            let result = state.decline_meeting_detection(&detection_id).await;
            match result {
                crate::state::MeetingDeclineResult::Declined
                | crate::state::MeetingDeclineResult::AlreadyDeclined => {
                    crate::close_meeting_reminder(&state.app);
                    Ok("declined".to_string())
                }
                crate::state::MeetingDeclineResult::AlreadyAccepted => {
                    crate::close_meeting_reminder(&state.app);
                    Ok("already_accepted".to_string())
                }
                crate::state::MeetingDeclineResult::InFlight => {
                    Err("meeting decision is already in progress".to_string())
                }
                crate::state::MeetingDeclineResult::NotFound => {
                    Err("meeting detection is not pending".to_string())
                }
            }
        }
    }
}

#[tauri::command]
pub async fn stop_meeting(state: State<'_, AppState>) -> Result<AudioRecording, String> {
    #[cfg(not(target_os = "macos"))]
    {
        let _ = state;
        return Err("meeting mode is only available on macOS".to_string());
    }

    #[cfg(target_os = "macos")]
    {
        let session = {
            let mut meeting = state.meeting_session.lock().await;
            let active = meeting
                .as_mut()
                .ok_or_else(|| "meeting mode is not recording".to_string())?;
            if active.stopping {
                return Err("meeting mode is already stopping".to_string());
            }
            active.stopping = true;
            active.clone()
        };
        let duration_ms = session.started_at.elapsed().as_millis() as u64;

        let capture_result = tokio::task::spawn_blocking(bridge::stop_meeting_capture)
            .await
            .map_err(|error| format!("meeting capture task failed: {error}"))
            .and_then(|result| result);
        let audio_path = match capture_result {
            Ok(path) => path,
            Err(error) => {
                state.record_telemetry_usage(crate::telemetry::UsageCounters {
                    recordings_failed: 1,
                    ..crate::telemetry::UsageCounters::default()
                });
                // Never delete the capture on an error path: ScreenCaptureKit may
                // still be finalizing the file, and even a partial capture is the
                // user's only copy of the meeting. Preserve it and surface the
                // path so the audio can be recovered.
                let preserved_path = session.output_path.clone();
                clear_meeting_session(state.inner(), session.id).await?;
                return Err(format!(
                    "{error} (capture preserved at {})",
                    preserved_path.display()
                ));
            }
        };
        let recording = AudioRecording {
            id: session.id.to_string(),
            output_path: audio_path,
            start_time_ms: 0,
            duration_ms: Some(duration_ms),
        };
        let queued = state
            .queue_managed_audio(recording.clone(), false, true)
            .await;
        if queued.is_err() && !recording_is_tracked(state.inner(), session.id) {
            let _ = tokio::fs::remove_file(&recording.output_path).await;
        }
        clear_meeting_session(state.inner(), session.id).await?;
        if let Err(error) = queued {
            state.record_telemetry_usage(crate::telemetry::UsageCounters {
                recordings_failed: 1,
                ..crate::telemetry::UsageCounters::default()
            });
            return Err(error);
        }
        state.record_telemetry_usage(crate::telemetry::UsageCounters {
            recordings_completed: 1,
            ..crate::telemetry::UsageCounters::default()
        });
        Ok(recording)
    }
}
#[tauri::command]
pub async fn get_pending_meeting_detection(
    state: State<'_, AppState>,
) -> Result<Option<crate::events::MeetingDetectionPendingPayload>, String> {
    Ok(state.pending_meeting_detection().await)
}

#[tauri::command]
pub async fn start_recording(state: State<'_, AppState>) -> Result<String, String> {
    state.begin_recording_with_target().await
}

#[tauri::command]
pub async fn stop_recording(state: State<'_, AppState>) -> Result<AudioRecording, String> {
    state.finish_recording(true).await
}
#[tauri::command]
pub fn get_transcription_status() -> Result<String, String> {
    Ok("idle".to_string())
}

#[tauri::command]
pub fn search_recordings(
    state: State<AppState>,
    query: String,
) -> Result<Vec<RecordingRow>, String> {
    let history = state
        .history
        .lock()
        .map_err(|e| format!("lock poisoned: {e}"))?;
    history.search(&query, 100, 0).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn retry_transcription(state: State<'_, AppState>, id: Uuid) -> Result<String, String> {
    state.retry_transcription(id).await
}

#[tauri::command]
pub fn get_shortcut_settings(state: State<AppState>) -> Result<ShortcutSettings, String> {
    let manager = state
        .config
        .lock()
        .map_err(|e| format!("lock poisoned: {e}"))?;
    let cfg = manager.get();
    Ok(ShortcutSettings {
        modifier_only_hotkey: cfg.modifier_only_hotkey.clone(),
        key_combination: Some(cfg.key_combination.clone()),
        hold_to_record: cfg.hold_to_record,
        meeting_key_combination: cfg.meeting_key_combination.clone(),
    })
}

#[tauri::command]
pub fn set_shortcut_settings(
    state: State<AppState>,
    settings: ShortcutSettings,
) -> Result<(), String> {
    let dictation_shortcut = settings.key_combination.as_deref().unwrap_or_default();
    if !dictation_shortcut.is_empty()
        && dictation_shortcut.eq_ignore_ascii_case(&settings.meeting_key_combination)
    {
        return Err("recording and meeting mode must use different shortcuts".to_string());
    }
    if modifier_conflicts_with_combination(&settings.modifier_only_hotkey, dictation_shortcut) {
        return Err("the modifier-only shortcut conflicts with the recording shortcut".to_string());
    }
    if modifier_conflicts_with_combination(
        &settings.modifier_only_hotkey,
        &settings.meeting_key_combination,
    ) {
        return Err("the modifier-only shortcut conflicts with the meeting shortcut".to_string());
    }
    let mut manager = state
        .config
        .lock()
        .map_err(|e| format!("lock poisoned: {e}"))?;
    let cfg = manager.mutate();
    cfg.modifier_only_hotkey = settings.modifier_only_hotkey;
    if let Some(combo) = settings.key_combination {
        cfg.key_combination = combo;
    }
    cfg.hold_to_record = settings.hold_to_record;
    cfg.meeting_key_combination = settings.meeting_key_combination;
    let config = cfg.clone();
    manager.save().map_err(|e| e.to_string())?;
    state.emit_settings_changed(&config)?;
    Ok(())
}

#[tauri::command]
pub async fn get_audio_devices(state: State<'_, AppState>) -> Result<Vec<AudioDeviceInfo>, String> {
    let audio = state.audio.lock().await;
    audio.list_devices().await.map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_audio_input_config() -> Result<AudioInputConfig, String> {
    Ok(AudioInputConfig::default())
}

#[tauri::command]
pub fn export_recording(
    state: State<AppState>,
    id: Uuid,
    destination: String,
) -> Result<String, String> {
    let history = state
        .history
        .lock()
        .map_err(|e| format!("lock poisoned: {e}"))?;
    let dest = if let Some(relative) = destination.strip_prefix("~/") {
        state
            .app
            .path()
            .home_dir()
            .map_err(|error| error.to_string())?
            .join(relative)
    } else {
        std::path::PathBuf::from(destination)
    };
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent).map_err(|error| {
            format!(
                "could not create export directory {}: {error}",
                parent.display()
            )
        })?;
    }
    history
        .export(id, dest.clone())
        .map_err(|error| error.to_string())?;
    Ok(dest.to_string_lossy().to_string())
}

#[cfg(test)]
mod tests {
    use super::{
        modifier_conflicts_with_combination, permission_settings_pane,
        screen_recording_permission_error, validate_remote_url, PermissionKind,
    };

    #[test]
    fn media_url_requires_http_or_https() {
        assert_eq!(
            validate_remote_url("https://www.youtube.com/watch?v=dQw4w9WgXcQ").unwrap(),
            "https://www.youtube.com/watch?v=dQw4w9WgXcQ"
        );
        assert!(validate_remote_url("file:///tmp/recording.wav").is_err());
        assert!(validate_remote_url("not a URL").is_err());
    }

    #[test]
    fn modifier_only_shortcuts_cannot_prefix_key_combinations() {
        assert!(modifier_conflicts_with_combination(
            "rightOption",
            "Option+M"
        ));
        assert!(modifier_conflicts_with_combination(
            "leftCommand",
            "Command+Shift+M"
        ));
        assert!(!modifier_conflicts_with_combination(
            "rightCommand",
            "Control+M"
        ));
        assert!(!modifier_conflicts_with_combination("none", "Option+M"));
    }

    #[test]
    fn screen_recording_errors_identify_ultravox_as_the_owner() {
        assert_eq!(
            screen_recording_permission_error(),
            "Screen Recording access is disabled for UltraVox. Enable UltraVox in System Settings > Privacy & Security > Screen Recording, then choose Recheck permissions."
        );
    }

    #[test]
    fn screen_recording_settings_open_the_macos_capture_privacy_pane() {
        assert_eq!(
            permission_settings_pane(&PermissionKind::ScreenRecording),
            "Privacy_ScreenCapture"
        );
    }
}
