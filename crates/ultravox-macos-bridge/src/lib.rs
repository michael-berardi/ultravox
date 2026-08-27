//! macOS-native capabilities used by UltraVox Light.
//!
//! The bridge keeps the C ABI small: microphone permissions, accessibility text
//! insertion, global recording shortcuts, the recording indicator, and local
//! FluidAudio/CoreML transcription.

use libc::{c_char, c_double, c_int};
use std::ffi::{CStr, CString};

extern "C" {
    fn ultravox_macos_bridge_version() -> *mut c_char;
    fn ultravox_macos_bridge_free_string(s: *mut c_char);
    fn ultravox_macos_bridge_is_accessibility_trusted(prompt: c_int) -> c_int;
    fn ultravox_macos_bridge_microphone_authorization_status() -> c_int;
    fn ultravox_macos_bridge_request_microphone_access() -> c_int;
    fn ultravox_macos_bridge_get_caret_position(x: *mut c_double, y: *mut c_double) -> c_int;
    fn ultravox_macos_bridge_capture_insertion_target(x: *mut c_double, y: *mut c_double) -> c_int;
    fn ultravox_macos_bridge_clear_insertion_target();
    fn ultravox_macos_bridge_paste_text(text: *const c_char) -> c_int;
    fn ultravox_macos_bridge_start_modifier_hotkey(modifier: *const c_char) -> c_int;
    fn ultravox_macos_bridge_stop_modifier_hotkey() -> c_int;
    fn ultravox_macos_bridge_start_key_combination_hotkey(combo: *const c_char, hold_to_record: c_int) -> c_int;
    fn ultravox_macos_bridge_stop_key_combination_hotkey() -> c_int;
    fn ultravox_macos_bridge_set_key_combination_callback(callback: extern "C" fn(event: c_int, combo: *const c_char));
    fn ultravox_macos_bridge_show_indicator(x: c_double, y: c_double) -> c_int;
    fn ultravox_macos_bridge_set_indicator_state(state: *const c_char) -> c_int;
    fn ultravox_macos_bridge_hide_indicator() -> c_int;
    fn ultravox_macos_bridge_transcribe_file_with_version(path: *const c_char, version: *const c_char, recording_id: *const c_char, directory: *const c_char, text: *mut *mut c_char) -> c_int;
    fn ultravox_macos_bridge_cancel_transcription(recording_id: *const c_char) -> c_int;
    fn ultravox_macos_bridge_prepare_model(version: *const c_char, directory: *const c_char) -> c_int;
    fn ultravox_macos_bridge_is_model_downloaded(version: *const c_char, directory: *const c_char) -> c_int;
    fn ultravox_macos_bridge_get_model_progress(version: *const c_char) -> c_double;
}

pub fn version() -> String {
    unsafe {
        let ptr = ultravox_macos_bridge_version();
        let value = c_char_to_string(ptr);
        ultravox_macos_bridge_free_string(ptr);
        value
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MicrophoneAuthorizationStatus { NotDetermined, Authorized, Denied, Restricted }

pub fn microphone_authorization_status() -> MicrophoneAuthorizationStatus {
    match unsafe { ultravox_macos_bridge_microphone_authorization_status() } {
        0 => MicrophoneAuthorizationStatus::NotDetermined,
        1 => MicrophoneAuthorizationStatus::Authorized,
        2 => MicrophoneAuthorizationStatus::Denied,
        _ => MicrophoneAuthorizationStatus::Restricted,
    }
}

pub fn request_microphone_access() -> bool {
    unsafe { ultravox_macos_bridge_request_microphone_access() != 0 }
}

pub fn is_accessibility_trusted(prompt: bool) -> bool {
    unsafe { ultravox_macos_bridge_is_accessibility_trusted(prompt as c_int) != 0 }
}

pub fn get_caret_position() -> (f64, f64, i32) {
    unsafe {
        let (mut x, mut y) = (0.0, 0.0);
        let found = ultravox_macos_bridge_get_caret_position(&mut x, &mut y);
        (x, y, found)
    }
}

pub fn capture_insertion_target() -> (f64, f64, bool) {
    unsafe {
        let (mut x, mut y) = (0.0, 0.0);
        let found = ultravox_macos_bridge_capture_insertion_target(&mut x, &mut y) != 0;
        (x, y, found)
    }
}

pub fn clear_insertion_target() { unsafe { ultravox_macos_bridge_clear_insertion_target() } }

pub fn paste_text(text: &str) -> i32 {
    unsafe { CString::new(text).map_or(-1, |text| ultravox_macos_bridge_paste_text(text.as_ptr())) }
}

pub fn start_modifier_hotkey(modifier: &str) -> i32 {
    unsafe { CString::new(modifier).map_or(-1, |value| ultravox_macos_bridge_start_modifier_hotkey(value.as_ptr())) }
}

pub fn stop_modifier_hotkey() -> i32 { unsafe { ultravox_macos_bridge_stop_modifier_hotkey() } }

pub fn start_key_combination_hotkey(combo: &str, hold_to_record: bool) -> i32 {
    unsafe {
        CString::new(combo).map_or(-1, |value| {
            ultravox_macos_bridge_start_key_combination_hotkey(value.as_ptr(), hold_to_record as c_int)
        })
    }
}

pub fn stop_key_combination_hotkey() -> i32 { unsafe { ultravox_macos_bridge_stop_key_combination_hotkey() } }

pub fn set_key_combination_callback(callback: extern "C" fn(event: c_int, combo: *const c_char)) {
    unsafe { ultravox_macos_bridge_set_key_combination_callback(callback) }
}

pub fn show_indicator(x: f64, y: f64) -> i32 { unsafe { ultravox_macos_bridge_show_indicator(x, y) } }

pub fn set_indicator_state(state: &str) -> i32 {
    unsafe { CString::new(state).map_or(-1, |value| ultravox_macos_bridge_set_indicator_state(value.as_ptr())) }
}

pub fn hide_indicator() -> i32 { unsafe { ultravox_macos_bridge_hide_indicator() } }

pub fn transcribe_file(path: &str) -> Result<String, ()> { transcribe_file_with_version(path, "v2") }

pub fn transcribe_file_with_version(path: &str, version: &str) -> Result<String, ()> {
    transcribe_file_with_version_for_recording_in_directory(path, version, "", None)
}

pub fn transcribe_file_with_version_for_recording(path: &str, version: &str, recording_id: &str) -> Result<String, ()> {
    transcribe_file_with_version_for_recording_in_directory(path, version, recording_id, None)
}

pub fn transcribe_file_with_version_for_recording_in_directory(path: &str, version: &str, recording_id: &str, directory: Option<&std::path::Path>) -> Result<String, ()> {
    unsafe {
        let path = CString::new(path).map_err(|_| ())?;
        let version = CString::new(version).map_err(|_| ())?;
        let recording_id = CString::new(recording_id).map_err(|_| ())?;
        let directory = directory.and_then(|path| CString::new(path.to_string_lossy().as_bytes()).ok());
        let mut text = std::ptr::null_mut();
        let success = ultravox_macos_bridge_transcribe_file_with_version(path.as_ptr(), version.as_ptr(), recording_id.as_ptr(), directory.as_ref().map_or(std::ptr::null(), |path| path.as_ptr()), &mut text);
        let value = c_char_to_string(text);
        ultravox_macos_bridge_free_string(text);
        if success != 0 { Ok(value) } else { Err(()) }
    }
}

pub fn cancel_transcription(recording_id: &str) -> bool {
    unsafe { CString::new(recording_id).map_or(false, |value| ultravox_macos_bridge_cancel_transcription(value.as_ptr()) != 0) }
}

pub fn prepare_model(version: &str, directory: Option<&std::path::Path>) -> bool {
    call_model(version, directory, |version, directory| unsafe { ultravox_macos_bridge_prepare_model(version, directory) != 0 })
}

pub fn is_model_downloaded(version: &str, directory: Option<&std::path::Path>) -> bool {
    call_model(version, directory, |version, directory| unsafe { ultravox_macos_bridge_is_model_downloaded(version, directory) != 0 })
}

fn call_model(version: &str, directory: Option<&std::path::Path>, call: impl FnOnce(*const c_char, *const c_char) -> bool) -> bool {
    let Ok(version) = CString::new(version) else { return false };
    let directory = directory.and_then(|path| CString::new(path.to_string_lossy().as_bytes()).ok());
    call(version.as_ptr(), directory.as_ref().map_or(std::ptr::null(), |path| path.as_ptr()))
}

pub fn model_progress(version: &str) -> f64 {
    unsafe { CString::new(version).map_or(0.0, |value| ultravox_macos_bridge_get_model_progress(value.as_ptr()).clamp(0.0, 1.0)) }
}

fn c_char_to_string(ptr: *const c_char) -> String {
    if ptr.is_null() { return String::new(); }
    unsafe { CStr::from_ptr(ptr).to_string_lossy().into_owned() }
}
