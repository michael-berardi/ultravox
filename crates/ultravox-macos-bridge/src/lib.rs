//! Minimal macOS-native bridge for Rust/Tauri UltraVox parity.
//!
//! This crate exposes macOS-only platform capabilities (AX caret positioning,
//! CGEvent paste, modifier-only hotkey taps, nonactivating indicator windows,
//! and FluidAudio / CoreML transcription) to Rust via a small Swift static
//! library. The corresponding Swift source lives in
//! `native/UltraVoxMacOSBridge/Sources/UltraVoxMacOSBridge`.
//!
//! Native calls use explicit success values so the desktop shell can surface
//! permission, shortcut-registration, insertion, and transcription failures.
//!
//! License: MIT (see /LICENSE in the repository root)

use libc::{c_char, c_double, c_int};
use std::ffi::{CStr, CString};

#[allow(dead_code)]
extern "C" {
    fn ultravox_macos_bridge_version() -> *mut c_char;
    fn ultravox_macos_bridge_free_string(s: *mut c_char);

    fn ultravox_macos_bridge_is_accessibility_trusted(prompt: c_int) -> c_int;
    fn ultravox_macos_bridge_microphone_authorization_status() -> c_int;
    fn ultravox_macos_bridge_request_microphone_access() -> c_int;
    fn ultravox_macos_bridge_screen_recording_authorization_status() -> c_int;
    fn ultravox_macos_bridge_request_screen_recording_access() -> c_int;
    fn ultravox_macos_bridge_start_meeting_capture(
        path: *const c_char,
        error: *mut *mut c_char,
    ) -> c_int;
    fn ultravox_macos_bridge_stop_meeting_capture(
        output_path: *mut *mut c_char,
        error: *mut *mut c_char,
    ) -> c_int;
    fn ultravox_macos_bridge_set_meeting_capture_failure_callback(
        callback: extern "C" fn(error: *const c_char),
    );
    fn ultravox_macos_bridge_get_caret_position(x: *mut c_double, y: *mut c_double) -> c_int;
    fn ultravox_macos_bridge_capture_insertion_target(x: *mut c_double, y: *mut c_double) -> c_int;
    fn ultravox_macos_bridge_clear_insertion_target();
    fn ultravox_macos_bridge_paste_text(text: *const c_char) -> c_int;

    fn ultravox_macos_bridge_start_modifier_hotkey(modifier: *const c_char) -> c_int;
    fn ultravox_macos_bridge_stop_modifier_hotkey() -> c_int;

    fn ultravox_macos_bridge_start_key_combination_hotkey(
        combo: *const c_char,
        hold_to_record: c_int,
    ) -> c_int;
    fn ultravox_macos_bridge_stop_key_combination_hotkey() -> c_int;
    fn ultravox_macos_bridge_set_key_combination_callback(
        callback: extern "C" fn(event: c_int, combo: *const c_char),
    );
    fn ultravox_macos_bridge_start_meeting_hotkey(combo: *const c_char) -> c_int;
    fn ultravox_macos_bridge_stop_meeting_hotkey() -> c_int;
    fn ultravox_macos_bridge_set_meeting_hotkey_callback(
        callback: extern "C" fn(event: c_int, combo: *const c_char),
    );

    fn ultravox_macos_bridge_show_indicator(x: c_double, y: c_double) -> c_int;
    fn ultravox_macos_bridge_set_indicator_state(state: *const c_char) -> c_int;
    fn ultravox_macos_bridge_hide_indicator() -> c_int;

    fn ultravox_macos_bridge_transcribe_file(path: *const c_char, text: *mut *mut c_char) -> c_int;
    fn ultravox_macos_bridge_transcribe_file_with_version(
        path: *const c_char,
        version: *const c_char,
        recording_id: *const c_char,
        directory: *const c_char,
        text: *mut *mut c_char,
    ) -> c_int;
    fn ultravox_macos_bridge_cancel_transcription(recording_id: *const c_char) -> c_int;
    fn ultravox_macos_bridge_prepare_model(
        version: *const c_char,
        directory: *const c_char,
    ) -> c_int;
    fn ultravox_macos_bridge_is_model_downloaded(
        version: *const c_char,
        directory: *const c_char,
    ) -> c_int;
    fn ultravox_macos_bridge_get_model_progress(version: *const c_char) -> c_double;
}

/// Returns the bridge version string.
pub fn version() -> String {
    unsafe {
        let ptr = ultravox_macos_bridge_version();
        let s = c_char_to_string(ptr);
        ultravox_macos_bridge_free_string(ptr);
        s
    }
}

/// Current macOS microphone authorization for this process.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MicrophoneAuthorizationStatus {
    NotDetermined,
    Authorized,
    Denied,
    Restricted,
}

pub fn microphone_authorization_status() -> MicrophoneAuthorizationStatus {
    match unsafe { ultravox_macos_bridge_microphone_authorization_status() } {
        0 => MicrophoneAuthorizationStatus::NotDetermined,
        1 => MicrophoneAuthorizationStatus::Authorized,
        2 => MicrophoneAuthorizationStatus::Denied,
        _ => MicrophoneAuthorizationStatus::Restricted,
    }
}

/// Requests microphone access when authorization has not yet been decided.
pub fn request_microphone_access() -> bool {
    unsafe { ultravox_macos_bridge_request_microphone_access() != 0 }
}

/// Runtime Screen Recording preflight; TCC database rows are not authoritative.
pub fn screen_recording_authorized() -> bool {
    unsafe { ultravox_macos_bridge_screen_recording_authorization_status() != 0 }
}

/// Requests Screen Recording access after an explicit user action.
pub fn request_screen_recording_access() -> bool {
    unsafe { ultravox_macos_bridge_request_screen_recording_access() != 0 }
}

/// Starts a ScreenCaptureKit meeting recording containing system and microphone audio.
pub fn start_meeting_capture(path: &std::path::Path) -> Result<(), String> {
    let path = CString::new(path.to_string_lossy().as_bytes())
        .map_err(|_| "meeting recording path contains an invalid null byte".to_string())?;
    let mut error = std::ptr::null_mut();
    let result = unsafe { ultravox_macos_bridge_start_meeting_capture(path.as_ptr(), &mut error) };
    let error = take_bridge_string(error);
    if result != 0 {
        Ok(())
    } else if error.is_empty() {
        Err("meeting capture could not start".to_string())
    } else {
        Err(error)
    }
}

/// Stops meeting capture, finalizes the recording, and returns its audio-only file.
pub fn stop_meeting_capture() -> Result<std::path::PathBuf, String> {
    let mut output_path = std::ptr::null_mut();
    let mut error = std::ptr::null_mut();
    let result =
        unsafe { ultravox_macos_bridge_stop_meeting_capture(&mut output_path, &mut error) };
    let output_path = take_bridge_string(output_path);
    let error = take_bridge_string(error);
    if result != 0 && !output_path.is_empty() {
        Ok(output_path.into())
    } else if error.is_empty() {
        Err("meeting capture could not stop".to_string())
    } else {
        Err(error)
    }
}

/// Registers a callback for asynchronous meeting recording failures.
///
/// The callback is invoked from a ScreenCaptureKit delegate thread and must
/// return immediately.
pub fn set_meeting_capture_failure_callback(callback: extern "C" fn(error: *const c_char)) {
    unsafe { ultravox_macos_bridge_set_meeting_capture_failure_callback(callback) }
}

/// Checks whether this process has macOS Accessibility access. When `prompt`
/// is true, macOS may show the standard permission prompt.
pub fn is_accessibility_trusted(prompt: bool) -> bool {
    unsafe { ultravox_macos_bridge_is_accessibility_trusted(prompt as c_int) != 0 }
}

/// Queries the current caret position via macOS Accessibility APIs.
///
/// Returns `(x, y, found)` where `found` is non-zero when the position could be
/// read from the focused accessibility element.
pub fn get_caret_position() -> (f64, f64, i32) {
    unsafe {
        let mut x: c_double = 0.0;
        let mut y: c_double = 0.0;
        let found = ultravox_macos_bridge_get_caret_position(&mut x, &mut y);
        (x, y, found)
    }
}

/// Captures the focused insertion target and returns the best indicator point.
/// `has_target` is false when Accessibility access is unavailable or no focused
/// UI element can be read; the coordinates then fall back to the mouse.
pub fn capture_insertion_target() -> (f64, f64, bool) {
    unsafe {
        let mut x: c_double = 0.0;
        let mut y: c_double = 0.0;
        let has_target = ultravox_macos_bridge_capture_insertion_target(&mut x, &mut y) != 0;
        (x, y, has_target)
    }
}

/// Releases the focused insertion target retained for the active recording.
pub fn clear_insertion_target() {
    unsafe { ultravox_macos_bridge_clear_insertion_target() }
}

/// Pastes `text` into the target captured at recording start. The native bridge
/// first tries direct AX insertion, then falls back to restoring the target
/// focus and posting Cmd+V while preserving the previous clipboard contents.
/// Returns a non-zero value on success.
pub fn paste_text(text: &str) -> i32 {
    unsafe {
        match CString::new(text) {
            Ok(cstr) => ultravox_macos_bridge_paste_text(cstr.as_ptr()),
            Err(_) => -1,
        }
    }
}

/// Starts a modifier-only hotkey tap for a physical modifier key.
///
/// Supported modifiers are `leftOption`, `rightOption`, and `rightCommand`.
/// The native layer treats `leftShift`, `rightShift`, `shift`, and `none` as
/// disabled so that Shift cannot be used as a modifier-only recording trigger.
/// Returns a non-zero value on success and `0` / `-1` on failure or when the
/// requested modifier is not supported.
pub fn start_modifier_hotkey(modifier: &str) -> i32 {
    unsafe {
        match CString::new(modifier) {
            Ok(cstr) => ultravox_macos_bridge_start_modifier_hotkey(cstr.as_ptr()),
            Err(_) => -1,
        }
    }
}

/// Stops the active modifier-only hotkey tap.
pub fn stop_modifier_hotkey() -> i32 {
    unsafe { ultravox_macos_bridge_stop_modifier_hotkey() }
}

/// Registers a key combination hotkey (e.g. "Option+Backtick") and, when
/// `hold_to_record` is true, reports key-down / key-up events through the
/// callback registered via [`set_key_combination_callback`].
pub fn start_key_combination_hotkey(combo: &str, hold_to_record: bool) -> i32 {
    unsafe {
        match CString::new(combo) {
            Ok(cstr) => ultravox_macos_bridge_start_key_combination_hotkey(
                cstr.as_ptr(),
                hold_to_record as c_int,
            ),
            Err(_) => -1,
        }
    }
}

/// Stops the key combination hotkey tap.
pub fn stop_key_combination_hotkey() -> i32 {
    unsafe { ultravox_macos_bridge_stop_key_combination_hotkey() }
}

/// Registers a Rust callback that receives key-down (0) and key-up (1) events
/// from the macOS key-combination hotkey tap.
///
/// # Safety
///
/// The callback is invoked from a CoreFoundation runloop thread. It must be
/// `extern "C"`, thread-safe, and must not block.
pub fn set_key_combination_callback(callback: extern "C" fn(event: c_int, combo: *const c_char)) {
    unsafe { ultravox_macos_bridge_set_key_combination_callback(callback) }
}

/// Registers the global shortcut used to toggle meeting mode.
pub fn start_meeting_hotkey(combo: &str) -> i32 {
    unsafe {
        match CString::new(combo) {
            Ok(cstr) => ultravox_macos_bridge_start_meeting_hotkey(cstr.as_ptr()),
            Err(_) => -1,
        }
    }
}

/// Stops the meeting-mode hotkey tap.
pub fn stop_meeting_hotkey() -> i32 {
    unsafe { ultravox_macos_bridge_stop_meeting_hotkey() }
}

/// Registers the meeting-mode key-down callback.
pub fn set_meeting_hotkey_callback(callback: extern "C" fn(event: c_int, combo: *const c_char)) {
    unsafe { ultravox_macos_bridge_set_meeting_hotkey_callback(callback) }
}

/// Shows a nonactivating indicator panel near the given point.
///
/// Returns a non-zero value on success. The native implementation creates an
/// `NSPanel` with `.nonactivatingPanel` behavior; it does not require or steal
/// foreground activation.
pub fn show_indicator(x: f64, y: f64) -> i32 {
    unsafe { ultravox_macos_bridge_show_indicator(x, y) }
}

/// Updates the visible nonactivating indicator state.
pub fn set_indicator_state(state: &str) -> i32 {
    unsafe {
        match CString::new(state) {
            Ok(cstr) => ultravox_macos_bridge_set_indicator_state(cstr.as_ptr()),
            Err(_) => -1,
        }
    }
}

/// Hides the nonactivating indicator panel.
pub fn hide_indicator() -> i32 {
    unsafe { ultravox_macos_bridge_hide_indicator() }
}

/// Transcribes the audio file at `path` using the configured engine.
///
/// On macOS this routes to the native Swift bridge and loads the FluidAudio
/// English v2 model by default. If FluidAudio integration is not available,
/// the call returns an error so callers do not receive fake text.
pub fn transcribe_file(path: &str) -> Result<String, ()> {
    transcribe_file_with_version(path, "v2")
}

/// Transcribes the audio file at `path` using the specified FluidAudio model
/// version. `version` should be `"v2"` (English) or `"v3"` (multilingual).
pub fn transcribe_file_with_version(path: &str, version: &str) -> Result<String, ()> {
    transcribe_file_with_version_for_recording(path, version, "")
}

/// Transcribes the audio file at `path` using the specified FluidAudio model
/// version, associating the work with a recording identity. The recording_id
/// lets the caller cancel this specific transcription later via
/// [`cancel_transcription`].
pub fn transcribe_file_with_version_for_recording(
    path: &str,
    version: &str,
    recording_id: &str,
) -> Result<String, ()> {
    transcribe_file_with_version_for_recording_in_directory(path, version, recording_id, None)
}

/// Transcribes a recording with models loaded from the configured directory.
/// Passing `None` uses FluidAudio's standard cache directory.
pub fn transcribe_file_with_version_for_recording_in_directory(
    path: &str,
    version: &str,
    recording_id: &str,
    directory: Option<&std::path::Path>,
) -> Result<String, ()> {
    unsafe {
        let cstr = CString::new(path).map_err(|_| ())?;
        let version_cstr = CString::new(version).map_err(|_| ())?;
        let recording_id_cstr = CString::new(recording_id).map_err(|_| ())?;
        let directory_cstr =
            directory.and_then(|path| CString::new(path.to_string_lossy().as_bytes()).ok());
        let mut text: *mut c_char = std::ptr::null_mut();
        let success = ultravox_macos_bridge_transcribe_file_with_version(
            cstr.as_ptr(),
            version_cstr.as_ptr(),
            recording_id_cstr.as_ptr(),
            directory_cstr
                .as_ref()
                .map_or(std::ptr::null(), |path| path.as_ptr()),
            &mut text,
        );
        let result = c_char_to_string(text);
        ultravox_macos_bridge_free_string(text);
        if success != 0 {
            Ok(result)
        } else {
            Err(())
        }
    }
}

/// Cancels the active FluidAudio transcription for the given recording identity.
/// Returns `true` when a matching in-flight transcription was found and cancelled.
/// The shared engine will never cancel a different recording.
pub fn cancel_transcription(recording_id: &str) -> bool {
    unsafe {
        match CString::new(recording_id) {
            Ok(cstr) => ultravox_macos_bridge_cancel_transcription(cstr.as_ptr()) != 0,
            Err(_) => false,
        }
    }
}

/// Downloads and loads the FluidAudio model for the given version.
/// Returns `true` when the model is ready to use.
pub fn prepare_model(version: &str, directory: Option<&std::path::Path>) -> bool {
    let Ok(version) = CString::new(version) else {
        return false;
    };
    let directory = directory.and_then(|path| CString::new(path.to_string_lossy().as_bytes()).ok());
    unsafe {
        ultravox_macos_bridge_prepare_model(
            version.as_ptr(),
            directory
                .as_ref()
                .map_or(std::ptr::null(), |path| path.as_ptr()),
        ) != 0
    }
}

/// Returns `true` if the FluidAudio model is already downloaded.
pub fn is_model_downloaded(version: &str, directory: Option<&std::path::Path>) -> bool {
    let Ok(version) = CString::new(version) else {
        return false;
    };
    let directory = directory.and_then(|path| CString::new(path.to_string_lossy().as_bytes()).ok());
    unsafe {
        ultravox_macos_bridge_is_model_downloaded(
            version.as_ptr(),
            directory
                .as_ref()
                .map_or(std::ptr::null(), |path| path.as_ptr()),
        ) != 0
    }
}

/// Returns the latest native model preparation progress in [0, 1].
pub fn model_progress(version: &str) -> f64 {
    unsafe {
        CString::new(version)
            .map(|value| ultravox_macos_bridge_get_model_progress(value.as_ptr()))
            .unwrap_or(0.0)
    }
}

fn take_bridge_string(ptr: *mut c_char) -> String {
    if ptr.is_null() {
        return String::new();
    }
    let value = c_char_to_string(ptr);
    unsafe { ultravox_macos_bridge_free_string(ptr) };
    value
}

fn c_char_to_string(ptr: *const c_char) -> String {
    if ptr.is_null() {
        return String::new();
    }
    unsafe { CStr::from_ptr(ptr).to_string_lossy().into_owned() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_version_returns_non_empty() {
        let v = version();
        assert!(!v.is_empty(), "version string should not be empty");
    }

    #[test]
    fn test_microphone_authorization_status_returns_known_value() {
        assert!(matches!(
            microphone_authorization_status(),
            MicrophoneAuthorizationStatus::NotDetermined
                | MicrophoneAuthorizationStatus::Authorized
                | MicrophoneAuthorizationStatus::Denied
                | MicrophoneAuthorizationStatus::Restricted
        ));
    }

    #[test]
    #[ignore = "Interactive caret lookup requires a UI runloop and can hang in headless tests"]
    fn test_caret_bridge_returns_signal() {
        let (_x, _y, found) = get_caret_position();
        // Without accessibility focus in a test environment this will usually
        // be 0, but the bridge must return a well-formed result.
        assert!(found == 0 || found == 1);
    }

    #[test]
    #[ignore = "Interactive native paste requires a UI runloop and can hang in headless tests"]
    fn test_paste_text_bridge_returns_code() {
        let result = paste_text("hello");
        // In a headless / non-focusing test environment the real paste may
        // return 0; we only verify the bridge does not panic and returns a code.
        assert!(result >= -1);
    }

    #[test]
    #[ignore = "Interactive event taps must be registered on a running macOS UI runloop"]
    fn test_key_combination_can_be_registered() {
        let result = start_key_combination_hotkey("Option+Backtick", true);
        assert!(
            result >= 0,
            "key combination hotkey should register without crashing"
        );
        let stopped = stop_key_combination_hotkey();
        assert!(stopped >= 0);
    }

    #[test]
    fn test_transcribe_missing_file_fails() {
        let result = transcribe_file("/this/path/does/not/exist.wav");
        assert!(result.is_err(), "missing file should fail validation");
    }
}
