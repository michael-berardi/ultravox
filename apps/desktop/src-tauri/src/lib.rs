use std::sync::atomic::{AtomicBool, Ordering};

use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::Emitter;
use tauri::{AppHandle, Manager, Wry};

mod commands;
mod events;
mod state;
mod update;

use commands::*;
use state::AppState;

static SHUTTING_DOWN: AtomicBool = AtomicBool::new(false);

pub use commands::{
    bridge_version, cancel_download, check_for_update, copy_to_clipboard, delete_all_recordings,
    delete_recording, dictionary_apply, dictionary_skill_info, export_recording, get_app_info,
    get_app_status, get_audio_devices, get_audio_input_config, get_caret_position,
    get_download_progress, get_downloads, get_input_level, get_model_catalog, get_model_progress,
    get_permission_status, get_recording, get_settings, get_shortcut_settings,
    get_transcription_status, get_update_preferences, hide_indicator, import_file, import_url,
    install_update, is_model_downloaded, list_recordings, open_permission_settings, paste_text,
    prepare_model, report_frontend_error, request_permission, retry_transcription,
    reveal_dictionary_skill, search_recordings, set_settings, set_shortcut_settings,
    set_theme_material, show_indicator, start_download, start_key_combination_hotkey,
    start_modifier_hotkey, start_recording, stop_key_combination_hotkey, stop_modifier_hotkey,
    stop_recording, transcribe_file, update_recording, AppInfo, AppStatus, BridgeVersion,
    CaretPosition, DictionaryApplyResult, TranscriptionResult,
};

fn build_tray_menu(app: &AppHandle) -> Result<Menu<Wry>, Box<dyn std::error::Error>> {
    let open = MenuItem::with_id(app, "open", "Open UltraVox Light", true, None::<&str>)?;
    let settings = MenuItem::with_id(app, "settings", "Settings...", true, None::<&str>)?;
    let separator = PredefinedMenuItem::separator(app)?;
    let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    Menu::with_items(app, &[&open, &settings, &separator, &quit]).map_err(Into::into)
}

fn show_main_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

fn request_shutdown(app: &AppHandle) {
    if SHUTTING_DOWN.swap(true, Ordering::SeqCst) {
        return;
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let _ = app.state::<AppState>().finish_recording(false).await;
        app.exit(0);
    });
}

pub fn setup_tray(app: &AppHandle) -> Result<(), Box<dyn std::error::Error>> {
    let menu = build_tray_menu(app)?;
    TrayIconBuilder::new()
        .tooltip("UltraVox Light")
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
                show_main_window(tray.app_handle());
            }
        })
        .build(app)?;
    Ok(())
}

#[cfg(target_os = "macos")]
mod hotkey {
    use crate::commands;
    use crate::events::{
        SettingsChangedPayload, ShortcutTriggeredPayload, SETTINGS_CHANGED, SHORTCUT_TRIGGERED,
    };
    use crate::state::AppState;
    use std::ffi::CStr;
    use std::os::raw::{c_char, c_int};
    use std::sync::OnceLock;
    use std::time::Duration;
    use tauri::{AppHandle, Emitter, Listener, Manager};
    use ultravox_macos_bridge as bridge;
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
    fn register_key(combo: &str, hold: bool) {
        if bridge::start_key_combination_hotkey(combo, hold) <= 0 {
            eprintln!("failed to register global shortcut {combo}");
        }
    }
    fn register_modifier(modifier: &str) {
        bridge::stop_modifier_hotkey();
        if modifier != "none" && bridge::start_modifier_hotkey(modifier) <= 0 {
            eprintln!("failed to register modifier-only shortcut {modifier}");
        }
    }
    fn register(app: &AppHandle) {
        let state = app.state::<AppState>();
        let Ok(config) = state.config.lock().map(|config| config.get().clone()) else {
            return;
        };
        register_key(&config.key_combination, config.hold_to_record);
        register_modifier(&config.modifier_only_hotkey);
    }
    pub fn setup(app: &AppHandle) {
        if !bridge::is_accessibility_trusted(false) {
            eprintln!("UltraVox Light needs Accessibility access for insertion");
        }
        let _ = APP_HANDLE.set(app.clone());
        bridge::set_key_combination_callback(on_hotkey_event);
        register(app);
        let app_for_permission = app.clone();
        tauri::async_runtime::spawn(async move {
            let mut was_trusted = false;
            loop {
                tokio::time::sleep(Duration::from_secs(2)).await;
                let trusted = bridge::is_accessibility_trusted(false);
                if trusted && !was_trusted {
                    register(&app_for_permission);
                }
                was_trusted = trusted;
            }
        });
        let app_for_events = app.clone();
        app.listen(SHORTCUT_TRIGGERED, move |event: tauri::Event| {
            let Ok(payload) = serde_json::from_str::<ShortcutTriggeredPayload>(event.payload())
            else {
                return;
            };
            let app = app_for_events.clone();
            tauri::async_runtime::spawn(async move {
                if let Err(error) = handle_shortcut_event(app.state::<AppState>(), payload).await {
                    eprintln!("shortcut handling failed: {error}");
                }
            });
        });
        app.listen(SETTINGS_CHANGED, move |event: tauri::Event| {
            if let Ok(payload) = serde_json::from_str::<SettingsChangedPayload>(event.payload()) {
                register_key(
                    &payload.config.key_combination,
                    payload.config.hold_to_record,
                );
                register_modifier(&payload.config.modifier_only_hotkey);
            }
        });
    }
    async fn handle_shortcut_event(
        state: tauri::State<'_, AppState>,
        payload: ShortcutTriggeredPayload,
    ) -> Result<(), String> {
        let (trigger, action) = payload.shortcut.rsplit_once(':').unwrap_or(("", "unknown"));
        let hold = {
            let config = state.config.lock().map_err(|e| e.to_string())?;
            config.get().hold_to_record
                || config
                    .get()
                    .modifier_only_hotkey
                    .eq_ignore_ascii_case(trigger)
        };
        let recording = state.session.lock().await.is_some();
        match action {
            "down" if recording => stop_shortcut_recording(state).await?,
            "down" => {
                let (x, y, _) = bridge::capture_insertion_target();
                bridge::show_indicator(x, y);
                if let Err(error) = state.begin_recording().await {
                    bridge::hide_indicator();
                    bridge::clear_insertion_target();
                    return Err(error);
                }
            }
            "up" if recording && hold => stop_shortcut_recording(state).await?,
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
        .plugin(tauri_plugin_single_instance::init(|app, _, _| {
            show_main_window(app)
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
            get_caret_position,
            copy_to_clipboard,
            dictionary_skill_info,
            reveal_dictionary_skill,
            set_theme_material,
            stop_modifier_hotkey,
            start_key_combination_hotkey,
            stop_key_combination_hotkey,
            show_indicator,
            hide_indicator,
            transcribe_file,
            dictionary_apply,
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
            get_transcription_status,
            retry_transcription,
            get_shortcut_settings,
            set_shortcut_settings,
            get_audio_devices,
            get_audio_input_config,
            get_input_level,
            export_recording
        ])
        .setup(|app| {
            let state = AppState::new(app.handle().clone()).map_err(|e| {
                Box::<dyn std::error::Error>::from(format!("failed to initialize app state: {e}"))
            })?;
            app.manage(state);
            app.state::<AppState>().warm_transcription_model();
            setup_hotkey(app.handle());
            setup_tray(app.handle())?;
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building UltraVox Light application");
    app.run(|app, event| {
        if let tauri::RunEvent::ExitRequested { api, .. } = event {
            if !SHUTTING_DOWN.load(Ordering::SeqCst) {
                api.prevent_exit();
                request_shutdown(app);
            }
        }
    });
}

#[cfg(test)]
mod dictionary_skill_tests {
    /// The bundled copy inside `src-tauri/resources` must always match the
    /// canonical skill at the repository root so shipped installs never
    /// diverge from the documented skill.
    #[test]
    fn bundled_dictionary_skill_matches_canonical_skill() {
        let canonical = include_str!("../../../../skills/dictionary-review/SKILL.md");
        let bundled = include_str!("../resources/skills/dictionary-review/SKILL.md");
        assert_eq!(canonical, bundled);
        assert!(canonical.contains("# UltraVox dictionary review"));
        assert!(canonical.len() > 4_000);
    }
}
