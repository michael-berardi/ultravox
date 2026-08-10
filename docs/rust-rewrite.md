# UltraVox Rust / Tauri Rewrite

This directory contains the scaffold for the Rust-first, Tauri v2 desktop rewrite of UltraVox. The existing SwiftUI macOS implementation in the repository root and `UltraVox/` is preserved as the verified baseline and remains the production app until the rewrite is complete.

## Repository Layout

```
/                                 # Repository root
├── UltraVox/             # SwiftUI baseline app source
├── UltraVox.xcodeproj/   # Xcode project for the Swift baseline
├── UltraVoxTests/        # Swift baseline unit tests
├── UltraVoxUITests/      # Swift baseline UI tests
├── apps/
│   └── desktop/                  # Rust / Tauri v2 rewrite
│       ├── src/                  # React + TypeScript frontend
│       ├── src-tauri/            # Rust backend and Tauri configuration
│       └── LEGAL_NOTICES.md      # License and third-party notices
├── docs/
│   └── rust-rewrite.md           # This file
└── Readme.md                     # Product readme (architecture note)
```

## Why a Separate `apps/desktop` Directory?

- **Preserves the Swift baseline.** Nothing in `UltraVox/`, `UltraVox.xcodeproj/`, or the existing Xcode tests is deleted or modified.
- **Allows parallel work.** The Swift app can still be built, signed, and released while the Rust rewrite progresses.
- **Matches the long-term architecture.** The Readme states that the core transcription pipeline will eventually be extracted into Rust and wrapped in a Tauri shell. `apps/desktop` is that shell.
- **Keeps tooling isolated.** `pnpm`/`npm`, `cargo`, and `vite` live under `apps/desktop` without polluting the Swift build.

## Current Scope of the Rewrite

The scaffold is intentionally a thin, buildable shell. It includes:

- Tauri v2 with a React + TypeScript frontend.
- Product name `UltraVox` and bundle identifier `com.imploselabs.ultravox`.
- A macOS menu-bar-capable tray icon configuration.
- A main UltraVox window (450 × 650 px, matching the Swift baseline dimensions).
- A settings view with placeholders for the four baseline tabs:
  - **Shortcut** — recording trigger and shortcut capture.
  - **Model** — model choice (English default, Multilingual optional) and download directory.
  - **Transcription** — language, output, and clipboard settings.
  - **Advanced** — decoding strategy, parameters, and debug options.
- Basic Rust IPC commands:
  - `get_app_info` — returns `{ name, version, identifier }`.
  - `get_app_status` — returns `{ status: "ready" | "loading" | "error" }`.
- Legal notices in `apps/desktop/LEGAL_NOTICES.md`.

## Model Choice UI

The UI exposes only two user-facing choices:

- **English** — default, English-only, higher accuracy.
- **Multilingual** — supports 25+ languages.

No specific Whisper or FluidAudio model names are shown in the UI. This matches the Swift baseline onboarding and settings model picker.

## Building

Install dependencies:

```bash
cd apps/desktop
pnpm install
```

Run TypeScript checks:

```bash
pnpm check
```

Run Rust checks:

```bash
cd src-tauri
cargo check
```

Build the Tauri app (from the repository root):

```bash
pnpm desktop:build
```

Do not launch the foreground UI from automated builds unless explicitly requested.

## Migration Plan (Future Milestones)

1. **Phase 1 — Shell (this scaffold):** buildable app with settings UI and IPC plumbing.
2. **Phase 2 — Audio pipeline:** Rust audio capture, file queue, and playback.
3. **Phase 3 — Transcription:** integrate whisper.cpp or an equivalent Rust inference backend.
4. **Phase 4 — Parity:** global shortcuts, tray microphone/language menu, clipboard/paste, and Asian autocorrect.
5. **Phase 5 — Cutover:** replace the Swift baseline with the Rust app for releases.

Until Phase 5, the Swift baseline remains the canonical app.

## License

See `LICENSE` and `apps/desktop/LEGAL_NOTICES.md`.
