use std::{
    env,
    io::ErrorKind,
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, State};
use tauri_plugin_clipboard_manager::ClipboardExt;
use uuid::Uuid;

use ultravox_core::{
    decode_media_file_to_wav, AppConfig, AudioBackend, AudioDeviceInfo, AudioInputConfig,
    AudioRecording, CustomDictionary, DownloadManager, DownloadProgress, ModelCatalog,
    ModelDownload, RecordingRow, IMPORT_MAX_BYTES,
};

#[cfg(target_os = "macos")]
use ultra_media_remote::TransportCommand;
#[cfg(target_os = "macos")]
use ultravox_macos_bridge as bridge;

use crate::state::AppState;
use crate::update::{self, UpdateInfo, UpdatePreferences};

pub const APP_NAME: &str = "UltraVox";
pub const APP_IDENTIFIER: &str = "com.imploselabs.ultravox";
/// Bundle identifier used to decide whether the global recording shortcut
/// hands off to UltraTerm instead of the native mini-recorder flow.
pub const ULTRATERM_BUNDLE_IDENTIFIER: &str = "com.libertydesignstudio.ultraterm";
pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");
/// Build flavor compiled into this binary: `official` links the closed Pro
/// module, `open-source` is the public build (see docs/pro-licensing.md).
pub const APP_BUILD: &str = crate::pro_api::APP_BUILD;

/// Theme ids that ship in every build. Any other theme is a Pro theme and is
/// only kept when the Pro entitlement verifies right now.
pub const FREE_THEMES: &[&str] = &[
    "midnight",
    "winamp-hifi",
    "nord-frost",
    "vapor",
    "obsidian-rite",
];
/// Fallback theme whenever a stored or requested theme is not available.
pub const DEFAULT_THEME: &str = "midnight";

/// Keep Pro themes only while Pro is unlocked; otherwise fall back to the
/// default theme. Applied on every config save and load.
pub(crate) fn enforce_theme(theme: &str) -> String {
    if FREE_THEMES.contains(&theme) {
        return theme.to_string();
    }
    if crate::pro_api::verify_unlocked().is_ok() {
        theme.to_string()
    } else {
        DEFAULT_THEME.to_string()
    }
}

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

pub(crate) fn recording_is_tracked(state: &AppState, id: Uuid) -> bool {
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

/// Repository-relative location of the bundled dictionary-review skill, also
/// used as the bundle resource path on every platform.
const DICTIONARY_SKILL_RESOURCE: &str = "skills/dictionary-review/SKILL.md";

#[derive(Debug, Clone, Serialize)]
pub struct DictionarySkillInfo {
    pub path: String,
    pub exists: bool,
    pub text: String,
}

/// Resolve the bundled dictionary-review skill: prefer the installed bundle
/// resource, then fall back to the in-repository copy for source builds.
pub fn dictionary_skill_path(app: &AppHandle) -> PathBuf {
    if let Ok(resource_dir) = app.path().resource_dir() {
        let candidate = resource_dir.join(DICTIONARY_SKILL_RESOURCE);
        if candidate.is_file() {
            return candidate;
        }
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("resources")
        .join(DICTIONARY_SKILL_RESOURCE)
}

#[tauri::command]
pub fn dictionary_skill_info(app: AppHandle) -> DictionarySkillInfo {
    let path = dictionary_skill_path(&app);
    let exists = path.is_file();
    let text = std::fs::read_to_string(&path).unwrap_or_default();
    DictionarySkillInfo {
        path: path.display().to_string(),
        exists,
        text,
    }
}

/// Reveal the bundled skill file in the platform file manager so the user can
/// add it to their own agent. UltraVox never writes outside its own install.
#[tauri::command]
pub fn reveal_dictionary_skill(app: AppHandle) -> Result<(), String> {
    let path = dictionary_skill_path(&app);
    if !path.is_file() {
        return Err(format!(
            "dictionary skill file is missing: {}",
            path.display()
        ));
    }
    #[cfg(target_os = "macos")]
    let opened = std::process::Command::new("open")
        .arg("-R")
        .arg(&path)
        .status();
    #[cfg(target_os = "windows")]
    let opened = std::process::Command::new("explorer")
        .arg(format!("/select,{}", path.display()))
        .status();
    #[cfg(all(unix, not(target_os = "macos")))]
    let opened = std::process::Command::new("xdg-open")
        .arg(path.parent().unwrap_or(&path))
        .status();
    match opened {
        Ok(status) if status.success() => Ok(()),
        Ok(status) => Err(format!("file manager exited with {status}")),
        Err(error) => Err(error.to_string()),
    }
}

#[tauri::command]
pub fn report_frontend_error(message: String) {
    let message: String = message
        .chars()
        .take(2_048)
        .map(|character| {
            if matches!(character, '\n' | '\r') {
                ' '
            } else {
                character
            }
        })
        .collect();
    eprintln!("[frontend] {message}");
}

#[derive(Debug, Serialize, Clone)]
pub struct AppInfo {
    pub name: String,
    pub version: String,
    pub identifier: String,
    /// `open-source` for the public build, `official` for the Pro-enabled build.
    pub build: &'static str,
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

/// Used by the Pro build's Meeting/Lecture capture flow.
#[cfg_attr(not(feature = "ultravox-pro"), allow(dead_code))]
pub(crate) fn screen_recording_permission_error() -> String {
    "Screen Recording access is disabled for UltraVox. Enable UltraVox in System Settings > Privacy & Security > Screen Recording, then choose Recheck permissions."
        .to_string()
}

fn permission_status() -> PermissionStatus {
    #[cfg(debug_assertions)]
    if std::env::var_os("ULTRAVOX_QA_PERMISSIONS_GRANTED").is_some() {
        return PermissionStatus {
            microphone: PermissionState::Granted,
            accessibility: PermissionState::Granted,
            screen_recording: PermissionState::Granted,
        };
    }
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
pub async fn request_permission(kind: PermissionKind) -> Result<PermissionStatus, String> {
    #[cfg(target_os = "macos")]
    tauri::async_runtime::spawn_blocking(move || match kind {
        PermissionKind::Microphone => {
            let _ = bridge::request_microphone_access();
        }
        PermissionKind::Accessibility => {
            let _ = bridge::is_accessibility_trusted(true);
        }
        PermissionKind::ScreenRecording => {
            let _ = bridge::request_screen_recording_access();
        }
    })
    .await
    .map_err(|error| format!("permission request task failed: {error}"))?;
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
        build: APP_BUILD,
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

// Native macOS bridge-backed command response types.

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

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ForwardVoiceTriggerStatus {
    pub handled: bool,
}

/// Publishes a `recordingTriggered` push frame to every live voice-IPC
/// `listen` subscriber (UltraTerm). `handled` is true only when at least one
/// live subscriber existed at publish time, letting the caller suppress the
/// native recorder flow solely on successful handoff.
#[tauri::command]
pub fn forward_voice_trigger(combo: String, action: String) -> ForwardVoiceTriggerStatus {
    ForwardVoiceTriggerStatus {
        handled: crate::voice_ipc::publish_voice_trigger(&combo, &action),
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FrontmostAppBundleIdentifier {
    pub bundle_identifier: Option<String>,
}

#[cfg(target_os = "macos")]
#[tauri::command]
pub fn frontmost_app() -> FrontmostAppBundleIdentifier {
    FrontmostAppBundleIdentifier {
        bundle_identifier: bridge::frontmost_application_bundle_id(),
    }
}

#[cfg(not(target_os = "macos"))]
#[tauri::command]
pub fn frontmost_app() -> FrontmostAppBundleIdentifier {
    FrontmostAppBundleIdentifier {
        bundle_identifier: None,
    }
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
    let mut config = manager.get().clone();
    // Pro themes fall back to the default theme unless Pro verifies right now.
    config.theme = enforce_theme(&config.theme);
    Ok(config)
}

#[tauri::command]
pub fn set_settings(state: State<AppState>, mut config: AppConfig) -> Result<(), String> {
    CustomDictionary::parse(&config.custom_dictionary)
        .map_err(|error| format!("Custom dictionary: {error}"))?;
    // Pro themes fall back to the default theme unless Pro verifies right now.
    config.theme = enforce_theme(&config.theme);

    let mut manager = state
        .config
        .lock()
        .map_err(|error| format!("lock poisoned: {error}"))?;
    let previous = manager.get().clone();
    // Retex consent and scan results are server-owned. The webview may not
    // manufacture vault grants or overwrite a concurrent refresh snapshot.
    config.config_version = previous.config_version;
    config.retex_dictionary = previous.retex_dictionary.clone();
    config.retex_vault_paths = previous.retex_vault_paths.clone();
    config.retex_vault_identities = previous.retex_vault_identities.clone();
    config.retex_last_refresh_at = previous.retex_last_refresh_at;
    config.retex_auto_refresh = previous.retex_auto_refresh;

    let models_dir = match config.models_directory.as_ref() {
        Some(directory) => directory.clone(),
        None => crate::state::data_dir(&state.app)?.join("models"),
    };
    std::fs::create_dir_all(&models_dir).map_err(|error| {
        format!(
            "could not use models directory {}: {error}",
            models_dir.display()
        )
    })?;

    let models_directory_changed = previous.models_directory != config.models_directory;
    let selected_model_changed = models_directory_changed
        || previous.selected_engine != config.selected_engine
        || previous.fluid_audio_model_version != config.fluid_audio_model_version
        || previous.selected_whisper_model_path != config.selected_whisper_model_path
        || previous.model_language != config.model_language;
    manager
        .set(config.clone())
        .map_err(|error| error.to_string())?;
    drop(manager);

    if models_directory_changed {
        *state
            .downloads
            .lock()
            .map_err(|error| format!("lock poisoned: {error}"))? =
            DownloadManager::with_models_dir(models_dir);
    }
    if selected_model_changed {
        state.warm_transcription_model();
    }
    state.emit_settings_changed(&config)?;
    Ok(())
}

#[tauri::command]
pub fn revoke_retex_vault(state: State<'_, AppState>, path: PathBuf) -> Result<(), String> {
    let mut manager = state
        .config
        .lock()
        .map_err(|error| format!("lock poisoned: {error}"))?;
    let mut updated = manager.get().clone();
    let Some(index) = updated
        .retex_vault_paths
        .iter()
        .position(|candidate| candidate == &path)
    else {
        return Err("That directory does not have Retex vocabulary permission.".to_string());
    };
    updated.retex_vault_paths.remove(index);
    if index < updated.retex_vault_identities.len() {
        updated.retex_vault_identities.remove(index);
    }
    updated.retex_dictionary.clear();
    updated.retex_last_refresh_at = None;
    if updated.retex_vault_paths.is_empty() {
        updated.retex_auto_refresh = false;
    }
    manager
        .set(updated.clone())
        .map_err(|error| error.to_string())?;
    drop(manager);
    if let Some(cancel) = state
        .retex_scan_cancel
        .lock()
        .map_err(|error| format!("lock poisoned: {error}"))?
        .take()
    {
        cancel.store(true, std::sync::atomic::Ordering::Relaxed);
    }
    state.emit_settings_changed(&updated)
}

#[tauri::command]
pub fn set_retex_auto_refresh(state: State<'_, AppState>, enabled: bool) -> Result<(), String> {
    let mut manager = state
        .config
        .lock()
        .map_err(|error| format!("lock poisoned: {error}"))?;
    let mut updated = manager.get().clone();
    updated.retex_auto_refresh = enabled && !updated.retex_vault_paths.is_empty();
    manager
        .set(updated.clone())
        .map_err(|error| error.to_string())?;
    drop(manager);
    state.emit_settings_changed(&updated)
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

/// Media containers/codecs the in-app decoder (Symphonia + libopus) accepts.
/// Extensions are matched case-insensitively against the dropped file name.
const IMPORTABLE_EXTENSIONS: &[&str] = &[
    "wav", "wave", "mp3", "m4a", "m4b", "mp4", "m4v", "mov", "aac", "flac", "ogg", "oga", "opus",
    "webm", "aif", "aiff", "caf", "alac",
];

#[tauri::command]
pub async fn import_file(state: State<'_, AppState>, path: String) -> Result<String, String> {
    let source = PathBuf::from(path.trim());
    if !source.is_absolute() {
        return Err("file import requires an absolute path".to_string());
    }
    let metadata = tokio::fs::metadata(&source)
        .await
        .map_err(|_| format!("file not found: {}", source.display()))?;
    if !metadata.is_file() {
        return Err("dropped item is not a file".to_string());
    }
    if metadata.len() == 0 {
        return Err("the dropped file is empty".to_string());
    }
    if metadata.len() > IMPORT_MAX_BYTES {
        return Err("the dropped file is larger than 2 GB".to_string());
    }
    let extension = source
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.to_ascii_lowercase())
        .unwrap_or_default();
    if !IMPORTABLE_EXTENSIONS.contains(&extension.as_str()) {
        return Err(format!(
            ".{extension} files are not supported; drop an audio or media file instead"
        ));
    }
    if state.session.lock().await.is_some() {
        return Err("stop dictation before importing a file".to_string());
    }
    if state.meeting_session.lock().await.is_some() {
        return Err("stop meeting mode before importing a file".to_string());
    }

    let id = Uuid::new_v4();
    let recordings_dir = state.recordings_dir()?;
    tokio::fs::create_dir_all(&recordings_dir)
        .await
        .map_err(|error| format!("could not create recordings directory: {error}"))?;
    let output_path = recordings_dir.join(format!("{id}.wav"));

    let decode_source = source.clone();
    let decode_dest = output_path.clone();
    let duration_ms =
        tokio::task::spawn_blocking(move || decode_media_file_to_wav(&decode_source, &decode_dest))
            .await
            .map_err(|error| format!("could not decode the dropped file: {error}"))?
            .map_err(|error| {
                let name = source
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("the dropped file");
                format!("could not import {name}: {error}")
            })?;

    let recording = AudioRecording {
        id: id.to_string(),
        output_path: output_path.clone(),
        start_time_ms: 0,
        duration_ms: Some(duration_ms),
    };
    if state.meeting_session.lock().await.is_some() {
        let _ = tokio::fs::remove_file(&output_path).await;
        return Err("meeting mode started before the import finished".to_string());
    }
    // Only one transcription runs at a time. Serialize dropped-file imports so
    // each file waits for the previous transcription to finish and is queued in
    // drop order instead of failing.
    let _import_guard = state.file_import.lock().await;
    let wait_started = std::time::Instant::now();
    while state.active_transcription.lock().await.is_some() {
        if wait_started.elapsed() > std::time::Duration::from_secs(600) {
            let _ = tokio::fs::remove_file(&output_path).await;
            return Err("timed out waiting for the active transcription to finish".to_string());
        }
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
    }
    let result = state.queue_managed_audio(recording, false, false).await;
    if result.is_err() && !recording_is_tracked(state.inner(), id) {
        let _ = tokio::fs::remove_file(&output_path).await;
    }
    result
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
pub fn get_audio_input_config(state: State<'_, AppState>) -> Result<AudioInputConfig, String> {
    let selected_device = state
        .config
        .lock()
        .map_err(|error| error.to_string())?
        .get()
        .audio_input_device_id
        .clone();
    Ok(AudioInputConfig {
        device_id: selected_device,
        ..AudioInputConfig::default()
    })
}

#[tauri::command]
pub async fn get_input_level(state: State<'_, AppState>) -> Result<f32, String> {
    Ok(state
        .audio
        .lock()
        .await
        .current_input_level()
        .clamp(0.0, 1.0))
}

/// Enables or disables the audio-only system-output meter used by reactive
/// media equalizers. Starting ScreenCaptureKit may block while it resolves
/// shareable content, so it runs off Tauri's IPC dispatch path.
#[cfg(target_os = "macos")]
#[tauri::command]
pub async fn set_system_audio_meter_enabled(enabled: bool) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || bridge::set_system_audio_meter_enabled(enabled))
        .await
        .map_err(|error| format!("system audio meter task failed: {error}"))?
}

#[cfg(not(target_os = "macos"))]
#[tauri::command]
pub async fn set_system_audio_meter_enabled(_enabled: bool) -> Result<(), String> {
    Err("system audio metering requires macOS".to_string())
}

#[cfg(target_os = "macos")]
#[tauri::command]
pub fn get_system_audio_level() -> f64 {
    bridge::system_audio_level()
}

#[cfg(not(target_os = "macos"))]
#[tauri::command]
pub fn get_system_audio_level() -> f64 {
    0.0
}

#[cfg(target_os = "macos")]
#[tauri::command]
pub fn get_system_audio_spectrum() -> Vec<f64> {
    bridge::system_audio_spectrum()
}

#[cfg(not(target_os = "macos"))]
#[tauri::command]
pub fn get_system_audio_spectrum() -> Vec<f64> {
    vec![0.0; 11]
}

/// Glanceable media-panel state for the frontend wire contract.
/// Optional fields are `null` when unknown or unsupported; `volume` is 0..1.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaState {
    pub active: bool,
    pub app_name: Option<String>,
    pub bundle_id: Option<String>,
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub artwork_data_url: Option<String>,
    pub elapsed_seconds: Option<f64>,
    pub duration_seconds: Option<f64>,
    pub is_playing: Option<bool>,
    pub volume: Option<f64>,
    pub muted: Option<bool>,
    pub volume_available: bool,
    pub transport_available: bool,
    pub previous_available: bool,
    pub next_available: bool,
}

impl MediaState {
    fn inactive() -> Self {
        Self {
            active: false,
            app_name: None,
            bundle_id: None,
            title: None,
            artist: None,
            album: None,
            artwork_data_url: None,
            elapsed_seconds: None,
            duration_seconds: None,
            is_playing: None,
            volume: None,
            muted: None,
            volume_available: false,
            transport_available: false,
            previous_available: false,
            next_available: false,
        }
    }
}

/// System-session media state from the shared adapter-first metadata crate,
/// plus the existing bridge's CoreAudio source and default-output controls.
#[cfg(target_os = "macos")]
fn collect_media_state() -> MediaState {
    let now_playing =
        ultra_media_remote::now_playing_fetch(Duration::from_millis(600)).filter(|item| {
            item.title.is_some()
                || item.artist.is_some()
                || item.album.is_some()
                || item.artwork_data_url.is_some()
                || item.duration_seconds.is_some()
                || item.is_playing.is_some()
        });
    let audio_source = bridge::active_audio_source()
        .filter(|source| source.app_name.is_some() || source.bundle_id.is_some());
    if now_playing.is_none() && audio_source.is_none() {
        return MediaState::inactive();
    }
    // The shared crate discovers the canonical system session and reports
    // secondary commands only when MediaRemote proves they are enabled.
    let transport_capabilities = ultra_media_remote::transport_capabilities();
    let app_name = now_playing
        .as_ref()
        .and_then(|item| item.app_name.clone())
        .or_else(|| {
            audio_source
                .as_ref()
                .and_then(|source| source.app_name.clone())
        });
    let bundle_id = now_playing
        .as_ref()
        .and_then(|item| item.bundle_id.clone())
        .or_else(|| {
            audio_source
                .as_ref()
                .and_then(|source| source.bundle_id.clone())
        });
    let volume_state = bridge::output_volume_state();
    MediaState {
        active: true,
        app_name,
        bundle_id,
        title: now_playing.as_ref().and_then(|item| item.title.clone()),
        artist: now_playing.as_ref().and_then(|item| item.artist.clone()),
        album: now_playing.as_ref().and_then(|item| item.album.clone()),
        artwork_data_url: now_playing
            .as_ref()
            .and_then(|item| item.artwork_data_url.clone()),
        elapsed_seconds: now_playing.as_ref().and_then(|item| item.elapsed_seconds),
        duration_seconds: now_playing.as_ref().and_then(|item| item.duration_seconds),
        is_playing: audio_source
            .is_some()
            .then_some(true)
            .or_else(|| now_playing.as_ref().and_then(|item| item.is_playing)),
        volume: volume_state.volume,
        muted: volume_state.muted,
        volume_available: volume_state.volume.is_some(),
        transport_available: transport_capabilities.play_pause,
        previous_available: transport_capabilities.previous,
        next_available: transport_capabilities.next,
    }
}

#[cfg(target_os = "macos")]
#[tauri::command]
pub async fn get_media_state() -> MediaState {
    tauri::async_runtime::spawn_blocking(collect_media_state)
        .await
        .unwrap_or_else(|_| MediaState::inactive())
}

#[cfg(not(target_os = "macos"))]
#[tauri::command]
pub async fn get_media_state() -> MediaState {
    MediaState::inactive()
}

#[cfg(target_os = "macos")]
#[tauri::command]
pub fn set_system_volume(volume: f64) -> Result<(), String> {
    bridge::set_system_volume(volume)
}

#[cfg(not(target_os = "macos"))]
#[tauri::command]
pub fn set_system_volume(_volume: f64) -> Result<(), String> {
    Err("system volume requires macOS".to_string())
}

#[cfg(target_os = "macos")]
#[tauri::command]
pub fn set_system_muted(muted: bool) -> Result<(), String> {
    bridge::set_system_muted(muted)
}

#[cfg(not(target_os = "macos"))]
#[tauri::command]
pub fn set_system_muted(_muted: bool) -> Result<(), String> {
    Err("system mute requires macOS".to_string())
}

#[cfg(target_os = "macos")]
fn parse_media_transport_command(command: &str) -> Result<TransportCommand, String> {
    match command {
        "play_pause" => Ok(TransportCommand::PlayPause),
        "previous" => Ok(TransportCommand::Previous),
        "next" => Ok(TransportCommand::Next),
        _ => Err(format!("unknown transport command: {command}")),
    }
}

/// Sends `play_pause`, `previous`, or `next` to the canonical system
/// now-playing session. The adapter/direct probes may block, so the command
/// runs off Tauri's IPC dispatch path.
#[cfg(target_os = "macos")]
fn send_media_transport(command: String) -> Result<(), String> {
    let command = parse_media_transport_command(&command)?;
    if ultra_media_remote::transport_send(command) {
        Ok(())
    } else {
        Err("system media session is unavailable".to_string())
    }
}

#[cfg(target_os = "macos")]
#[tauri::command]
pub async fn media_transport(command: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || send_media_transport(command))
        .await
        .map_err(|error| format!("media transport task failed: {error}"))?
}

#[cfg(not(target_os = "macos"))]
#[tauri::command]
pub async fn media_transport(_command: String) -> Result<(), String> {
    Err("media transport requires macOS".to_string())
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
        screen_recording_permission_error, validate_remote_url, MediaState, PermissionKind,
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
    #[test]
    fn media_state_serializes_new_metadata_in_camel_case() {
        let mut state = MediaState::inactive();
        state.active = true;
        state.app_name = Some("Music".to_string());
        state.bundle_id = Some("com.apple.Music".to_string());
        state.album = Some("Album".to_string());
        state.artwork_data_url = Some("data:image/jpeg;base64,QUJD".to_string());
        state.elapsed_seconds = Some(4.5);
        state.duration_seconds = Some(120.0);
        state.previous_available = true;
        state.next_available = false;
        let value = serde_json::to_value(state).expect("media state should serialize");
        assert_eq!(value["elapsedSeconds"], 4.5);
        assert_eq!(value["durationSeconds"], 120.0);
        assert_eq!(value["artworkDataUrl"], "data:image/jpeg;base64,QUJD");
        assert_eq!(value["previousAvailable"], true);
        assert_eq!(value["nextAvailable"], false);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn media_transport_strings_map_to_shared_commands() {
        assert_eq!(
            super::parse_media_transport_command("play_pause").unwrap(),
            ultra_media_remote::TransportCommand::PlayPause
        );
        assert_eq!(
            super::parse_media_transport_command("previous").unwrap(),
            ultra_media_remote::TransportCommand::Previous
        );
        assert_eq!(
            super::parse_media_transport_command("next").unwrap(),
            ultra_media_remote::TransportCommand::Next
        );
        assert_eq!(
            super::parse_media_transport_command("stop").unwrap_err(),
            "unknown transport command: stop"
        );
    }
    #[cfg(target_os = "macos")]
    #[test]
    fn shared_media_crate_preserves_adapter_metadata_artwork_and_cache() {
        let directory = tempfile::tempdir().expect("adapter fixture directory");
        let framework = directory.path().join("MediaRemoteAdapter.framework");
        std::fs::create_dir(&framework).expect("adapter fixture framework");
        let script = directory.path().join("mediaremote-adapter.pl");
        std::fs::write(
            &script,
            r#"print '{"processIdentifier":42,"bundleIdentifier":"com.google.Chrome.app.youtube","title":"Track","artist":"Artist","album":"Album","elapsedTimeNow":12.5,"duration":180,"playing":true,"artworkMimeType":"image/jpeg","artworkData":"YWJj"}';"#,
        )
        .expect("adapter fixture script");

        let previous_directory = std::env::var_os("ULTRA_MEDIA_REMOTE_ADAPTER_DIR");
        std::env::set_var("ULTRA_MEDIA_REMOTE_ADAPTER_DIR", directory.path());
        let first = ultra_media_remote::now_playing_fetch(std::time::Duration::from_millis(50));
        std::fs::write(&script, r#"print '{"title":"Changed"}';"#)
            .expect("changed adapter fixture");
        let cached = ultra_media_remote::now_playing_fetch(std::time::Duration::from_millis(50));
        if let Some(previous_directory) = previous_directory {
            std::env::set_var("ULTRA_MEDIA_REMOTE_ADAPTER_DIR", previous_directory);
        } else {
            std::env::remove_var("ULTRA_MEDIA_REMOTE_ADAPTER_DIR");
        }

        let first = first.expect("shared crate should parse the adapter fixture");
        assert_eq!(first.pid, Some(42));
        assert_eq!(
            first.bundle_id.as_deref(),
            Some("com.google.Chrome.app.youtube")
        );
        assert_eq!(first.title.as_deref(), Some("Track"));
        assert_eq!(first.artist.as_deref(), Some("Artist"));
        assert_eq!(first.album.as_deref(), Some("Album"));
        assert_eq!(first.elapsed_seconds, Some(12.5));
        assert_eq!(first.duration_seconds, Some(180.0));
        assert_eq!(first.is_playing, Some(true));
        assert_eq!(
            first.artwork_data_url.as_deref(),
            Some("data:image/jpeg;base64,YWJj")
        );
        assert_eq!(cached.as_ref(), Some(&first));
    }
}

#[cfg(target_os = "macos")]
fn theme_mode(theme: &str) -> tauri::Theme {
    match theme {
        "frutiger-aero" | "nord-frost" | "winamp-mmd3" | "winamp-hifi" | "crystal" => {
            tauri::Theme::Light
        }
        _ => tauri::Theme::Dark,
    }
}

/// Re-applies the native window material for the given UI theme.
#[tauri::command]
pub fn set_theme_material(app: tauri::AppHandle, theme: String) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        // NSVisualEffectView must be touched on the main thread; Tauri commands
        // run on a worker, so hop over and surface failures to the caller.
        let (tx, rx) = std::sync::mpsc::channel();
        app.clone()
            .run_on_main_thread(move || {
                use tauri::Manager;
                let result = match app.get_webview_window("main") {
                    Some(window) => window
                        .set_theme(Some(theme_mode(&theme)))
                        .map_err(|error| error.to_string())
                        .and_then(|_| {
                            apply_theme_material(&window, &theme).map_err(|error| error.to_string())
                        }),
                    None => Err("main window not found".to_string()),
                };
                let _ = tx.send(result);
            })
            .map_err(|e| e.to_string())?;
        rx.recv().map_err(|e| e.to_string())??;
    }
    #[cfg(not(target_os = "macos"))]
    let _ = (app, theme);
    Ok(())
}

/// Only themes intentionally designed around translucent native glass receive
/// an NSVisualEffectView. Opaque/CSS-driven themes must clear vibrancy: placing
/// a native material over WKWebView can cover the entire client area on macOS.
#[cfg(target_os = "macos")]
fn theme_material(theme: &str) -> Option<window_vibrancy::NSVisualEffectMaterial> {
    use window_vibrancy::NSVisualEffectMaterial;
    match theme {
        "frutiger-aero" | "nord-frost" | "crystal" => {
            Some(NSVisualEffectMaterial::UnderWindowBackground)
        }
        "frutiger-dark" => Some(NSVisualEffectMaterial::FullScreenUI),
        _ => None,
    }
}

#[cfg(target_os = "macos")]
static THEME_MATERIAL_ACTIVE: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

#[cfg(target_os = "macos")]
pub fn apply_theme_material(
    window: &tauri::WebviewWindow,
    theme: &str,
) -> Result<(), window_vibrancy::Error> {
    use std::sync::atomic::Ordering;

    let was_active = THEME_MATERIAL_ACTIVE.load(Ordering::Acquire);
    match theme_material(theme) {
        Some(material) => {
            if was_active {
                window_vibrancy::clear_vibrancy(window)?;
                THEME_MATERIAL_ACTIVE.store(false, Ordering::Release);
            }
            window_vibrancy::apply_vibrancy(
                window,
                material,
                Some(window_vibrancy::NSVisualEffectState::Active),
                Some(16.0),
            )?;
            THEME_MATERIAL_ACTIVE.store(true, Ordering::Release);
            Ok(())
        }
        None if was_active => {
            window_vibrancy::clear_vibrancy(window)?;
            THEME_MATERIAL_ACTIVE.store(false, Ordering::Release);
            Ok(())
        }
        None => Ok(()),
    }
}
