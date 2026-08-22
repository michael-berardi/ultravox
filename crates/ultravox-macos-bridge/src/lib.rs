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
    fn ultravox_macos_bridge_active_audio_process(
        process_id: *mut c_int,
        app_name: *mut *mut c_char,
        bundle_id: *mut *mut c_char,
    ) -> c_int;
    fn ultravox_macos_bridge_get_output_volume(volume: *mut c_double) -> c_int;
    fn ultravox_macos_bridge_set_output_volume(volume: c_double) -> c_int;
    fn ultravox_macos_bridge_get_output_muted(muted: *mut c_int) -> c_int;
    fn ultravox_macos_bridge_set_output_muted(muted: c_int) -> c_int;
    fn ultravox_macos_bridge_media_remote_available() -> c_int;
    fn ultravox_macos_bridge_media_transport_capabilities(
        play_pause: *mut c_int,
        previous: *mut c_int,
        next: *mut c_int,
    ) -> c_int;
    fn ultravox_macos_bridge_now_playing(
        process_id: *mut c_int,
        app_name: *mut *mut c_char,
        bundle_id: *mut *mut c_char,
        title: *mut *mut c_char,
        artist: *mut *mut c_char,
        album: *mut *mut c_char,
        elapsed_seconds: *mut c_double,
        duration_seconds: *mut c_double,
        is_playing: *mut c_int,
    ) -> c_int;
    fn ultravox_macos_bridge_media_transport(command: *const c_char) -> c_int;
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

// MARK: - Media panel support

/// Source application detected through public CoreAudio process state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioSource {
    pub process_id: i32,
    pub app_name: Option<String>,
    pub bundle_id: Option<String>,
}

/// Default-output volume and mute state. `None` members mean the current
/// device does not support that control; callers must surface that as
/// "not available" rather than a fabricated value.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OutputVolumeState {
    pub volume: Option<f64>,
    pub muted: Option<bool>,
}

/// Now-playing metadata from optional runtime MediaRemote access.
#[derive(Debug, Clone, PartialEq)]
pub struct NowPlayingInfo {
    pub process_id: i32,
    pub app_name: Option<String>,
    pub bundle_id: Option<String>,
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub elapsed_seconds: Option<f64>,
    pub duration_seconds: Option<f64>,
    pub is_playing: Option<bool>,
}

/// Runtime MediaRemote command capabilities. Secondary commands are false
/// unless the supported-command and command-info APIs prove they are enabled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TransportCapabilities {
    pub play_pause: bool,
    pub previous: bool,
    pub next: bool,
}

/// Transport commands accepted by [`media_transport`].
pub const TRANSPORT_COMMANDS: [&str; 3] = ["play_pause", "previous", "next"];

/// Returns true only for an exact bundle ID or a known browser helper/main
/// family. Arbitrary bundle ID prefixes are deliberately rejected.
pub fn same_bundle_family(left: Option<&str>, right: Option<&str>) -> bool {
    fn family(bundle_id: &str) -> Option<&'static str> {
        const BROWSER_BASES: [&str; 7] = [
            "com.google.Chrome",
            "com.microsoft.edgemac",
            "com.brave.Browser",
            "company.thebrowser.Browser",
            "org.mozilla.firefox",
            "com.vivaldi.Vivaldi",
            "com.operasoftware.Opera",
        ];
        const SAFARI_FAMILY: [&str; 4] = [
            "com.apple.Safari",
            "com.apple.WebKit.Networking",
            "com.apple.WebKit.WebContent",
            "com.apple.WebKit.GPU",
        ];
        if SAFARI_FAMILY.contains(&bundle_id) {
            return Some("com.apple.Safari");
        }
        BROWSER_BASES.into_iter().find(|base| {
            bundle_id == *base
                || bundle_id
                    .strip_prefix(base)
                    .is_some_and(|suffix| suffix == ".helper" || suffix.starts_with(".helper."))
        })
    }

    match (left, right) {
        (Some(left), Some(right)) if left == right && !left.is_empty() => true,
        (Some(left), Some(right)) => family(left)
            .zip(family(right))
            .is_some_and(|(left_family, right_family)| left_family == right_family),
        _ => false,
    }
}

/// Returns true when source and now-playing metadata identify the same app.
pub fn same_media_app(source: &AudioSource, now_playing: &NowPlayingInfo) -> bool {
    source.process_id == now_playing.process_id
        || same_bundle_family(
            source.bundle_id.as_deref(),
            now_playing.bundle_id.as_deref(),
        )
}

/// Returns the best other process currently running audio output, or `None`
/// when no other process is active. The native bridge prefers the MediaRemote
/// client PID, then a known browser helper/main family, then the first active
/// process. Capture-free: only public process-state properties are read and
/// this process is always excluded.
pub fn active_audio_source() -> Option<AudioSource> {
    unsafe {
        let mut process_id: c_int = 0;
        let mut app_name: *mut c_char = std::ptr::null_mut();
        let mut bundle_id: *mut c_char = std::ptr::null_mut();
        if ultravox_macos_bridge_active_audio_process(
            &mut process_id,
            &mut app_name,
            &mut bundle_id,
        ) != 1
        {
            return None;
        }
        let app_name = take_bridge_string(app_name);
        let bundle_id = take_bridge_string(bundle_id);
        Some(AudioSource {
            process_id,
            app_name: (!app_name.is_empty()).then_some(app_name),
            bundle_id: (!bundle_id.is_empty()).then_some(bundle_id),
        })
    }
}

/// Reads default-output volume and mute. Unsupported controls report `None`.
pub fn output_volume_state() -> OutputVolumeState {
    unsafe {
        let mut volume: c_double = 0.0;
        let volume_ok = ultravox_macos_bridge_get_output_volume(&mut volume) == 1;
        let mut muted: c_int = 0;
        let muted_ok = ultravox_macos_bridge_get_output_muted(&mut muted) == 1;
        OutputVolumeState {
            volume: volume_ok.then(|| volume.clamp(0.0, 1.0)),
            muted: muted_ok.then_some(muted != 0),
        }
    }
}

/// Validates a volume value in [0, 1]; rejects NaN and out-of-range input.
fn validate_unit_volume(volume: f64) -> Result<f64, String> {
    if !volume.is_finite() || !(0.0..=1.0).contains(&volume) {
        return Err("volume must be between 0 and 1".to_string());
    }
    Ok(volume)
}

/// Sets the system default-output volume in [0, 1].
///
/// Errors when the value is invalid or the device has no settable volume.
pub fn set_system_volume(volume: f64) -> Result<(), String> {
    let clamped = validate_unit_volume(volume)?;
    let ok = unsafe { ultravox_macos_bridge_set_output_volume(clamped) } == 1;
    if ok {
        Ok(())
    } else {
        Err("default output device does not support volume changes".to_string())
    }
}

/// Sets the system default-output mute state.
///
/// Errors when the device has no mute control.
pub fn set_system_muted(muted: bool) -> Result<(), String> {
    let ok = unsafe { ultravox_macos_bridge_set_output_muted(muted as c_int) } == 1;
    if ok {
        Ok(())
    } else {
        Err("default output device does not support mute".to_string())
    }
}

/// Returns true when runtime MediaRemote transport/metadata access resolved.
pub fn transport_available() -> bool {
    unsafe { ultravox_macos_bridge_media_remote_available() != 0 }
}

/// Reads runtime MediaRemote command capabilities. Missing secondary command
/// discovery APIs safely report previous/next as false; play/pause remains
/// independently available when the send API resolves.
pub fn transport_capabilities() -> TransportCapabilities {
    unsafe {
        let mut play_pause = 0;
        let mut previous = 0;
        let mut next = 0;
        let _ = ultravox_macos_bridge_media_transport_capabilities(
            &mut play_pause,
            &mut previous,
            &mut next,
        );
        TransportCapabilities {
            play_pause: play_pause != 0,
            previous: previous != 0,
            next: next != 0,
        }
    }
}

fn valid_seconds(value: c_double) -> Option<f64> {
    value
        .is_finite()
        .then_some(value)
        .filter(|value| *value >= 0.0)
}

/// Fetches now-playing metadata, or `None` when MediaRemote is unavailable
/// or did not reply in time.
pub fn now_playing() -> Option<NowPlayingInfo> {
    unsafe {
        let mut process_id: c_int = 0;
        let mut app_name: *mut c_char = std::ptr::null_mut();
        let mut bundle_id: *mut c_char = std::ptr::null_mut();
        let mut title: *mut c_char = std::ptr::null_mut();
        let mut artist: *mut c_char = std::ptr::null_mut();
        let mut album: *mut c_char = std::ptr::null_mut();
        let mut elapsed_seconds: c_double = -1.0;
        let mut duration_seconds: c_double = -1.0;
        let mut is_playing: c_int = -1;
        if ultravox_macos_bridge_now_playing(
            &mut process_id,
            &mut app_name,
            &mut bundle_id,
            &mut title,
            &mut artist,
            &mut album,
            &mut elapsed_seconds,
            &mut duration_seconds,
            &mut is_playing,
        ) != 1
        {
            return None;
        }
        let app_name = take_bridge_string(app_name);
        let bundle_id = take_bridge_string(bundle_id);
        let title = take_bridge_string(title);
        let artist = take_bridge_string(artist);
        let album = take_bridge_string(album);
        Some(NowPlayingInfo {
            process_id,
            app_name: (!app_name.is_empty()).then_some(app_name),
            bundle_id: (!bundle_id.is_empty()).then_some(bundle_id),
            title: (!title.is_empty()).then_some(title),
            artist: (!artist.is_empty()).then_some(artist),
            album: (!album.is_empty()).then_some(album),
            elapsed_seconds: valid_seconds(elapsed_seconds),
            duration_seconds: valid_seconds(duration_seconds),
            is_playing: match is_playing {
                1 => Some(true),
                0 => Some(false),
                _ => None,
            },
        })
    }
}

/// Validates a media transport command without crossing the FFI boundary.
fn validate_media_transport_command(command: &str) -> Result<&str, String> {
    TRANSPORT_COMMANDS
        .contains(&command)
        .then_some(command)
        .ok_or_else(|| {
            format!(
                "unknown transport command: {command} (expected one of {})",
                TRANSPORT_COMMANDS.join(", ")
            )
        })
}

/// Sends a media transport command (`play_pause`, `previous`, or `next`)
/// through optional runtime MediaRemote access.
///
/// Errors on an unknown command without touching the FFI, or when the
/// transport is unavailable / delivery failed.
pub fn media_transport(command: &str) -> Result<(), String> {
    let command = validate_media_transport_command(command)?;
    let command = CString::new(command).map_err(|_| "command contained interior NUL")?;
    match unsafe { ultravox_macos_bridge_media_transport(command.as_ptr()) } {
        1 => Ok(()),
        _ => Err("media transport is unavailable on this system".to_string()),
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

    #[test]
    fn test_volume_validation_rejects_invalid_values() {
        assert_eq!(validate_unit_volume(0.0), Ok(0.0));
        assert_eq!(validate_unit_volume(1.0), Ok(1.0));
        assert_eq!(validate_unit_volume(0.42), Ok(0.42));
        assert!(validate_unit_volume(-0.01).is_err());
        assert!(validate_unit_volume(1.01).is_err());
        assert!(validate_unit_volume(f64::NAN).is_err());
        assert!(validate_unit_volume(f64::INFINITY).is_err());
    }

    #[test]
    fn test_media_transport_rejects_unknown_commands_without_ffi() {
        for command in ["", "stop", "PLAY_PAUSE", "play_pause;"] {
            let result = validate_media_transport_command(command);
            assert!(result.is_err(), "{command:?} should be rejected");
            assert!(result.unwrap_err().contains("unknown transport command"));
        }
    }

    #[test]
    fn test_transport_command_list_matches_accepted_inputs() {
        for command in TRANSPORT_COMMANDS {
            assert_eq!(validate_media_transport_command(command), Ok(command));
        }
    }

    #[test]
    fn test_bundle_family_matching_is_conservative() {
        assert!(same_bundle_family(
            Some("com.google.Chrome"),
            Some("com.google.Chrome.helper.renderer")
        ));
        assert!(same_bundle_family(
            Some("com.apple.Safari"),
            Some("com.apple.WebKit.WebContent")
        ));
        assert!(same_bundle_family(
            Some("com.example.player"),
            Some("com.example.player")
        ));
        assert!(!same_bundle_family(
            Some("com.google.Chrome"),
            Some("com.google.Chromeish.helper")
        ));
        assert!(!same_bundle_family(
            Some("com.google.Chrome"),
            Some("com.apple.Safari")
        ));
        assert!(!same_bundle_family(
            Some("com.apple.Music"),
            Some("com.spotify.client")
        ));
        assert!(!same_bundle_family(None, Some("com.google.Chrome")));
    }

    #[test]
    fn test_metadata_match_requires_pid_or_bundle_family() {
        let source = AudioSource {
            process_id: 42,
            app_name: Some("Chrome".to_string()),
            bundle_id: Some("com.google.Chrome".to_string()),
        };
        let browser_helper = NowPlayingInfo {
            process_id: 99,
            app_name: Some("Google Chrome Helper".to_string()),
            bundle_id: Some("com.google.Chrome.helper.renderer".to_string()),
            title: Some("Track".to_string()),
            artist: Some("Artist".to_string()),
            album: Some("Album".to_string()),
            elapsed_seconds: Some(12.5),
            duration_seconds: Some(180.0),
            is_playing: Some(true),
        };
        assert!(same_media_app(&source, &browser_helper));
        assert!(same_media_app(
            &source,
            &NowPlayingInfo {
                process_id: 42,
                ..browser_helper.clone()
            }
        ));
        assert!(!same_media_app(
            &source,
            &NowPlayingInfo {
                process_id: 99,
                bundle_id: Some("com.apple.Music".to_string()),
                ..browser_helper
            }
        ));
    }

    #[test]
    fn test_now_playing_wire_metadata_preserves_optional_fields() {
        let info = NowPlayingInfo {
            process_id: 7,
            app_name: Some("Music".to_string()),
            bundle_id: Some("com.apple.Music".to_string()),
            title: Some("Song".to_string()),
            artist: Some("Artist".to_string()),
            album: Some("Album".to_string()),
            elapsed_seconds: Some(1.25),
            duration_seconds: Some(123.0),
            is_playing: Some(false),
        };
        assert_eq!(info.album.as_deref(), Some("Album"));
        assert_eq!(info.elapsed_seconds, Some(1.25));
        assert_eq!(info.duration_seconds, Some(123.0));
        assert_eq!(info.app_name.as_deref(), Some("Music"));
    }

    #[test]
    fn test_invalid_wire_times_are_rejected() {
        assert_eq!(valid_seconds(-1.0), None);
        assert_eq!(valid_seconds(f64::NAN), None);
        assert_eq!(valid_seconds(f64::INFINITY), None);
        assert_eq!(valid_seconds(0.0), Some(0.0));
    }

    #[test]
    fn test_active_audio_source_is_well_formed() {
        // Environment-dependent: Some in any environment where another process
        // plays audio, None otherwise. Either way the struct must be coherent.
        if let Some(source) = active_audio_source() {
            assert!(source.process_id > 0);
        }
    }

    #[test]
    fn test_output_volume_state_is_well_formed() {
        let state = output_volume_state();
        if let Some(volume) = state.volume {
            assert!((0.0..=1.0).contains(&volume));
        }
    }
}
