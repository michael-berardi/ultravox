# Updates

<!-- update-contract: v1 -->

UltraVox tells you when a newer stable release exists and installs it in one click. Installing on its own is an opt-in.

## The update notice

When the daily check finds a newer release, a notice opens under the window's top bar. It does not block the window: recording, history and Settings keep working underneath.

- **UltraVox <new> is available — you have <current>.** and the release notes (plain text, scrollable, at most 600 characters).
- **Install Update** downloads, verifies, installs and relaunches. Nothing is installed while a recording, meeting, transcription or import is running.
- **Remind Me Later** closes the notice. The top-bar **Update** button stays, and the notice returns after 24 hours.
- **Skip This Version** hides the notice and the **Update** button for that version. A newer release is offered again. **Check Now** still offers a skipped version, because you asked.
- **Automatically download and install updates in the future** (unchecked) turns on **Install updates automatically**. It does not install the waiting update; **Install Update** does.

The notice shows at most once a day per version. With **Install updates automatically** on, no notice appears: the update installs while UltraVox is idle.

## Settings → Updates

| Control | Default | What it does |
| --- | --- | --- |
| **Check for updates automatically** | On | At most once every 24 hours, one anonymous request for the public release list. |
| **Install updates automatically** | Off | When a check finds a newer release, install it while UltraVox is idle, then relaunch. A saved choice to turn it on is kept. |
| **Check Now** | — | Checks once, right away, even when automatic checks are off. |

The last row shows the installed version on the left and **Check Now** on the right. After a check, one line below it gives the result (`UltraVox <new> is available — you have <current>.`, **UltraVox is up to date.** with the time of the check, or the cause of a failure).

**Check Now** is disabled while an environment block is active (`ULTRAVOX_NO_UPDATE_CHECK=1`, CI, an agent host, or a development mirror build). The panel shows the reason instead of sending a request. This is the one remaining difference from the update contract.

## The check

- **What is sent:** one HTTPS `GET https://api.github.com/repos/michael-berardi/ultravox/releases?per_page=100` with the fixed User-Agent `ultravox-update-check`. No app version, account, install ID, machine data or usage is sent. GitHub sees your IP address, as with any HTTPS request.
- **Why the list, not `/releases/latest`:** the public latest pointer is deliberately pinned to v0.13.3, the temporary identity bridge for older updaters, so it is not a download target for current users. The app reads the release list instead and selects a stable release that is newer than the installed version and carries this platform's asset and `.sha256` manifest.
- **How often:** at most once per 24 hours per user account. The time of the last check, the update it found and the cause of any failure are kept in `update-check.json` in the platform cache directory (`~/Library/Caches/com.imploselabs.ultravox/` on macOS). The file is written atomically with mode 0600. With `ULTRAVOX_DATA_DIR` set, the file lives in that directory instead.
- **Never in the way:** the check runs in the background after the window opens, with a 4-second timeout. A failure is recorded with its cause and shown in Settings → Updates. It is not shown as a popup, and the app does not retry it in a loop. The next attempt comes after the 24-hour interval or when you click **Check Now**.
- **Which releases count:** only published, stable `x.y.z` releases that are strictly newer than the running version and carry this platform's asset and its `.sha256` manifest. Drafts, prereleases and unparseable tags are ignored.

### When no check runs

- **Check for updates automatically** is off. **Check Now** still works.
- `ULTRAVOX_NO_UPDATE_CHECK=1` is set in UltraVox's environment. This turns off every update request, including **Check Now**.
- `CI` is set, or UltraVox was started by an agent host that marks its processes (`CLAUDECODE`, `CODEX_SANDBOX`, `GEMINI_CLI`).
- The build is a development mirror build (`mirror-debug`).
- The update preference file cannot be read. Automatic checks stay off until you choose again in Settings.

### Official Pro builds

Automatic checks use only the anonymous public feed. In an official build with an activated license, **Check Now** may also ask the authenticated Pareto update channel (`software.paretocybernetics.com`) when the public feed has no update or cannot be reached. That request is signed with the license's install credentials and includes the running version. The same environment opt-outs apply to it. The open-source build has no such channel.

## Install safety

- Every download is checked against its immutable SHA-256 manifest before anything is replaced.
- On macOS the new bundle must also carry the canonical Developer ID, sealed code, the designated requirement and a stapled notarization ticket.
- The previous app is kept until the new one is in place. A failed verification or install leaves the installed version untouched and shows the cause.
- Downloads go to a temporary folder (`ultravox-update-…` in the system temp directory) that is deleted on success and on every failure.
- The one rollback copy of the previous version (`.UltraVox.previous-<id>.app` beside the app, or `<AppImage>.previous`) is deleted once the new version has run for a minute.
- `.deb` and `.rpm` installs are updated by the system package manager, not by UltraVox.

Download requests to `github.com` send the User-Agent `ultravox-updater/<version>`. They happen only when you click **Install Update** or **Update**, or when **Install updates automatically** is on.

## Files

- `update-check.json` in the platform cache directory (`~/Library/Caches/com.imploselabs.ultravox/` on macOS; inside `ULTRAVOX_DATA_DIR` when set): the time of the last check, the update it found and the cause of a failure. Mode 0600, written atomically.
- `update-preferences.json` in the app data directory: the two switches, a skipped version, and which version the notice last showed and when. Written when you change a switch, skip a version, or the notice shows.
- No update log is written. Deleting both files resets the update state; see [PRIVACY.md](../PRIVACY.md) for the full list of local files.

## Turning it off

- In the app: **Settings → Updates → Check for updates automatically**.
- For a managed or scripted environment: set `ULTRAVOX_NO_UPDATE_CHECK=1` for the UltraVox process.
