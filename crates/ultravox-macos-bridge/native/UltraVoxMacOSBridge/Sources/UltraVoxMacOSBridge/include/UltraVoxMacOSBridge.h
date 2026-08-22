/*
 *  UltraVoxMacOSBridge.h
 *  Minimal C ABI for the UltraVox macOS native bridge.
 *
 *  This header is provided for documentation and Rust FFI declarations.
 *  The Swift target exports these symbols via @_cdecl.
 *
 *  License: MIT (see /LICENSE in the repository root)
 */

#ifndef ULTRAVOX_MACOS_BRIDGE_H
#define ULTRAVOX_MACOS_BRIDGE_H

#include <stdint.h>
#include <stdbool.h>

#ifdef __cplusplus
extern "C" {
#endif

/* Returns a version string. Caller must free the result with
 * ultravox_macos_bridge_free_string.
 */
const char *ultravox_macos_bridge_version(void);

/* Frees a string previously returned by this bridge. */
void ultravox_macos_bridge_free_string(char *s);

/* Microphone authorization: 0 not determined, 1 authorized, 2 denied, 3 restricted. */
int32_t ultravox_macos_bridge_microphone_authorization_status(void);
/* Screen Recording preflight: 0 unavailable/not granted, 1 granted. */
int32_t ultravox_macos_bridge_screen_recording_authorization_status(void);
int32_t ultravox_macos_bridge_request_screen_recording_access(void);
int32_t ultravox_macos_bridge_request_microphone_access(void);

/* Captures system and microphone audio through ScreenCaptureKit. */
int32_t ultravox_macos_bridge_start_meeting_capture(const char *path, char **error);
int32_t ultravox_macos_bridge_stop_meeting_capture(char **output_path, char **error);
typedef void (*ultravox_macos_bridge_meeting_failure_callback)(const char *error);
void ultravox_macos_bridge_set_meeting_capture_failure_callback(
    ultravox_macos_bridge_meeting_failure_callback callback
);

/* Accessibility and focused text insertion. */
int32_t ultravox_macos_bridge_is_accessibility_trusted(int32_t prompt);
int32_t ultravox_macos_bridge_get_caret_position(double *x, double *y);
int32_t ultravox_macos_bridge_capture_insertion_target(double *x, double *y);
void ultravox_macos_bridge_clear_insertion_target(void);
int32_t ultravox_macos_bridge_paste_text(const char *text);

/* Modifier-only hotkey compatibility surface. */
int32_t ultravox_macos_bridge_start_modifier_hotkey(const char *modifier);
int32_t ultravox_macos_bridge_stop_modifier_hotkey(void);

/* Global key-combination monitor. Event 0 is key-down and event 1 is key-up. */
typedef void (*ultravox_macos_bridge_hotkey_callback)(int32_t event, const char *combo);
int32_t ultravox_macos_bridge_start_key_combination_hotkey(
    const char *combo,
    int32_t hold_to_record
);
int32_t ultravox_macos_bridge_stop_key_combination_hotkey(void);
void ultravox_macos_bridge_set_key_combination_callback(
    ultravox_macos_bridge_hotkey_callback callback
);
int32_t ultravox_macos_bridge_start_meeting_hotkey(const char *combo);
int32_t ultravox_macos_bridge_stop_meeting_hotkey(void);
void ultravox_macos_bridge_set_meeting_hotkey_callback(
    ultravox_macos_bridge_hotkey_callback callback
);

/* Nonactivating recording/transcription indicator. */
int32_t ultravox_macos_bridge_show_indicator(double x, double y);
int32_t ultravox_macos_bridge_set_indicator_state(const char *state);
int32_t ultravox_macos_bridge_hide_indicator(void);
/* Media panel support. Activity detection uses only public CoreAudio process
 * state (kAudioHardwarePropertyProcessObjectList +
 * kAudioProcessPropertyIsRunningOutput), excludes this process, and never
 * captures audio or requires new permissions.
 */

/* Returns 1 when another process is running output audio and fills its PID,
 * app_name / bundle_id (either string may remain NULL when unknown); the
 * MediaRemote client PID is preferred, then a conservative browser
 * helper/main bundle-family match, then the first active process. Returns 0
 * when no other process is active. Caller frees strings with
 * ultravox_macos_bridge_free_string.
 */
int32_t ultravox_macos_bridge_active_audio_process(
    int32_t *process_id,
    char **app_name,
    char **bundle_id
);

/* Default-output volume in [0, 1]. Getters/setters return 1 on success and 0
 * when no default output device exists or it lacks a volume/mute control
 * (unsupported state). Callers must surface unsupported as "not available",
 * never as a fabricated value.
 */
int32_t ultravox_macos_bridge_get_output_volume(double *volume);
int32_t ultravox_macos_bridge_set_output_volume(double volume);
int32_t ultravox_macos_bridge_get_output_muted(int32_t *muted);
int32_t ultravox_macos_bridge_set_output_muted(int32_t muted);

/* Optional transport/metadata through MediaRemote, resolved at runtime via
 * dlopen/dlsym only; nothing is statically linked. Availability may change on
 * any OS release, so callers must treat absence as a normal state.
 */
/* Writes 1 for play/pause when the send API resolves. Previous/next require
 * supported-command and command-info APIs to report enabled. Missing
 * discovery symbols leave previous/next at 0.
 */
int32_t ultravox_macos_bridge_media_transport_capabilities(
    int32_t *play_pause,
    int32_t *previous,
    int32_t *next
);

/* Returns 1 and fills the owning PID, app name / bundle ID, title / artist /
 * album (NULL when absent), elapsed/duration seconds (-1 when unknown), plus
 * *is_playing (1 playing, 0 not playing, -1 unknown); 0 when MediaRemote is
 * unavailable or the reply did not arrive in time. Caller frees strings.
 */
int32_t ultravox_macos_bridge_now_playing(
    int32_t *process_id,
    char **app_name,
    char **bundle_id,
    char **title,
    char **artist,
    char **album,
    double *elapsed_seconds,
    double *duration_seconds,
    int32_t *is_playing
);

/* command is "play_pause", "previous", or "next". Returns 1 when sent,
 * 0 when unavailable or delivery failed, -1 for an unknown command.
 */
int32_t ultravox_macos_bridge_media_transport(const char *command);


/* Transcribes the audio file at path using FluidAudio / CoreML.
 * Defaults to the English v2 model. On success, writes a malloc-allocated
 * string into *text and returns 1. Caller must free *text with
 * ultravox_macos_bridge_free_string.
 */
int32_t ultravox_macos_bridge_transcribe_file(const char *path, char **text);

/* Same as ultravox_macos_bridge_transcribe_file, but lets the caller choose the
 * model version. version should be "v2" (English) or "v3" (multilingual).
 * recording_id enables targeted cancellation; directory optionally selects a
 * custom model cache.
 */
int32_t ultravox_macos_bridge_transcribe_file_with_version(
    const char *path,
    const char *version,
    const char *recording_id,
    const char *directory,
    char **text
);

/* Cancels the active FluidAudio transcription for the given recording_id.
 * Returns 1 if a matching in-flight transcription was found and cancelled,
 * 0 otherwise. Safe to call from any thread; the shared engine will not cancel
 * a different recording.
 */
int32_t ultravox_macos_bridge_cancel_transcription(const char *recording_id);

/* Downloads/loads FluidAudio model assets in the optional custom cache
 * directory.
 */
int32_t ultravox_macos_bridge_prepare_model(
    const char *version,
    const char *directory
);
int32_t ultravox_macos_bridge_is_model_downloaded(
    const char *version,
    const char *directory
);
double ultravox_macos_bridge_get_model_progress(const char *version);

#ifdef __cplusplus
}
#endif

#endif /* ULTRAVOX_MACOS_BRIDGE_H */
