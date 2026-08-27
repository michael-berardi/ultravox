/* Minimal C ABI for the UltraVox Light macOS native bridge. */
#ifndef ULTRAVOX_MACOS_BRIDGE_H
#define ULTRAVOX_MACOS_BRIDGE_H
#include <stdint.h>
#ifdef __cplusplus
extern "C" {
#endif
const char *ultravox_macos_bridge_version(void);
void ultravox_macos_bridge_free_string(char *s);
int32_t ultravox_macos_bridge_microphone_authorization_status(void);
int32_t ultravox_macos_bridge_request_microphone_access(void);
int32_t ultravox_macos_bridge_is_accessibility_trusted(int32_t prompt);
int32_t ultravox_macos_bridge_get_caret_position(double *x, double *y);
int32_t ultravox_macos_bridge_capture_insertion_target(double *x, double *y);
void ultravox_macos_bridge_clear_insertion_target(void);
int32_t ultravox_macos_bridge_paste_text(const char *text);
typedef void (*ultravox_macos_bridge_hotkey_callback)(int32_t event, const char *combo);
int32_t ultravox_macos_bridge_start_modifier_hotkey(const char *modifier);
int32_t ultravox_macos_bridge_stop_modifier_hotkey(void);
int32_t ultravox_macos_bridge_start_key_combination_hotkey(const char *combo, int32_t hold_to_record);
int32_t ultravox_macos_bridge_stop_key_combination_hotkey(void);
void ultravox_macos_bridge_set_key_combination_callback(ultravox_macos_bridge_hotkey_callback callback);
int32_t ultravox_macos_bridge_show_indicator(double x, double y);
int32_t ultravox_macos_bridge_set_indicator_state(const char *state);
int32_t ultravox_macos_bridge_hide_indicator(void);
int32_t ultravox_macos_bridge_transcribe_file_with_version(const char *path, const char *version, const char *recording_id, const char *directory, char **text);
int32_t ultravox_macos_bridge_cancel_transcription(const char *recording_id);
int32_t ultravox_macos_bridge_prepare_model(const char *version, const char *directory);
int32_t ultravox_macos_bridge_is_model_downloaded(const char *version, const char *directory);
double ultravox_macos_bridge_get_model_progress(const char *version);
#ifdef __cplusplus
}
#endif
#endif
