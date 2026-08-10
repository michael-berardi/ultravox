# UltraVox macOS Bridge

Minimal macOS-native bridge for Rust/Tauri UltraVox parity. Lives at
`crates/ultravox-macos-bridge` so it does not conflict with the existing
UltraVox Xcode project or the `asian-autocorrect` workspace.

## Layout

```
crates/ultravox-macos-bridge/
├── Cargo.toml                  # Rust crate metadata
├── build.rs                    # Builds native/UltraVoxMacOSBridge via SwiftPM
├── src/lib.rs                  # Rust FFI declarations + safe wrappers
└── native/UltraVoxMacOSBridge/
    ├── Package.swift           # SwiftPM static library, macOS 13+
    └── Sources/UltraVoxMacOSBridge/
        ├── UltraVoxMacOSBridge.swift   # @_cdecl ABI stubs
        └── include/
            └── UltraVoxMacOSBridge.h    # C ABI documentation header
```

## Functions exposed (currently no-op stubs)

| Rust API                         | C symbol                               | Parity target                               |
|----------------------------------|----------------------------------------|---------------------------------------------|
| `version()`                      | `ultravox_macos_bridge_version`          | Bridge health / version string              |
| `get_caret_position()`           | `ultravox_macos_bridge_get_caret_position` | AX caret positioning                        |
| `paste_text(text)`               | `ultravox_macos_bridge_paste_text`       | CGEvent paste                               |
| `start_modifier_hotkey(mod)`     | `ultravox_macos_bridge_start_modifier_hotkey` | Modifier-only hotkey tap                 |
| `stop_modifier_hotkey()`         | `ultravox_macos_bridge_stop_modifier_hotkey`  | Stop modifier tap                        |
| `show_indicator(x, y)`           | `ultravox_macos_bridge_show_indicator`   | Nonactivating indicator (NSPanel)           |
| `hide_indicator()`               | `ultravox_macos_bridge_hide_indicator`   | Hide indicator                              |
| `transcribe_file(path)`          | `ultravox_macos_bridge_transcribe_file`  | FluidAudio / CoreML transcription           |

## Integration notes

- The Swift package is built automatically by `build.rs` on macOS hosts.
- On non-macOS hosts, `build.rs` is a no-op so `cargo check` can still validate
  the Rust code, but the Swift library is not linked.
- The existing UltraVox Swift baseline is **not** modified; this is a
  parallel, isolated bridge intended for the future Rust/Tauri rewrite.

## Next steps

1. Implement `ultravox_macos_bridge_get_caret_position` by porting the
   `FocusUtils.getCaretRect()` logic from `UltraVox/Utils/FocusUtils.swift`.
2. Implement `ultravox_macos_bridge_paste_text` using `CGEvent` keyboard events.
3. Implement `ultravox_macos_bridge_start_modifier_hotkey` by porting the
   `ModifierKeyMonitor` CGEventTap logic from `UltraVox/ModifierKeyMonitor.swift`.
4. Implement `ultravox_macos_bridge_show_indicator` / `hide_indicator` using an
   `NSPanel` with `.nonactivatingPanel`, similar to
   `UltraVox/Indicator/IndicatorWindowManager.swift`.
5. Implement `ultravox_macos_bridge_transcribe_file` by integrating the
   `FluidAudioEngine` / Whisper engine path from
   `UltraVox/Engines/TranscriptionEngine.swift`.

## License

MIT (see repository root `/LICENSE`).
