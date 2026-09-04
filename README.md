# UltraVox Light

Private, on-device transcription for macOS, Windows, and Linux. UltraVox Light is MIT-licensed open-source software: recordings, transcripts, and downloaded models stay on your device.

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
- Global recording shortcuts on macOS, plus in-app recording on every supported platform.
- Transcription from supported URLs and local audio files.
- A manual, local custom dictionary for preferred terms and spoken aliases, with a bundled [dictionary-review skill](#custom-dictionary) any AI agent can run locally.
- Local history with search, copy, export, retry, and deletion controls.
- Five free themes: Midnight, Silver Rack, Nord Frost, Vapor, and Obsidian Rite.
- Public release updates verified with published SHA-256 checksums and platform identity checks.

## Supported platforms

- macOS 14 or newer on Apple silicon.
- Windows 10 or 11 on x86_64.
- x86_64 Linux distributions compatible with the Ubuntu 22.04 WebKitGTK/GLIBC baseline.

## Install

Download the artifact for your system from the [latest UltraVox Light release](https://github.com/michael-berardi/ultravox-light/releases/latest), then verify it with the adjacent SHA-256 file.

| System | Release artifact |
| --- | --- |
| macOS | `UltraVox-Light-macos-arm64.zip` |
| Windows | `UltraVox-Light-windows-x86_64-setup.exe` |
| Linux | `UltraVox-Light-linux-x86_64.AppImage` |

Light and Pro deliberately share the same signed application identity, canonical install path, settings directory, and OS permission grants. Installing Pro over Light upgrades the edition without asking you to grant microphone, accessibility, or screen-capture access again.

## Quick start

1. Launch UltraVox Light.
2. Choose an English or multilingual model and download it once.
3. Set a global shortcut in **Settings → Shortcut**.
4. Press the shortcut, speak, then press it again to transcribe.
5. Use **Transcribe URL** for a supported HTTP(S) URL, or drop a local audio file onto the window.

## Custom dictionary

Open **Settings → Dictionary** to add preferred spellings. Use one entry per line:

```text
Retex = retext
UltraVox = Ultra Box
```

A line may contain only the canonical term, or `Canonical term = alias one, alias two`. Blank lines and lines beginning with `#` or `//` are ignored. Alias matching is case-insensitive, and conservative typo matching is limited to distinctive custom terms. Corrections happen locally before a transcript is saved, copied, or pasted. On Windows and Linux, canonical terms are also appended to the existing Whisper initial prompt.

The dictionary accepts up to 128 KiB and 512 canonical entries. Canonical and alias fields are limited to 128 bytes, with at most 16 aliases total for each canonical term.

This feature is manual in UltraVox Light. It does not scan Retex, contacts, files, or other apps for vocabulary.

### Review the dictionary with your AI agent

UltraVox bundles a dictionary-review skill for AI agents at `skills/dictionary-review/SKILL.md`, also installed inside the app under `Resources/skills/dictionary-review/SKILL.md`. Open **Settings → Dictionary → Agent dictionary review** to reveal the file or copy its contents, then add it to your agent yourself — UltraVox never modifies your agent. The skill audits local transcripts read-only, proposes only verified corrections, and verifies each one before finishing; everything stays on your device.

## Privacy

Transcription runs locally. Audio and transcripts are not sent to a transcription service. URL transcription downloads audio through the local `yt-dlp` executable and then processes it with the selected on-device model. History and model caches use the app's local data directory.

UltraVox Light uses no analytics or hosted service credentials. Public updates use immutable release assets and adjacent SHA-256 checksum files.

## Updates

UltraVox Light checks the public GitHub release metadata at launch and at most once per day. You may install a candidate manually or enable automatic installation in **Settings → Privacy**. Before installation, the app verifies the version and checksum; macOS also requires the canonical bundle identity, Developer ID signature, designated requirement, and stapled notarization ticket. Failed verification leaves the installed version untouched.

## Build from source

```bash
git clone --recurse-submodules https://github.com/michael-berardi/ultravox-light.git
cd ultravox-light
# Install the current platform's Tauri prerequisites first:
# https://v2.tauri.app/start/prerequisites/
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


## Architecture

- **Tauri + React** provide the desktop shell and accessible interface.
- **Rust** handles recording, audio decoding, history, model downloads, updates, and CLI tooling.
- **Swift/FluidAudio** provide native macOS transcription, permissions, and insertion.
- **Whisper.cpp** provides local transcription on Windows and Linux.

## CLI

The optional `ultravox-control` binary provides health, status, model-catalog, audio-device, and dictionary diagnostics. Run it with `--help` to print the exact command usage. Dictionary commands read the installed app's real `settings.toml` by default:

```bash
cargo run -p ultravox --features cli --bin ultravox-control -- dictionary-smoke
cargo run -p ultravox --features cli --bin ultravox-control -- dictionary-apply 'retext, Ultra Box!'
```

Use `ULTRAVOX_DATA_DIR` to point automated tests at an isolated settings directory. Inline entries remain available for a deterministic one-off check:

```bash
ULTRAVOX_DATA_DIR=/tmp/ultravox-test \
  cargo run -p ultravox --features cli --bin ultravox-control -- \
  dictionary-apply --dictionary $'Retex\nUltraVox = Ultra Box' 'retext, Ultra Box!'
# Retex, UltraVox!
```

## Contributing

Please do not include recordings, transcripts, credentials, private URLs, or generated build output in issues or pull requests. See `apps/desktop/LEGAL_NOTICES.md` for third-party attribution.

## Support and security

Use [GitHub Issues](https://github.com/michael-berardi/ultravox-light/issues) for reproducible bugs and feature requests. Report vulnerabilities privately through [GitHub Security Advisories](https://github.com/michael-berardi/ultravox-light/security/advisories/new); do not include credentials, private recordings, or transcripts in a public issue.

On macOS, a plain local Tauri bundle is ad-hoc signed and must not be installed over the canonical app when testing permission continuity. Accessibility, Microphone, and Screen Recording grants persist only across builds carrying the same stable Developer ID designated requirement; published builds additionally require notarization.

## License

UltraVox Light source and documentation are available under the [MIT License](LICENSE). Third-party rights remain with their respective owners.
