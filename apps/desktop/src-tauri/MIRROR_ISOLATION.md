# Isolated mirror-debug launch

Create an empty, private sandbox directory before launching, then launch the mirror-debug executable with `ULTRAVOX_DATA_DIR=/tmp/ultravox-ui-sandbox`. The directory must already exist; the application fails closed if missing or if its canonical path overlaps the production app-data root. Non-mirror builds also honor the override, preserving the CLI convention. Default config/history stores are initialized empty; Voice Studio initializes its empty store on demand. No recordings are copied.

Settings, history, recordings, model defaults, telemetry preferences, update preferences and Voice Studio all resolve under that root. Use a fresh directory, not copied production settings or symlinked contents. Mirror builds skip single-instance registration, hotkeys, voice IPC, telemetry startup tasks, transcription warmup, Retex refresh and tray setup. All renderer invokes use the mirror allowlist, denying capture, media control/polling and update checks even from the main renderer.

The mirror control socket is `<ULTRAVOX_DATA_DIR>/mirror-control/mirror.sock`, NOT `~/.ultravox/mirror.sock`; clients must connect to the sandbox endpoint. Normal exit removes only the endpoint this instance successfully bound. Mirror close does not delete the app's data root.

Current limitation: corpus reads are sandbox-only, including when `ULTRAVOX_MIRROR_READ_REAL_RECORDINGS=1`; optional fresh-sandbox read-only production fallback is not implemented. This deliberately fails closed rather than opening production history via its writable constructor.

## Quality investigation

`PocketTtsEngine` in `crates/ultravox-macos-bridge/native/UltraVoxMacOSBridge/Sources/UltraVoxMacOSBridge/UltraVoxMacOSBridge.swift` caches managers by language/precision and initializes only when unavailable. Each cloned synthesis calls `loadClonedVoice` (line 2975 at inspection); the bridge itself does not explicitly reload Mimi. Proving that FluidAudio reloads the encoder requires inspecting its implementation; no bridge change was made. Separate CLI invocations cannot reuse this process-local manager cache, so sequential CLI timings are cold-process measurements even if downloaded models and OS disk caches are warm.

Core leading trim now uses 0.02 amplitude instead of 0.01 and removes short quiet lead-ins too; the quiet-leading-hum regression fixture passes. Real-corpus clone/speak/transcribe accuracy and two-call timing remain unmeasured in this worker: production Library reads and `/tmp` fixture writes were outside its exact filesystem permissions.
