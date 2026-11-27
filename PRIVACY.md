# Privacy

UltraVox sends no telemetry, analytics, or crash reports. Transcription and dictionary processing run on your device. The open-source build sends no install or device identifier; official builds send the random install ID described under [Pro and licensing](#pro-and-licensing), for licensing only.

## Network traffic, and when it happens

- **Update check:** at most once every 24 hours, one anonymous HTTPS GET to `api.github.com/repos/michael-berardi/ultravox/releases` with the fixed User-Agent `ultravox-update-check`. It sends no app version, account, install ID, or machine data; GitHub sees your IP address, as with any HTTPS request. It runs in the background with a 4-second timeout and never delays startup; a failure is recorded with its cause in **Settings → Updates**. **Settings → Updates → Check for updates automatically** (on by default) or `ULTRAVOX_NO_UPDATE_CHECK=1` turns it off. No check runs when `CI` is set, under an agent host, or in development mirror builds. **Check Now** checks once when you click it. Details: [docs/updates.md](docs/updates.md).
- **Update download:** only when you click **Install Update** or the top-bar **Update** button, or when you turned on **Install updates automatically** (off by default) and the app is idle. The app fetches the release asset and its SHA-256 manifest from `github.com/michael-berardi/ultravox/releases` with the User-Agent `ultravox-updater/<version>` and verifies the checksum before installing. The host sees your IP.
- **Models:** only when you click **Download** for a model, or when you switch to an English model that is not installed yet (UltraVox then fetches Phonon-2). Phonon-2 is fetched from `huggingface.co/FermionResearch/Phonon-2` and checked against pinned SHA-256 hashes. Multilingual model packs are fetched from Hugging Face by the native bridge or the HTTP downloader; this source pins no checksum for those packs. Startup loads an installed model and never downloads one. In official Pro builds on macOS, Voice Studio downloads its speech models from Hugging Face the first time you use it.
- **Media URL import:** only when you paste a URL and start an import. The `yt-dlp` tool you install connects to the site you named and its media hosts; those hosts see your IP and the URL you entered. UltraVox runs it with `--no-cache-dir`. UltraVox does not send your recordings or transcripts to them.
- <a id="pro-and-licensing"></a>**Pro and licensing:** the open-source build has locked Pro stubs and makes no license, entitlement, or private-update requests. Official builds include the Pro module, which contacts `software.paretocybernetics.com` at launch and then every 12 hours to refresh the license or this install's trial state. Those requests carry a random install ID and a signature from a key that UltraVox generates on first launch, plus your access key once you activate a license. They carry no recordings, transcripts, dictionary text, or usage data. In a licensed official build, **Check Now** may also ask the authenticated Pareto update channel on the same host when the public feed has no update or cannot be reached; automatic checks never use it.

## What stays local

Data lives in the Tauri application data directory for `com.imploselabs.ultravox` (override with `ULTRAVOX_DATA_DIR`).

- `settings.toml` holds settings, shortcuts, the manual dictionary, and any vault permissions you granted.
- `recordings.sqlite` holds timestamps, titles, previews, full transcripts, and local source paths; `recordings/` holds the audio.
- `models/` holds downloaded models.
- `update-preferences.json` holds your update choices: the two **Settings → Updates** switches, a version you skipped, and which version the update notice last showed and when.
- In official Pro builds, `voice-studio/` holds Voice Studio profiles, voice embeddings, reference audio, and generated speech. The install ID, its signing key, the access key, and the license token are kept in the system credential store (the Keychain on macOS), not in files.
- `update-check.json` in the platform cache directory (`~/Library/Caches/com.imploselabs.ultravox/` on macOS, or inside `ULTRAVOX_DATA_DIR` when set) holds the time and result of the last update check.
- An update download goes to a temporary folder that is removed whether the install succeeds or fails. The previous app version is kept only until the new one has run for a minute, then removed.
- The voice socket in the user temp directory lets paired local tools start dictation and read the resulting transcript; it is a local Unix socket, not a network service.

On Unix, the default app data directory is set to 0700, which keeps everything inside it private to you; the recordings, models, and Voice Studio directories are 0700 too, and settings, history, audio, voice, and preference files are 0600. A directory you choose with `ULTRAVOX_DATA_DIR` is used as-is, but UltraVox still sets 0700 on the application directories it uses there and 0600 on files it creates there; symlinked directories keep their own permissions. Other platforms use their native per-user protections.

Errors are printed to the app's own stderr; UltraVox writes no log file with transcripts or secrets. History is kept until you delete it; transient import files are removed after import and at startup.

## Deleting it

History → **Delete all** removes recordings, transcripts, and their audio from the app. Voice Studio deletes a voice and its embedding when you delete it there. Deleting the application data directory removes everything else, including settings, update preferences, Voice Studio data, and models; delete `update-check.json` from the cache directory as well. Pro credentials stay in the system credential store until you remove them there. Exported files, backups, and files you supplied for import are outside those directories and must be deleted separately.

## Dictionary and Retex

Manual dictionary text never leaves the device. On macOS with Pro unlocked, UltraVox does not inspect an Obsidian vault directly: it invokes Retex 1.2.0 or newer read-only as `retex vocabulary --vault <selected> --limit 10000 --raw-json`, locally ranks those bounded candidates, retains at most 256 terms, and only does so after the user completes the **Settings → Dictionary** permission wizard. Retex deterministically scans local records and returns only bounded canonical terms and aggregate counts; UltraVox never receives raw note bodies, excerpts, or paths. UltraVox ranks returned terms against recent on-device transcript text so likely mishearings are retained, but transcript text is never passed to Retex or uploaded. Removing a selected vault revokes permission and clears generated vocabulary; manual entries remain. Meeting capture and Voice Studio exist only in official Pro builds; Voice Studio storage is described in [docs/voice-studio.md](docs/voice-studio.md).
