# Privacy

UltraVox sends no telemetry, analytics or crash reports. Transcription and dictionary processing run on your device. The open-source build sends no install or device identifier. Official builds send the random install ID described under [Pro and licensing](#pro-and-licensing), and use it only for licensing.

## Network traffic, and when it happens

- **Update check:** at most once every 24 hours, one anonymous HTTPS GET to `api.github.com/repos/michael-berardi/ultravox/releases` with the fixed User-Agent `ultravox-update-check`. It sends no app version, account, install ID or machine data. GitHub sees your IP address, as with any HTTPS request. The check runs in the background with a 4-second timeout and never delays startup. If it fails, the cause is shown in **Settings → Updates**. To turn it off, use **Settings → Updates → Check for updates automatically** (on by default) or set `ULTRAVOX_NO_UPDATE_CHECK=1`. No check runs when `CI` is set, under an agent host, or in development mirror builds. **Check Now** checks once when you click it. Details: [docs/updates.md](docs/updates.md).
- **Update download:** only when you click **Install Update** (or **Update** in the top bar), or when you turned on **Install updates automatically** (off by default) and the app is idle. The app fetches the release asset and its SHA-256 manifest from `github.com/michael-berardi/ultravox/releases` with the User-Agent `ultravox-updater/<version>`, and verifies the checksum before installing. The host sees your IP address.
- **Models:** only when you click **Download** for a model, or when you switch to an English model that is not installed yet (UltraVox then fetches Phonon-2). Phonon-2 comes from `huggingface.co/FermionResearch/Phonon-2` and is checked against pinned SHA-256 hashes. The native bridge or the HTTP downloader fetches multilingual model packs from Hugging Face. No checksum is pinned for those packs. Startup loads an installed model and never downloads one. In official Pro builds on macOS, Voice Studio downloads its speech models from Hugging Face the first time you use it.
- **Media URL import:** only when you paste a URL and start an import. The `yt-dlp` tool you installed connects to the site you named and its media hosts. Those hosts see your IP address and the URL you entered. UltraVox runs `yt-dlp` with `--no-cache-dir` and never sends your recordings or transcripts to those hosts.
- <a id="pro-and-licensing"></a>**Pro and licensing:** the open-source build has locked Pro stubs and makes no license, entitlement or private-update requests. Official builds include the Pro module. It contacts `software.paretocybernetics.com` at launch and then every 12 hours to refresh the license, or this install's trial state. Those requests carry a random install ID and a signature from a key that UltraVox generates on first launch. After you activate a license, they also carry your access key. They carry no recordings, transcripts, dictionary text or usage data. In a licensed official build, **Check Now** may also ask the authenticated Pareto update channel on the same host if the public feed has no update or cannot be reached. That request is signed and includes the running version. Automatic checks never use it.

## Your data

Your data lives in the Tauri application data directory for `com.imploselabs.ultravox` (`~/Library/Application Support/com.imploselabs.ultravox/` on macOS). You can override it with `ULTRAVOX_DATA_DIR`. UltraVox keeps it until you delete it:

| What | Where | Kept |
| --- | --- | --- |
| Settings, shortcuts, the manual dictionary, vault permissions | `settings.toml` | Until you delete it |
| History: timestamps, titles, previews, full transcripts, local source paths | `recordings.sqlite` | Until you delete it |
| Recorded audio | `recordings/` | Until you delete the recording |
| Downloaded models | `models/` (or the folder you chose) | Until you delete them; superseded English models are removed |
| Update choices: the two switches, a skipped version, when the notice was last shown | `update-preferences.json` | Until you delete it |
| Voice Studio profiles, voice embeddings, reference audio, generated speech (official Pro builds) | `voice-studio/` | Until you delete them in Voice Studio |
| Install ID, its signing key, access key, license token (official builds) | System credential store (the Keychain on macOS), not files | Until you remove them there |

On Unix, the default app data directory is 0700, so everything inside it is private to you. The recordings, models and Voice Studio directories are 0700 too. Settings, history, audio, voice and preference files are 0600. A directory you choose with `ULTRAVOX_DATA_DIR` is used as-is. UltraVox still sets 0700 on the application directories it uses there and 0600 on files it creates there. Symlinked directories keep their own permissions. Other platforms use their native per-user protections.

## Everything else UltraVox writes, and how long it lasts

None of these files is your data. Each one is removed on the normal path. As a backstop for a crash or a force quit, normal builds sweep leftovers older than their limit once the app has been running for a minute, so they clean this local sandbox of expired leftovers. Development `mirror-debug` builds skip startup services, including this sweep; they are disposable QA builds, not installed releases.

| Write | Where | Lifetime |
| --- | --- | --- |
| Last update check: time, result, cause of a failure | `update-check.json` in the cache directory (`~/Library/Caches/com.imploselabs.ultravox/` on macOS, or `ULTRAVOX_DATA_DIR` when set) | One small 0600 file, replaced by each check |
| One-time legacy-migration notice | `legacy-migration-v1.json` beside the settings file | Kept until you delete the app data directory; the migration retry rewrites it in place |
| Atomic-write temporaries (`settings.toml.tmp-*`, `update-preferences.json*.tmp`, `update-check.json*.tmp`, `legacy-migration-v1.json.migrate.tmp`, `settings.toml.migrate.tmp`, `update-preferences.json.migrate.tmp`, `update-check.json.migrate.tmp`, `voices.*.tmp`) | Beside the file they replace | Renamed into place right away; swept after 1 hour. Only those exact `.migrate.tmp` names are swept, because you can choose the data directory: any other `*.migrate.tmp` file there is left alone. Deeper migration leftovers are retried by that one-time migration at the next launch |
| Update download staging (`ultravox-update-<id>/`) | System temporary directory | Deleted on every outcome (success, failure, refusal); swept after 1 hour |
| Rollback copy of the previous version (`.UltraVox.previous-<pid>.app`, or `<name>.AppImage.previous` on Linux) | Beside the installed app | Deleted once the new version is healthy, and again after it has run for 1 minute |
| Older Light/Pro edition bundles (`UltraVox Light.app`, `UltraVox Pro.app`) | `/Applications`, `~/Applications` | Moved to the Trash at launch when not running |
| Interrupted UltraVox model downloads (`phonon-2.bps.tar.zst.part`, `phonon-2.partial`, `fluidaudio-multilingual-v3.bin.part`, `ggml-base.bin.part`, and `fluidaudio-multilingual-v3.part` / `ggml-base.part` from builds before 0.13.7) | `models/`, including a folder you chose and share | Replaced by a finished download; only these exact UltraVox staging names are swept after 24 hours. Other `.part`, `.partial` and `.tmp` files in a shared folder are left alone |
| Media import working files | `recordings/.imports/` | Deleted after each import and at every launch |
| Retex output (`ultravox-retex-<id>.stdout` / `.stderr`, Pro on macOS) | System temporary directory | Deleted after each vocabulary refresh; swept after 1 hour |
| Voice socket and its owner file | Per-user temporary directory (`com.paretocybernetics.ultravox/`) | While UltraVox runs; removed on quit and replaced at the next launch |
| WebView website data (local storage, cookies and web cache) | Tauri's webview data for identifier `com.imploselabs.ultravox`: `~/Library/WebKit/com.imploselabs.ultravox/` on macOS (WebKit), or `EBWebView` under `%LOCALAPPDATA%\com.imploselabs.ultravox\` on Windows (WebView2) | Managed by the webview, not by UltraVox's sweep. Persists across launches until the webview expires it or you delete the folder; UltraVox writes settings and transcripts to the files above, not to webview storage |
| `ultravox-control` smoke-test scratch (`ultravox-import-smoke-*.wav`, `ultravox-control-models-*/`, `ultravox-control-shortcut-*/`) | System temporary directory | The `.wav` is deleted when the command finishes, and swept after 1 hour. The folders are kept for inspection and swept after 24 hours. |
| Development mirror sandbox (`ultravox-mirror-<id>/`, mirror-debug builds only) | System temporary directory | Deleted on close; swept after 24 hours |

UltraVox writes no log file. Errors go to the app's own stderr, and never include transcripts or secrets.

## Dictionary and Retex

Manual dictionary text never leaves the device. On macOS with Pro unlocked, UltraVox does not inspect an Obsidian vault directly. After you complete the **Settings → Dictionary** permission wizard, it runs Retex 1.2.0 or newer read-only as `retex vocabulary --vault <selected> --limit 10000 --raw-json`. It then ranks those bounded candidates locally and keeps at most 256 terms. Retex deterministically scans local records and returns only bounded canonical terms and aggregate counts. UltraVox never receives raw note bodies, excerpts or paths. UltraVox ranks the returned terms against recent on-device transcript text, so likely mishearings are kept. Transcript text is never passed to Retex or uploaded. Removing a selected vault revokes permission and clears generated vocabulary; manual entries remain. Meeting capture and Voice Studio exist only in official Pro builds. Voice Studio storage is described in [docs/voice-studio.md](docs/voice-studio.md).

## Deleting it

- History → **Delete all** removes recordings, transcripts and their audio from the app.
- Voice Studio deletes a voice and its embedding when you delete it there.
- To remove everything else, quit UltraVox and delete the application data directory. That includes settings, update preferences, Voice Studio data and models. Also delete `update-check.json` from the cache directory.
- Pro credentials stay in the system credential store until you remove them there.
- To clear webview data too, quit UltraVox and delete `~/Library/WebKit/com.imploselabs.ultravox/` on macOS, or the `EBWebView` folder under `%LOCALAPPDATA%\com.imploselabs.ultravox\` on Windows. The webview recreates its store when needed; UltraVox's sweep does not delete it.
- Exported files, backups and files you supplied for import live outside these directories. Delete them separately.
