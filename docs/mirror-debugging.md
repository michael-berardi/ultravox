# Native mirror debugging

UltraVox has a private, macOS native mirror for inspecting the real app frontend in a hidden WKWebView. It is not a browser mock or a second application. The normal primary window remains independent; only an explicit `apply` navigates it to a staged frontend revision.

## Operator contract

- Keep the mirror invisible and unfocused for the entire session. Never show it to make rendering or capture work.
- Use only permissioned recordings. Mirror Voice Studio operations read the real history and recording corpus one way, while voice creation, rename, deletion, and synthesis use a disposable store. Do not treat the mirror as a way to edit the real voice library.
- Inspect `state`, DOM results, and a native capture before applying a frontend revision. Applying changes frontend navigation, not sandbox data or primary storage.
- Close every session, including failed checks. The session deadline is a backstop, not a substitute for cleanup. There is no screen-recording fallback.

## Build and launch

Both frontend and Rust gates are required. With the normal desktop build prerequisites installed, run from the repository root:

```bash
cd apps/desktop
VITE_MIRROR_DEBUG=1 pnpm build --base ./
cd ../..
cargo build -p ultravox --bin ultravox --features mirror-debug
```

The frontend build script runs TypeScript and Vite. Relative asset URLs are required for staged revisions. The Rust default features include `custom-protocol`; keep it enabled for this workflow. Hot refresh requires embedded assets, not the Vite development server.

Launch the binary built above from the repository root (the default Cargo target directory is assumed):

```bash
./target/debug/ultravox
```

There is no mirror launch flag. The compiled feature starts the control socket; `open` creates the mirror later. Use an operator-authorized normal exit of any already-running UltraVox instance before launching this build: the app has a single-instance plugin. Do not overwrite a running binary or installed bundle. Rust changes require a separate rebuild and relaunch; staging only updates frontend assets.

## Control workflow

Run control commands from the repository root in another terminal. Responses are JSON. A successful socket request has outer `ok: true`; `eval` also has its own result under `value`, so check `value.ok` and `value.value` rather than relying on process exit status alone.

```bash
Scripts/mirror-control state
Scripts/mirror-control open
Scripts/mirror-control state
Scripts/mirror-control eval 'return {ready: document.readyState, width: innerWidth, height: innerHeight};'
Scripts/mirror-control capture /tmp/screenshots/mirror-initial.png
```

`open` defaults to 900 seconds; `open 120` selects a 120-second session. The accepted lifetime is 1–3600 seconds, and only one mirror can be open. It starts at 450 × 650 logical pixels, the default app window size. `state` reports the open state and, when open, visibility, focus, mirror URL, primary URL, and loaded hot revision. An embedded mirror has no hot revision digest.

Optional viewport control:

```bash
Scripts/mirror-control resize 1920x1080
```

Width must be 400–3840 and height 500–2160 (the app window is 450 × 650 by default). `eval` takes a JavaScript async function body, not a filename; use `return` to produce a JSON-serializable result. Script size is 1–32768 bytes. Evaluation times out after eight seconds, and finite animations are settled in the mirror before evaluation without waiting on suspended timers.

After changing frontend code, rebuild with the frontend command above, then stage the resulting directory:

```bash
Scripts/mirror-control stage apps/desktop/dist
```

Copy the returned 64-character lowercase revision digest into `REVISION` below. This is a shell variable, not an additional control option:

```bash
REVISION='paste-the-returned-digest-here'
Scripts/mirror-control refresh "$REVISION"
Scripts/mirror-control state
Scripts/mirror-control eval 'return {ready: document.readyState, width: innerWidth, height: innerHeight};'
Scripts/mirror-control capture /tmp/screenshots/mirror-revision.png
```

Refresh recreates only the hidden mirror, retaining its in-memory storage, viewport, and original session deadline. Navigation is asynchronous: confirm the requested revision in `state`, then verify the actual UI with DOM inspection and capture. Readiness and dimensions alone are not evidence that a UI change is correct.

Only after verification and explicit authorization to change the primary frontend:

```bash
Scripts/mirror-control apply
Scripts/mirror-control state
```

`apply` navigates the primary window to the mirror's current requested revision, without the mirror query parameter. It does not copy mirror storage, voices, or generated audio into the primary app. `state` exposes `primaryUrl` for checking navigation; `eval` and `capture` still target the mirror, not the primary window.

To return the mirror to its remembered previous revision:

```bash
Scripts/mirror-control rollback
Scripts/mirror-control state
Scripts/mirror-control eval 'return document.readyState;'
Scripts/mirror-control capture /tmp/screenshots/mirror-rollback.png
```

Rollback is mirror-first, including a previous embedded revision when available. Verify it before a separately authorized `apply`. If there is no remembered previous revision, refresh a retained staged digest explicitly. The command-line `refresh` accepts digests only, not the word `embedded`.

Always finish with:

```bash
Scripts/mirror-control close
Scripts/mirror-control state
```

The final state should report `open: false`. Closing an already-closed mirror returns an error; check state if the deadline may have expired. Close destroys the mirror and attempts sandbox removal, but does not undo an applied primary revision or delete staged assets and captures.

## Staging rules

Staging does not contact the app. It writes an immutable revision under `~/.ultravox/hot-assets/<revision>/`, using a temporary directory, a generated manifest, and atomic rename. It never overwrites embedded runtime assets or an installed app bundle.

- The source and its ancestors must not be symlinks. Build entries must not be symlinks, hidden names, or files/directories ending in `.pem`, `.key`, `.p12`, or `.pfx` (case-insensitive).
- Files must be regular, single-link files, at most 16 MiB each; the build is limited to 64 MiB total and 4096 files.
- Relative asset paths are limited to 240 characters, with ASCII letters, digits, `_`, `.`, and `-` in each component. Empty, `.` and `..` components are not allowed. Root `manifest.json` is reserved.
- `index.html` must exist, contain an external module script, and reference staged local relative assets in its nonempty, non-fragment `src` and `href` attributes. Root-relative and remote asset URLs fail validation. A JavaScript asset must contain the compiled mirror seed marker.
- `~/.ultravox` and `hot-assets` must be user-owned directories without group/other permissions. New directories use mode 0700 and staged files use 0600. An already-staged digest is rejected; retain and reuse that digest rather than editing its files.

The manifest records sizes, SHA-256, and FNV-64 values; the revision is derived from the file specifications. The native loader checks ownership, types, permissions, sizes, and FNV-64 values before caching assets in memory. Asset requests do not read the filesystem. At most eight distinct revisions can be loaded in one process; closing the mirror does not reset that cache.

## Isolation and disposable data

The sandbox is created at `std::env::temp_dir()/ultravox-mirror-<UUID>` with mode 0700. On macOS the temporary root can be the user's system temporary directory, not necessarily `/tmp`. It is allocated on open, reused during the session, and removal is attempted on close, deadline expiry, and normal app exit. A crash or removal failure can leave disposable data behind; do not assume secure erasure.

Voice Studio dispatch selects this directory only for the `native-mirror` window. It scans the real history and recordings for corpus candidates, then operates on the sandbox Studio. There is no process-global data-directory override: the primary window and CLI retain their own store. This is not an OS-level sandbox or a promise that every native operation is side-effect-free; it is a window-specific command allowlist and Voice Studio storage boundary.

The frontend seeds storage from the primary's local storage snapshot, then replaces both mirror `localStorage` and `sessionStorage` with an in-memory map. Distribution access is simulated; theme-material and telemetry-use calls are suppressed. Other calls go through the native allowlist. A mirror-only content security policy restricts network/resource access, navigation is confined to the primary origin with exactly `native-mirror=1`, and `window.open` is disabled. The normal app component still renders; the DEV-gated theme and settings harnesses are separate.

## Consumer-build exclusion

`mirror-debug` is not a default Cargo feature. Without it, the native mirror module, socket startup, eval handler, invocation guard, hot-asset installation, and cleanup hooks are excluded at compile time. Separately, when `VITE_MIRROR_DEBUG` is not `1`, Vite's consumer build plugin replaces the native mirror adapter with no-op exports. A runtime query parameter cannot enable a stripped adapter or add the missing native feature.

For a consumer build, explicitly disable the frontend gate and omit the Rust feature:

```bash
cd apps/desktop
VITE_MIRROR_DEBUG=0 pnpm build --base ./
cd ../..
cargo build -p ultravox --bin ultravox
```

Verify the build inputs and artifacts: confirm the native feature is absent from the Cargo invocation and any enabled feature dependencies, and inspect emitted `apps/desktop/dist` JavaScript for absence of `__ULTRAVOX_MIRROR_SEED__`, `__ULTRAVOX_MIRROR_SNAPSHOT__`, and `native-mirror`. Review the compile gates in `src-tauri/src/lib.rs` and the replacement plugin in `vite.config.ts`; neither a frontend-only nor a Rust-only check establishes both exclusions. When testing the consumer binary alone, it must not create `~/.ultravox/mirror.sock`; an old socket is not evidence of an active mirror server. Do not ship either mirror-enabled artifact as a consumer build.

## Troubleshooting and cleanup

- **Socket unavailable or unsafe:** the endpoint is `<ULTRAVOX_DATA_DIR>/mirror-control/mirror.sock` when `ULTRAVOX_DATA_DIR` is set, otherwise `~/.ultravox/mirror.sock`, a user-owned Unix socket with mode 0600 in a private directory. The controller validates ownership/type/permissions, sends newline-delimited JSON with a 64 KiB request limit, and waits up to 15 seconds. Confirm that the intended mirror-capable binary is running. Startup deliberately refuses to unlink an existing endpoint; investigate ownership and whether it is live before any operator-authorized stale-file cleanup.
- **Snapshot not ready on open:** wait for the primary frontend to initialize and confirm both build gates were enabled. Do not substitute a development-server build for embedded hot assets.
- **Refresh returns before rendering:** check `state` and retry bounded DOM inspection. Refresh does not extend the deadline. At the eight-revision runtime limit, arrange an authorized rebuild/relaunch rather than mutating staged files.
- **Capture path errors:** use a new direct child of `/tmp/screenshots` with lowercase `.png`. The controller creates the directory if absent; it must be user-owned and not a symlink. Capture creates a new 0600 file and never overwrites an existing path.
- **Empty, blank, or timed-out capture:** treat it as failure, never as visual evidence. Capture uses `WKWebView.takeSnapshot`, not screen recording. Native validation rejects absent image data, invalid PNG signatures, zero bytes, and PNGs over 32 MiB, but does not prove that a nonempty PNG contains useful rendered content. Inspect the image and use `state`/`eval` for diagnosis; never show or focus the mirror and never use a screen-recording fallback. The snapshot timeout is ten seconds after layout settling.
- **Session interrupted:** close the session as soon as control is available and confirm `open: false`. Normal app exit also attempts sandbox/socket cleanup. Retained screenshots and staged revisions are not removed by `close`; handle them as potentially sensitive operator artifacts under the local retention policy.
