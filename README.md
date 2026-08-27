# UltraVox Light

Private, on-device transcription for macOS. UltraVox Light is MIT-licensed open-source software: recordings, transcripts, and downloaded models stay on your device.

<p align="center">
  <a href="https://github.com/michael-berardi/ultravox-light/releases/latest"><strong>Download UltraVox Light</strong></a>
  ·
  <a href="#privacy"><strong>Privacy</strong></a>
  ·
  <a href="#build-from-source"><strong>Build from source</strong></a>
</p>

<p align="center">
  <img src="docs/image.png" alt="UltraVox Light ready to record" width="360" />
</p>

## Features

- On-device microphone transcription with English and multilingual models.
- Global recording shortcuts, including press-to-toggle and hold-to-record modes.
- Transcription from supported URLs and local audio files.
- Local history with search, copy, export, retry, and deletion controls.
- Five free themes: Midnight, Silver Rack, Nord Frost, Vapor, and Obsidian Rite.
- Public release updates verified with published SHA-256 checksums.
- Optional headless QA harness for deterministic UI checks.

## Requirements

- macOS on Apple silicon.
- Node.js 22, pnpm 10, Rust, and CMake.
- `yt-dlp` and `ffmpeg` for URL transcription (`brew install yt-dlp ffmpeg`).
- Accessibility permission enables caret targeting and automatic paste.

## Install

Download `UltraVox-Light-macos-arm64.zip` from the [latest UltraVox Light release](https://github.com/michael-berardi/ultravox-light/releases/latest), then verify its adjacent SHA-256 file.

## Quick start

1. Launch UltraVox Light.
2. Choose an English or multilingual model and download it once.
3. Set a global shortcut in **Settings → Shortcut**.
4. Press the shortcut, speak, then press it again to transcribe.
5. Use **Transcribe URL** for a supported HTTP(S) URL, or drop a local audio file onto the window.

## Privacy

Transcription runs locally. Audio and transcripts are not sent to a transcription service. URL transcription downloads audio through the local `yt-dlp` executable and then processes it with the selected on-device model. History and model caches use the app's local data directory.

UltraVox Light uses no analytics or hosted service credentials. Public updates use release assets and adjacent SHA-256 checksum files.

## Updates

UltraVox Light checks the public GitHub release metadata at launch and at most once per day. You may install a candidate manually or enable automatic installation in **Settings → Privacy**. Before installation, the app verifies the version, checksum, bundle identity, and downloaded archive. Failed verification leaves the installed version untouched.

## Build from source

```bash
git clone --recurse-submodules https://github.com/michael-berardi/ultravox-light.git
cd ultravox-light
brew install cmake libomp rust node@22
export PATH="$(brew --prefix node@22)/bin:$PATH"
npm install --global pnpm@10.27.0
pnpm install --frozen-lockfile

# Build UltraVox Light
pnpm desktop:build
```


For development and type checks:

```bash
pnpm desktop:dev
pnpm desktop:check
```

### Headless visual QA

```bash
pnpm desktop:qa:headless
```

The QA harness supports deterministic theme, drag/drop, model, transcription, and permission states without external services.

## Architecture

- **Tauri + React** provide the desktop shell and accessible interface.
- **Rust** handles recording, audio decoding, history, model downloads, updates, and CLI tooling.
- **Swift** bridges macOS microphone permissions, accessibility insertion, global shortcuts, and FluidAudio/CoreML transcription.
- **Whisper.cpp** remains available for local Whisper model workflows.

## CLI

The optional `ultravox-control` binary provides health, status, model-catalog, history, download, shortcut, audio-device, recording, transcription, import, clipboard, and caret diagnostics. Run it with no arguments to print the exact command usage.

## Contributing

Please do not include recordings, transcripts, credentials, private URLs, or generated build output in issues or pull requests. See `apps/desktop/LEGAL_NOTICES.md` for third-party attribution.
## License

UltraVox Light source and documentation are available under the [MIT License](LICENSE). Third-party rights remain with their respective owners.
