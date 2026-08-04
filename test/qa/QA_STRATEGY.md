# Dictator End-to-End QA Strategy

This document defines a non-disruptive QA sequence for Dictator's real transcription
path once the macOS bridge / transcription engine is implemented. It uses only the
existing `dictator-control` CLI surface and proposes small, safe extensions for the
post-implementation CLI. No core bridge, command, or Tauri command files are
modified here; only isolated fixtures and docs live under `test/`.

## Environment isolation

All QA commands use a temporary `DICTATOR_DATA_DIR` so they do not touch the real
user settings, model cache, or recording history. Run from the repo root or use
absolute paths for the fixture.

```bash
export DICTATOR_DATA_DIR="$(mktemp -d)/dictator-qa"
export DICTATOR_FIXTURE="$PWD/test/fixtures/jfk-short.wav"
export DICTATOR_CONTROL="$PWD/target/debug/dictator-control"
```

> On a fresh checkout the binary may need to be rebuilt with
> `cargo build -p dictator --features cli --bin dictator-control` first.

## Phase 1 — Core CLI health (no microphone, no UI, no paste)

These commands can run in any terminal, including CI/SSH, and never trigger
permission prompts.

### 1.1 health

```bash
$DICTATOR_CONTROL health
```

**Pass criteria:**
- Exit code `0`.
- Output contains `health: ok`.
- Default model is listed (currently `fluidaudio-en-v2`).
- Config and history paths point inside `DICTATOR_DATA_DIR`.

### 1.2 status

```bash
$DICTATOR_CONTROL status
```

**Pass criteria:**
- Exit code `0`.
- Output contains `status: ok`.
- `recording: false` and `transcription: idle` when no session is active.

### 1.3 model-catalog

```bash
$DICTATOR_CONTROL model-catalog
```

**Pass criteria:**
- Exit code `0`.
- At least one model is listed.
- One model is marked `(default)`.

### 1.4 history-smoke

```bash
$DICTATOR_CONTROL history-smoke
```

**Pass criteria:**
- Exit code `0`.
- Output contains `history-smoke: ok`.
- Verifies in-memory SQLite round-trip (insert, fetch, list, delete).

### 1.5 download-smoke

```bash
$DICTATOR_CONTROL download-smoke
```

**Pass criteria:**
- Exit code `0`.
- Output contains `download-smoke: ok`.
- Spins up a local HTTP server, downloads the payload, and verifies cache state.

### 1.6 shortcut-config-smoke

```bash
$DICTATOR_CONTROL shortcut-config-smoke
```

**Pass criteria:**
- Exit code `0`.
- Output contains `shortcut-config-smoke: ok`.
- Verifies `Option+Backtick` hold-to-record settings round-trip correctly.

## Phase 2 — Audio subsystem (no recording, just enumeration)

### 2.1 audio-devices

```bash
$DICTATOR_CONTROL audio-devices
```

**Pass criteria:**
- Exit code `0`.
- Output contains `audio-devices: ok`.
- At least one input device is listed.
- The default device is marked `(default)`.

**Risk:** On macOS this enumerates CoreAudio input devices. It does **not** open
a capture stream, so it should not trigger the microphone permission dialog, but
running it on a headless CI machine may report zero devices. That is expected and
is not a failure of the bridge.

## Phase 3 — macOS bridge dry-runs (isolated, no model load)

These commands are macOS-only. They exercise the native bridge without loading a
large model or opening a real recording stream.

### 3.1 caret-bridge-dry-run

```bash
$DICTATOR_CONTROL caret-bridge-dry-run
```

**Pass criteria:**
- Exit code `0`.
- Output contains `caret-bridge-dry-run: ok`.
- `found:` is either `0` or `1`. In a terminal with no focused accessibility
  element, `0` is expected.

### 3.2 paste-bridge-dry-run

```bash
$DICTATOR_CONTROL paste-bridge-dry-run
```

**Pass criteria:**
- Exit code `0`.
- Output contains `paste-bridge-dry-run: ok`.
- Result code is non-negative.

**Risk / safety:** This command actually writes "Dictator paste bridge dry run" to
the general pasteboard and posts a `Cmd+V` event to the currently focused
application. Run it with a harmless target window focused (e.g., a scratch text
file) and be aware it will overwrite the user's clipboard. Do not run it during
active work in another app.

## Phase 4 — Real transcription with the known fixture

The current CLI exposes two file-transcription checks backed by the macOS bridge.

### 4.1 Bundled fixture smoke

```bash
$DICTATOR_CONTROL transcribe-fixture-smoke
```

**Pass criteria:**
- Exit code `0`.
- Output contains `transcribe-fixture-smoke: ok`.
- The transcription is non-empty and contains `fellow` and `americans`
  (case-insensitive).

### 4.2 Arbitrary audio path

```bash
$DICTATOR_CONTROL transcribe "$DICTATOR_FIXTURE"
```

Pass `v2` or `v3` as a second argument to select a supported transcription
version; the default is `v2`.

**Pass criteria:**
- Exit code `0`.
- Output contains `transcribe: ok`.
- The transcription is non-empty and contains the expected fixture terms.

### Fixture validation (can run today)

Before trusting the fixture, verify it is valid WAV audio:

```bash
file "$DICTATOR_FIXTURE"
python3 - <<'PY'
import os
import wave
with wave.open(os.environ["DICTATOR_FIXTURE"], "rb") as w:
    assert w.getnchannels() == 1
    assert w.getframerate() == 16000
    assert w.getsampwidth() == 2
    assert w.getnframes() > 0
    print("fixture ok")
PY
```

## Phase 5 — Optional short microphone recording (manual, gated)

A real microphone test is valuable but inherently interactive on macOS because
the first capture triggers the system microphone permission dialog. To keep it
non-disruptive, gate it behind a manual step.

### Proposed command after implementation

```bash
$DICTATOR_CONTROL record-mic-smoke --duration 1 --output /tmp/dictator-qa-mic.wav
```

**Pass criteria:**
- Exit code `0`.
- Output contains `record-mic-smoke: ok`.
- `/tmp/dictator-qa-mic.wav` exists and is non-empty.
- WAV header is valid and duration is approximately 1 second.

**Safety rules:**
- Only run this on a machine where Dictator already has microphone permission, or
  where a human can approve the prompt.
- Run it in a quiet environment so the captured file has actual audio energy.
- Do not run it as part of an unattended CI job unless the runner has already
  granted permission and the test is expected to succeed.
- If the permission dialog appears unexpectedly, the test fails the
  "non-disruptive" requirement.

## Phase 6 — Background app launch and Tauri health

### 6.1 Launch the built app without stealing foreground

```bash
open -j -g /Applications/Dictator.app
```

Or, during development:

```bash
# from the repo root, in a subshell so it does not block the terminal
( pnpm desktop:dev & ) >/tmp/dictator-qa-dev.log 2>&1
```

### 6.2 Verify the app is alive

Use the CLI or an invoke-based probe:

```bash
$DICTATOR_CONTROL health
$DICTATOR_CONTROL status
```

**Pass criteria:**
- Both commands exit `0` when the app is running.
- `status` reports `recording: false` and `transcription: idle`.

### 6.3 Shut down the app cleanly

```bash
pkill -x "Dictator"
# or, if launched via pnpm desktop:dev, kill the Tauri process group
pkill -f "dictator-desktop"
```

## Phase 7 — Full non-disruptive run checklist

Run in order, stopping at any failure:

```bash
set -e
export DICTATOR_DATA_DIR="$(mktemp -d)/dictator-qa"
export DICTATOR_CONTROL="$PWD/target/debug/dictator-control"
export DICTATOR_FIXTURE="$PWD/test/fixtures/jfk-short.wav"

$DICTATOR_CONTROL health
$DICTATOR_CONTROL status
$DICTATOR_CONTROL model-catalog
$DICTATOR_CONTROL history-smoke
$DICTATOR_CONTROL download-smoke
$DICTATOR_CONTROL shortcut-config-smoke
$DICTATOR_CONTROL audio-devices
$DICTATOR_CONTROL caret-bridge-dry-run
$DICTATOR_CONTROL transcribe-fixture-smoke
$DICTATOR_CONTROL transcribe "$DICTATOR_FIXTURE"
# $DICTATOR_CONTROL paste-bridge-dry-run   # only in a safe scratch window
```

## Blockers and risks

1. **Microphone permission prompt.** Any command that opens a capture stream will
   trigger the macOS permission dialog on first use. This violates the
   "non-disruptive" goal for automated/headless runs. Keep mic tests optional and
   manual.

2. **Paste bridge is not side-effect free.** `paste-bridge-dry-run` overwrites the
   clipboard and posts a paste event to the focused app. It must run in a
   controlled window.

3. **Transcription checks are macOS-specific and resource-intensive.** The two
   commands load the native bridge and on-device model on macOS. Non-macOS
   builds report the checks as skipped, so they do not validate transcription.

4. **Audio device enumeration on headless CI.** `audio-devices` may report zero
   devices on CI runners. This is an environmental limitation, not a product bug.
   Accept the test only when at least one device is expected.

5. **Model download / network.** `download-smoke` uses a local server, so it does
   not hit the internet. Real model-download tests are out of scope here.

6. **Tauri app signing / notarization.** Launching the production app with `open`
   may fail on a development machine if the build is unsigned or quarantined. Use
   `pnpm desktop:dev` for local QA instead.

## Remaining CLI additions

Two optional commands would make more of this strategy directly scriptable:

- `record-mic-smoke --duration <seconds> --output <path>` — short capture with
  explicit duration and output path.
- `background-launch --data-dir <dir>` — launch the Tauri app in the background
  and verify it registers with the CLI data directory.

The implemented `transcribe` and `transcribe-fixture-smoke` commands already
cover file-based transcription through the current macOS bridge.
