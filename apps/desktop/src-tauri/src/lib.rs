use std::sync::atomic::{AtomicBool, Ordering};

use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::Emitter;
use tauri::{AppHandle, Manager, Wry};

mod commands;
mod events;
mod state;
mod telemetry;
mod update;
pub mod voice_ipc;

use commands::*;
use state::AppState;

static SHUTTING_DOWN: AtomicBool = AtomicBool::new(false);

pub use commands::{
    bridge_version, check_for_update, copy_to_clipboard, delete_recording, export_recording,
    get_app_info, get_app_status, get_app_telemetry_status, get_audio_devices,
    get_audio_input_config, get_caret_position, get_download_progress, get_downloads,
    get_media_state, get_model_catalog, get_pending_meeting_detection, get_permission_status,
    get_recording, get_settings, get_shortcut_settings, get_update_preferences, hide_indicator,
    import_file, import_url, install_update, is_model_downloaded, list_recordings, media_transport,
    open_permission_settings, paste_text, prepare_model, record_app_telemetry_usage,
    request_permission, respond_meeting_detection, retry_transcription, search_recordings,
    set_app_telemetry_enabled, set_settings, set_shortcut_settings, set_system_muted,
    set_system_volume, set_update_preferences, show_indicator, start_download,
    start_key_combination_hotkey, start_meeting, start_modifier_hotkey, start_recording,
    stop_key_combination_hotkey, stop_meeting, stop_modifier_hotkey, stop_recording,
    transcribe_file, update_recording, AppInfo, AppStatus, BridgeVersion, CaretPosition,
    TranscriptionResult,
};

fn build_tray_menu(app: &AppHandle) -> Result<Menu<Wry>, Box<dyn std::error::Error>> {
    let open_i = MenuItem::with_id(app, "open", "Open UltraVox", true, None::<&str>)?;
    let settings_i = MenuItem::with_id(app, "settings", "Settings...", true, None::<&str>)?;
    let separator = PredefinedMenuItem::separator(app)?;
    let quit_i = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    Menu::with_items(app, &[&open_i, &settings_i, &separator, &quit_i]).map_err(Into::into)
}

fn show_main_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.set_focus();
    }
}
pub(crate) fn show_meeting_reminder(app: &AppHandle) -> Result<(), String> {
    let window = app
        .get_webview_window("meeting-reminder")
        .ok_or_else(|| "meeting reminder window is unavailable".to_string())?;
    window.show().map_err(|error| error.to_string())?;
    let _ = window.unminimize();
    window.set_focus().map_err(|error| error.to_string())
}

pub(crate) fn close_meeting_reminder(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("meeting-reminder") {
        let _ = window.hide();
    }
}

fn request_shutdown(app: &AppHandle) {
    if SHUTTING_DOWN.swap(true, Ordering::SeqCst) {
        return;
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let state = app.state::<AppState>();
        commands::discard_active_meeting(state.inner()).await;
        voice_ipc::cleanup_socket();
        app.exit(0);
    });
}

pub fn setup_tray(app: &AppHandle) -> Result<(), Box<dyn std::error::Error>> {
    let menu = build_tray_menu(app)?;

    let _tray = TrayIconBuilder::new()
        .tooltip("UltraVox")
        .icon_as_template(true)
        .menu(&menu)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "open" => show_main_window(app),
            "settings" => {
                show_main_window(app);
                let _ = app.emit("navigate-to", "settings");
            }
            "quit" => request_shutdown(app),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                let app = tray.app_handle();
                show_main_window(app);
            }
        })
        .build(app)?;

    Ok(())
}

#[cfg(target_os = "macos")]
mod hotkey {
    use std::ffi::CStr;
    use std::os::raw::{c_char, c_int};
    use std::sync::OnceLock;
    use std::time::Duration;

    use tauri::{AppHandle, Emitter, Listener, Manager};
    use ultravox_macos_bridge as bridge;

    use crate::commands;
    use crate::events::{
        SettingsChangedPayload, ShortcutTriggeredPayload, SETTINGS_CHANGED, SHORTCUT_TRIGGERED,
    };
    use crate::state::AppState;

    static APP_HANDLE: OnceLock<AppHandle> = OnceLock::new();

    extern "C" fn on_hotkey_event(event: c_int, combo: *const c_char) {
        let Some(app) = APP_HANDLE.get() else {
            return;
        };
        let combo = if combo.is_null() {
            String::new()
        } else {
            unsafe { CStr::from_ptr(combo).to_string_lossy().into_owned() }
        };
        let action = match event {
            0 => "down",
            1 => "up",
            _ => "unknown",
        };
        let _ = app.emit(
            SHORTCUT_TRIGGERED,
            ShortcutTriggeredPayload::new(format!("{combo}:{action}")),
        );
    }

    extern "C" fn on_meeting_hotkey_event(event: c_int, _combo: *const c_char) {
        if event != 0 {
            return;
        }
        let Some(app) = APP_HANDLE.get().cloned() else {
            return;
        };
        tauri::async_runtime::spawn(async move {
            let state = app.state::<AppState>();
            let result = if state.meeting_session.lock().await.is_some() {
                commands::stop_meeting(state).await.map(|_| String::new())
            } else {
                commands::start_meeting(state).await
            };
            if let Err(error) = result {
                eprintln!("meeting shortcut failed: {error}");
            }
        });
    }

    extern "C" fn on_meeting_capture_failure(error: *const c_char) {
        let message = unsafe {
            if error.is_null() {
                "meeting capture failed".to_string()
            } else {
                CStr::from_ptr(error).to_string_lossy().into_owned()
            }
        };
        let Some(app) = APP_HANDLE.get().cloned() else {
            eprintln!("meeting capture failed before app setup: {message}");
            return;
        };
        tauri::async_runtime::spawn(async move {
            let state = app.state::<AppState>();
            let _transition = state.activity_transition.lock().await;
            let session = {
                let mut meeting = state.meeting_session.lock().await;
                if meeting.as_ref().is_some_and(|session| session.stopping) {
                    None
                } else {
                    meeting.take()
                }
            };
            if let Some(session) = session {
                let stopped = tokio::task::spawn_blocking(bridge::stop_meeting_capture).await;
                if let Ok(Ok(audio_path)) = stopped {
                    let _ = tokio::fs::remove_file(audio_path).await;
                }
                let _ = tokio::fs::remove_file(&session.output_path).await;
                let _ = tokio::fs::remove_file(session.output_path.with_extension("m4a")).await;
                let _ = state.emit_meeting_state_changed(false);
            }
            eprintln!("meeting capture failed: {message}");
        });
    }

    fn register_key_combination_hotkey(key_combination: &str, hold_to_record: bool) {
        if bridge::start_key_combination_hotkey(key_combination, hold_to_record) <= 0 {
            eprintln!("failed to register global shortcut {key_combination}");
        }
    }

    fn register_meeting_hotkey(key_combination: &str) {
        bridge::stop_meeting_hotkey();
        if bridge::start_meeting_hotkey(key_combination) <= 0 {
            eprintln!("failed to register meeting shortcut {key_combination}");
        }
    }

    fn register_modifier_only_hotkey(modifier: &str) {
        bridge::stop_modifier_hotkey();
        if modifier != "none" && bridge::start_modifier_hotkey(modifier) <= 0 {
            eprintln!("failed to register modifier-only shortcut {modifier}");
        }
    }

    fn register_configured_hotkeys(app: &AppHandle) {
        let state = app.state::<AppState>();
        let cfg = match state.config.lock() {
            Ok(config) => config.get().clone(),
            Err(error) => {
                eprintln!("failed to read shortcut configuration: {error}");
                return;
            }
        };
        register_key_combination_hotkey(&cfg.key_combination, cfg.hold_to_record);
        register_modifier_only_hotkey(&cfg.modifier_only_hotkey);
        register_meeting_hotkey(&cfg.meeting_key_combination);
    }

    pub fn setup(app: &AppHandle) {
        if !bridge::is_accessibility_trusted(false) {
            let _ = bridge::is_accessibility_trusted(true);
            eprintln!(
                "UltraVox needs Accessibility access to place its indicator and insert transcriptions"
            );
        }

        let app_handle = app.clone();
        if APP_HANDLE.set(app_handle).is_err() {
            eprintln!("hotkey app handle already set");
        }

        bridge::set_key_combination_callback(on_hotkey_event);
        bridge::set_meeting_hotkey_callback(on_meeting_hotkey_event);
        bridge::set_meeting_capture_failure_callback(on_meeting_capture_failure);

        register_configured_hotkeys(app);

        let permission_app = app.clone();
        tauri::async_runtime::spawn(async move {
            let mut was_trusted = false;
            loop {
                tokio::time::sleep(Duration::from_secs(2)).await;
                let is_trusted = bridge::is_accessibility_trusted(false);
                if is_trusted && !was_trusted {
                    register_configured_hotkeys(&permission_app);
                }
                was_trusted = is_trusted;
            }
        });

        let app_handle = app.clone();
        app.listen(SHORTCUT_TRIGGERED, move |event: tauri::Event| {
            let payload = match serde_json::from_str::<ShortcutTriggeredPayload>(event.payload()) {
                Ok(payload) => payload,
                Err(_) => return,
            };
            let app_handle = app_handle.clone();
            tauri::async_runtime::spawn(async move {
                let state = app_handle.state::<AppState>();
                if let Err(e) = handle_shortcut_event(state, payload).await {
                    eprintln!("shortcut event handling failed: {e}");
                }
            });
        });

        // Restart the key-combination tap whenever shortcut settings change so
        // the global hotkey always reflects the current config.
        app.listen(SETTINGS_CHANGED, move |event: tauri::Event| {
            let payload = match serde_json::from_str::<SettingsChangedPayload>(event.payload()) {
                Ok(payload) => payload,
                Err(_) => return,
            };
            register_key_combination_hotkey(
                &payload.config.key_combination,
                payload.config.hold_to_record,
            );
            register_meeting_hotkey(&payload.config.meeting_key_combination);
            register_modifier_only_hotkey(&payload.config.modifier_only_hotkey);
        });
    }

    async fn handle_shortcut_event(
        state: tauri::State<'_, AppState>,
        payload: ShortcutTriggeredPayload,
    ) -> Result<(), String> {
        let (trigger, action) = payload.shortcut.rsplit_once(':').unwrap_or(("", "unknown"));
        let hold_to_record = {
            let cfg = state.config.lock().map_err(|e| e.to_string())?;
            cfg.get().hold_to_record || cfg.get().modifier_only_hotkey.eq_ignore_ascii_case(trigger)
        };
        let recording = state.session.lock().await.is_some();

        match action {
            "down" => {
                if recording {
                    stop_shortcut_recording(state).await?;
                } else {
                    let (x, y, has_target) = bridge::capture_insertion_target();
                    if !has_target {
                        eprintln!(
                            "no focused insertion target; indicator will use the mouse position"
                        );
                    }
                    bridge::show_indicator(x, y);
                    if let Err(error) = state.begin_recording().await {
                        bridge::hide_indicator();
                        bridge::clear_insertion_target();
                        return Err(error);
                    }
                }
            }
            "up" => {
                if recording && hold_to_record {
                    stop_shortcut_recording(state).await?;
                }
            }
            _ => {}
        }

        Ok(())
    }

    async fn stop_shortcut_recording(state: tauri::State<'_, AppState>) -> Result<(), String> {
        bridge::set_indicator_state("transcribing");
        if let Err(error) = commands::stop_recording(state).await {
            bridge::set_indicator_state("failed");
            bridge::clear_insertion_target();
            bridge::hide_indicator();
            return Err(error);
        }
        Ok(())
    }
}

#[cfg(not(target_os = "macos"))]
mod hotkey {
    use tauri::AppHandle;

    pub fn setup(_app: &AppHandle) {}
}

fn setup_hotkey(app: &AppHandle) {
    hotkey::setup(app);
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.show();
                let _ = window.unminimize();
                let _ = window.set_focus();
            }
        }))
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            get_app_info,
            get_app_status,
            bridge_version,
            get_permission_status,
            request_permission,
            open_permission_settings,
            get_update_preferences,
            set_update_preferences,
            check_for_update,
            install_update,
            get_app_telemetry_status,
            set_app_telemetry_enabled,
            record_app_telemetry_usage,
            get_caret_position,
            paste_text,
            copy_to_clipboard,
            start_modifier_hotkey,
            stop_modifier_hotkey,
            start_key_combination_hotkey,
            stop_key_combination_hotkey,
            show_indicator,
            hide_indicator,
            transcribe_file,
            prepare_model,
            get_settings,
            set_settings,
            get_model_catalog,
            get_download_progress,
            get_downloads,
            start_download,
            is_model_downloaded,
            get_model_progress,
            cancel_download,
            list_recordings,
            search_recordings,
            get_recording,
            update_recording,
            delete_recording,
            delete_all_recordings,
            start_recording,
            import_url,
            import_file,
            stop_recording,
            start_meeting,
            stop_meeting,
            respond_meeting_detection,
            get_pending_meeting_detection,
            get_transcription_status,
            retry_transcription,
            get_shortcut_settings,
            set_shortcut_settings,
            get_audio_devices,
            get_audio_input_config,
            export_recording,
            get_media_state,
            set_system_volume,
            set_system_muted,
            media_transport,
            set_theme_material,
        ])
        .setup(|app| {
            #[cfg(target_os = "macos")]
            let _ = app.set_activation_policy(tauri::ActivationPolicy::Regular);

            #[cfg(target_os = "macos")]
            if let Some(window) = app.get_webview_window("main") {
                let _ = apply_theme_material(&window, "midnight");
            }

            let state = AppState::new(app.handle().clone()).map_err(|e| {
                Box::<dyn std::error::Error>::from(format!("failed to initialize app state: {e}"))
            })?;
            app.manage(state);
            let telemetry_app = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                {
                    let state = telemetry_app.state::<AppState>();
                    let _ = state.telemetry.launch().await;
                    let _ = state.telemetry.heartbeat().await;
                }
                loop {
                    tokio::time::sleep(std::time::Duration::from_secs(6 * 60 * 60)).await;
                    let state = telemetry_app.state::<AppState>();
                    let _ = state.telemetry.heartbeat().await;
                }
            });
            app.state::<AppState>().warm_transcription_model();
            voice_ipc::start(app.handle().clone());

            setup_hotkey(app.handle());
            setup_tray(app.handle())?;
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building UltraVox application");

    app.run(|app, event| match event {
        tauri::RunEvent::ExitRequested { api, .. } if !SHUTTING_DOWN.load(Ordering::SeqCst) => {
            api.prevent_exit();
            request_shutdown(app);
        }
        tauri::RunEvent::Exit => voice_ipc::cleanup_socket(),
        _ => {}
    });
}
