# UltraVox

UltraVox is a private, on-device macOS transcription app for microphone recordings, meeting capture, and supported media URLs.

<p align="center">
  <img src="docs/image.png" alt="UltraVox ready to record" width="360" />
  <img src="docs/image_indicator.png" alt="UltraVox on-device model settings" width="360" />
</p>

## Features

- On-device transcription with downloadable English and multilingual models.
- Microphone recording with configurable global shortcuts.
- Hold-to-record mode: hold the shortcut to record, then release to stop.
- Meeting mode for repeated capture, with each segment queued for transcription.
- Optional Google Meet and Zoom reminders from the local [OverSeer Browser](https://github.com/michael-berardi/overseer-browser) extension. UltraVox receives a minimized local event containing a schema version, random detection ID, provider name, opaque meeting key, and millisecond detection time, then waits for explicit recording consent.
- Supported media URL import and queued processing.
- Language auto-detection for supported models.

## Architecture

The canonical app is a Rust/Tauri v2 desktop application backed by the shared `ultravox-core` crate. The original SwiftUI implementation remains in `UltraVox/` as a reference during the migration. Current development, build, and release workflows target the Tauri app.

UltraTerm connects through UltraVox's per-user Unix socket. Both apps use
`$TMPDIR/com.imploselabs.ultravox/voice-v1.sock` by default. Set
`ULTRAVOX_VOICE_SOCKET` in both app environments only when a custom path is
required.

## Installation

Prebuilt Apple Silicon releases include `UltraVox.app` and the
`ultravox-control` CLI. Every release is Developer ID signed for team
`T63VT9UAY2`, notarized, and published with SHA-256 checksums. The package
installer verifies the bundle identifier, designated requirement, sealed code,
signature team, and checksum before installing.

UltraVox has one canonical install and update location:
`/Applications/UltraVox.app`. Use the signed `.pkg` from
[GitHub Releases](https://github.com/michael-berardi/ultravox/releases):

```bash
pkg=UltraVox-macos-arm64.pkg
base=https://github.com/michael-berardi/ultravox/releases/latest/download
release_url="$(curl -fsSI "$base/$pkg" | awk 'tolower($1) == "location:" { sub(/\r$/, "", $2); print $2; exit }')"
expected_version="${release_url#*/download/}"
expected_version="${expected_version%%/*}"
expected_version="${expected_version#v}"
[[ "$expected_version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]
curl -fL -o "$pkg" "$base/$pkg"
curl -fL -o "$pkg.sha256" "$base/$pkg.sha256"
shasum -a 256 -c "$pkg.sha256"
pkgutil --check-signature "$pkg" | grep -F "Developer ID Installer: Michael Berardi (T63VT9UAY2)"
spctl --assess --type install --verbose=2 "$pkg"
sudo installer -pkg "$pkg" -target /

app=/Applications/UltraVox.app
test "$(/usr/libexec/PlistBuddy -c 'Print :CFBundleIdentifier' "$app/Contents/Info.plist")" = com.imploselabs.ultravox
test "$(/usr/libexec/PlistBuddy -c 'Print :CFBundleShortVersionString' "$app/Contents/Info.plist")" = "$expected_version"
codesign --verify --deep --strict --verbose=2 "$app"
codesign -dvv "$app" 2>&1 | grep -F "TeamIdentifier=T63VT9UAY2"
codesign -d -r- "$app" 2>&1 | grep -F 'identifier "com.imploselabs.ultravox" and anchor apple generic'
codesign -d -r- "$app" 2>&1 | grep -E 'certificate leaf\[subject\.OU\] = "?T63VT9UAY2"?'
spctl --assess --type execute --verbose=2 "$app"
```

Per-user app installs are intentionally unsupported. Updates replace only the
verified `/Applications/UltraVox.app` candidate; failed verification leaves the
existing app untouched.

## Requirements

- macOS (Apple Silicon/ARM64)

## Support

If you encounter any issues or have questions, please:
1. Check the existing issues in the repository
2. Create a new issue with detailed information about your problem
3. Include system information and logs when reporting bugs

## Building locally

Requirements: macOS on Apple Silicon, Node.js 22, pnpm 10.27.0, Rust, CMake, and libomp.

```bash
git clone https://github.com/michael-berardi/ultravox.git
cd ultravox
git submodule update --init --recursive
brew install cmake libomp rust node@22
npm install --global pnpm@10.27.0
pnpm install --frozen-lockfile
./run.sh build
```

## Publishing a release

Releases are built locally, Developer ID signed, notarized, checksum-staged,
and published without GitHub Actions:

```bash
export APPLE_SIGNING_IDENTITY=\"Developer ID Application: …\"
export APPLE_INSTALLER_SIGNING_IDENTITY=\"Developer ID Installer: …\"
export NOTARYTOOL_PROFILE=\"your-keychain-profile\"
pnpm release:publish
```

`pnpm release:package` only stages the `.zip`, `.pkg`, and checksum assets in
`release/`. Publishing additionally requires an authenticated GitHub CLI.


## Contributing

Contributions are welcome! Please feel free to submit pull requests or create issues for bugs and feature requests.

## License

UltraVox is licensed under the MIT License. The root [LICENSE](LICENSE) preserves the OpenSuperWhisper notice and covers Implose Labs modifications.

This project is based on [OpenSuperWhisper](https://github.com/Starmel/OpenSuperWhisper) by Starmel.

## Credits & Third-Party Software

UltraVox includes or depends on the following open-source projects:

- [OpenSuperWhisper](https://github.com/Starmel/OpenSuperWhisper) — MIT License
- [Whisper.cpp](https://github.com/ggerganov/whisper.cpp) — MIT License
- [FluidAudio](https://github.com/FluidInference/FluidAudio) — used under its respective license
- [autocorrect](https://github.com/huacnlee/autocorrect) — used under its respective license

All third-party licenses and notices remain the property of their respective owners.

## Privacy and models

Transcription runs on-device. After the selected model is downloaded from Hugging Face, microphone recording and meeting capture can be transcribed offline. Media URL import still requires network access to download the source.

Browser meeting reminders are opt-in and local. The extension does not send UltraVox meeting URLs, titles, participants, or page contents, and UltraVox never starts recording until **Start recording** is selected in its visible reminder.

### Optional telemetry

Telemetry is off by default and is requested in a first-run consent dialog
before model onboarding. If enabled, UltraVox sends only coarse launch,
heartbeat, and usage aggregates to
`https://analytics.libertydesign.studio/api/app-telemetry/event` using schema
`lds.app-telemetry.event.v2`. Payloads contain a random installation UUID
created after acceptance, the app version, coarse platform and architecture,
the UTC day, and fixed bounded integer counters. They never contain
transcripts, audio, prompts, recordings, meeting IDs, URLs, titles, providers,
participants, paths, keys, shortcuts, errors, stacks, or host/user/device IDs.
Disabling telemetry clears the random install identifier and queued events;
declining persists without creating an identifier. Raw identifier-bearing
events are retained for 34 days and identifier-free daily aggregates for 360
days.

Offline queued counters are merged into the next successful current-day
heartbeat. Each usage delivery carries a lowercase UUIDv4 `batchId`; retries
reuse the same batch identifier and counters, while launch and heartbeat
events never include `batchId`. Counters remain bounded by the same allowlist.

### Updates

UltraVox checks the latest stable GitHub release at launch and approximately
once per day. Updates are offered as **Update now**, **Later**, or
**Install automatically**. Automatic installation is opt-in. Candidates are
staged atomically and must pass checksum, Developer ID team,
bundle/designated-requirement, sealed-code, and notarization checks before the
existing `/Applications/UltraVox.app` is touched.
