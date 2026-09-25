//! Always-locked Pro surface for the open-source build.
//!
//! The public repository builds UltraVox without the closed Pro module; this
//! stub exposes the same functions as `src/pro/` so the frontend contract is
//! identical in both flavors. Every entry point reports Pro as unavailable and
//! every locked command returns an error beginning `pro-locked:` (see
//! docs/pro-licensing.md). Pro is never unlockable in this build.
//!
//! Commands whose official-build result types live inside the Pro module return
//! `serde_json::Value` here: the Ok values are never produced, so only the
//! shared error vocabulary crosses IPC. Parameter names match the official
//! build exactly; they stay unused because nothing runs past the lock.

#![allow(unused_variables)]

use tauri::AppHandle;

use crate::events::{ProState, ProStatus};
use crate::state::AppState;

/// Build flavor compiled into this binary.
pub const APP_BUILD: &str = "open-source";

/// Entry guard for every Pro command: Pro is never available in this build.
#[inline(always)]
pub fn verify_unlocked() -> Result<(), String> {
    Err("pro-locked: UltraVox Pro is not included in this build.".to_string())
}

/// `get_pro_status` IPC command (docs/pro-licensing.md).
#[tauri::command]
pub fn get_pro_status() -> ProStatus {
    ProStatus {
        available: false,
        unlocked: false,
        state: ProState::Unavailable,
        trial_until: None,
        updates_until: None,
        plan: None,
        label: None,
        error: None,
    }
}

/// The open-source build has a constant status, so nothing ever changes.
pub fn emit_pro_status_changed(app: &AppHandle) {
    let _ = app;
}

/// The open-source build never contacts the license service.
pub fn start_entitlement_refresh(app: AppHandle) {
    let _ = app;
}

/// Every stub entry point funnels through the same always-locked guard.
fn locked<T>() -> Result<T, String> {
    verify_unlocked()?;
    unreachable!("the open-source stub never unlocks Pro")
}

pub mod distribution {
    //! License client: unavailable in the open-source build.

    use super::{locked, AppHandle};

    #[tauri::command(async)]
    pub async fn distribution_access_status(app: AppHandle) -> Result<serde_json::Value, String> {
        locked()
    }

    #[tauri::command(async)]
    pub async fn activate_distribution_access(
        app: AppHandle,
        access_key: String,
    ) -> Result<serde_json::Value, String> {
        locked()
    }

    #[tauri::command(async)]
    pub async fn register_device_license(app: AppHandle) -> Result<serde_json::Value, String> {
        locked()
    }

    #[tauri::command(async)]
    pub async fn bootstrap_distribution_access(
        app: AppHandle,
    ) -> Result<serde_json::Value, String> {
        locked()
    }
}

pub mod meeting {
    //! Meeting mode and Lecture mode: unavailable in the open-source build.

    use super::{locked, AppState};
    use crate::events::{MeetingDetectionDecision, MeetingDetectionPendingPayload};
    use tauri::State;
    use ultravox_core::AudioRecording;

    #[tauri::command]
    pub async fn start_meeting(state: State<'_, AppState>) -> Result<String, String> {
        locked()
    }

    #[tauri::command]
    pub async fn start_lecture(state: State<'_, AppState>) -> Result<String, String> {
        locked()
    }

    #[tauri::command]
    pub async fn respond_meeting_detection(
        state: State<'_, AppState>,
        detection_id: String,
        decision: MeetingDetectionDecision,
    ) -> Result<String, String> {
        locked()
    }

    #[tauri::command]
    pub async fn stop_meeting(state: State<'_, AppState>) -> Result<AudioRecording, String> {
        locked()
    }

    #[tauri::command]
    pub async fn get_pending_meeting_detection(
        state: State<'_, AppState>,
    ) -> Result<Option<MeetingDetectionPendingPayload>, String> {
        locked()
    }

    /// Shutdown cleanup: no meeting capture can exist in this build.
    pub async fn discard_active_meeting(state: &AppState) {
        let _ = state;
    }
}
pub mod retex {
    //! Retex custom dictionaries: unavailable in the open-source build.

    use super::{locked, AppHandle, AppState};
    use tauri::State;

    #[tauri::command]
    pub async fn choose_retex_vaults(state: State<'_, AppState>) -> Result<(), String> {
        locked()
    }

    #[tauri::command]
    pub async fn scan_retex_dictionary(
        state: State<'_, AppState>,
    ) -> Result<serde_json::Value, String> {
        locked()
    }

    /// No automatic vocabulary refresh runs in the open-source build.
    pub fn start_retex_auto_refresh(app: AppHandle) {
        let _ = app;
    }
}

pub mod voice_studio {
    //! Voice Studio: unavailable in the open-source build.

    use super::{locked, AppState};
    use tauri::{State, WebviewWindow};
    use uuid::Uuid;

    #[tauri::command]
    pub async fn voice_generations_list(
        window: WebviewWindow,
        state: State<'_, AppState>,
    ) -> Result<Vec<serde_json::Value>, String> {
        locked()
    }

    #[tauri::command]
    pub async fn voice_generation_delete(
        window: WebviewWindow,
        state: State<'_, AppState>,
        path_or_id: String,
    ) -> Result<(), String> {
        locked()
    }

    #[tauri::command]
    pub async fn voice_generations_delete_all(
        window: WebviewWindow,
        state: State<'_, AppState>,
    ) -> Result<(), String> {
        locked()
    }

    #[tauri::command]
    pub async fn vs_list_voices(
        window: WebviewWindow,
        state: State<'_, AppState>,
    ) -> Result<Vec<serde_json::Value>, String> {
        locked()
    }

    #[tauri::command]
    pub async fn vs_corpus_candidates(
        window: WebviewWindow,
        state: State<'_, AppState>,
    ) -> Result<Vec<serde_json::Value>, String> {
        locked()
    }

    #[tauri::command]
    pub async fn vs_create_voice(
        window: WebviewWindow,
        state: State<'_, AppState>,
        name: String,
        recording_ids: Option<Vec<Uuid>>,
        import_path: Option<String>,
        language: String,
    ) -> Result<serde_json::Value, String> {
        locked()
    }

    #[tauri::command]
    pub async fn vs_delete_voice(
        window: WebviewWindow,
        state: State<'_, AppState>,
        id: Uuid,
    ) -> Result<(), String> {
        locked()
    }

    #[tauri::command]
    pub async fn vs_rename_voice(
        window: WebviewWindow,
        state: State<'_, AppState>,
        id: Uuid,
        name: String,
    ) -> Result<(), String> {
        locked()
    }

    #[tauri::command]
    pub async fn vs_speak(
        window: WebviewWindow,
        state: State<'_, AppState>,
        voice_id: Option<Uuid>,
        builtin: Option<String>,
        text: String,
        language: String,
        use_int8: bool,
    ) -> Result<serde_json::Value, String> {
        locked()
    }

    #[tauri::command]
    pub async fn vs_tts_status(
        window: WebviewWindow,
        state: State<'_, AppState>,
    ) -> Result<serde_json::Value, String> {
        locked()
    }

    #[tauri::command]
    pub async fn vs_read_audio(
        window: WebviewWindow,
        state: State<'_, AppState>,
        path: String,
    ) -> Result<Vec<u8>, String> {
        locked()
    }
}
