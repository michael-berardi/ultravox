# UltraVox macOS Bridge

The macOS bridge provides the native capabilities used by the Rust/Tauri desktop app. A SwiftPM static library exposes a small C ABI; the Rust crate wraps that ABI with typed results, validation, and explicit string ownership.

## Layout

```text
crates/ultravox-macos-bridge/
├── Cargo.toml
├── build.rs
├── src/lib.rs
└── native/UltraVoxMacOSBridge/
    ├── Package.swift
    ├── Sources/UltraVoxMacOSBridge/
    │   ├── UltraVoxMacOSBridge.swift
    │   └── include/UltraVoxMacOSBridge.h
    └── Tests/UltraVoxMacOSBridgeTests/
```

## Capabilities

- FluidAudio/Core ML transcription and model preparation
- Microphone and Screen Recording permission checks
- Accessibility trust, caret positioning, and text insertion
- Global recording and meeting shortcuts
- Native recording-status indicator windows
- ScreenCaptureKit meeting audio capture
- Capture-free CoreAudio process activity and output volume/mute control
- Audio-only ScreenCaptureKit system-output spectrum metering for reactive visuals
- Optional runtime MediaRemote metadata and supported system-session controls

MediaRemote is loaded dynamically and treated as optional. Missing metadata returns unavailable values rather than fabricated results; transport commands still route to the canonical system session. Source detection never captures PCM. The reactive spectrum starts only while visible, excludes UltraVox audio, and never captures the microphone.

## Build and test

The Rust build script compiles the Swift package automatically on macOS. The Swift package can also be tested directly:

```bash
cd crates/ultravox-macos-bridge/native/UltraVoxMacOSBridge
swift test
```

From the repository root:

```bash
cargo test -p ultravox-macos-bridge
```

## License

MIT — see the repository root [`LICENSE`](../../LICENSE).
