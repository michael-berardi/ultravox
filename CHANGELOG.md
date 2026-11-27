# Changelog

## 0.13.7 (2026-11-27)

### Privacy
- A minute after launch, UltraVox now sweeps expired leftovers that a crash or force quit can strand: update staging, Retex output and atomic-write temporaries older than an hour, and interrupted model downloads and development sandboxes older than a day. Only exact UltraVox-owned names are matched in folders you can choose (`ULTRAVOX_DATA_DIR`, a shared models folder), so other files there are never removed.
- Interrupted model downloads are named after the full model file (`ggml-base.bin.part`), so the sweep can match them exactly. Partials left by earlier builds (`ggml-base.part`) are swept too.
- [PRIVACY.md](PRIVACY.md) now lists every local write and how long it lasts, including webview storage and how to clear it.

## 0.13.6 (2026-11-26)

Privacy hardening and update contract v1 ([docs/updates.md](docs/updates.md)).

### Updates
- **Check for updates automatically** (Settings → Updates, on by default) now turns the release check itself off, not just installs. The check runs at most once every 24 hours and is one anonymous GET with the fixed User-Agent `ultravox-update-check`. It sends no app version. It runs in the background with a 4-second timeout and records the cause of a failure, shown in Settings → Updates.
- **Install updates automatically** is now off by default. A saved choice to turn it on is kept.
- New update notice under the top bar when a check finds a newer release: "UltraVox <new> is available — you have <current>." with short release notes, **Install Update**, **Remind Me Later**, **Skip This Version** and an unchecked "Automatically download and install updates in the future" box. It does not block the window, shows at most once a day per version, and a skipped version stays quiet until a newer one exists.
- Settings → Updates is compact: one switch per row without captions, then the version on the left and **Check Now** on the right of the same row, with one result line after a check.
- Update downloads go to a temporary folder that is removed on success and on every failure, and the rollback copy of the previous version is removed once the new version has run for a minute.
- The private QA feed override accepts only plain-HTTP loopback URLs (no other host or credentials); consumer builds still ignore it.
- No update request runs with `ULTRAVOX_NO_UPDATE_CHECK=1`, in CI, under an agent host (`CLAUDECODE`, `CODEX_SANDBOX`, `GEMINI_CLI`) or in development mirror builds. An unreadable preference file keeps automatic checks off.
- Official Pro builds: the authenticated Pareto update channel is consulted only on **Check Now**, never by automatic checks, and is covered by the same opt-outs.
- Checksum, Developer ID, designated-requirement and notarization verification are unchanged.

### Privacy
- On Unix, the default app data directory and the recordings, models and Voice Studio directories are 0700. Settings, history, audio, voice and update files are 0600. A directory chosen with `ULTRAVOX_DATA_DIR` is used as-is, but UltraVox still applies 0700 to directories it uses there and 0600 to files it creates; symlinked directories keep their own permissions.
- Startup loads an installed model and never downloads one. Phonon-2 is fetched only after you pick an English model that is not installed, or click Download.
- `yt-dlp` media imports run with `--no-cache-dir`.
- Build and packaging scripts set `DO_NOT_TRACK=1` and `HOMEBREW_NO_ANALYTICS=1`.
- New [PRIVACY.md](PRIVACY.md) lists every network request, including the official build's licensing check, and every local store, with how to delete it. The README Privacy section is a five-line summary that links to it.

### Export
- `Scripts/export-open-source.sh` now fails an export that drops `docs/updates.md` (with its marker), `PRIVACY.md`, `CHANGELOG.md`, `private_fs.rs` or the gated update check.
