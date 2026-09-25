---
name: ultravox-voice-studio
description: >
  Operate UltraVox Pro Voice Studio on macOS through its CLI or UI for
  permissioned on-device voice cloning and speech synthesis.
category: engineering
version: 1.0.0
author: Implose Cybernetics
tags: [ultravox, voice-studio, cloning, speech, local]
trigger_keywords: [ultravox voice studio, clone voice, synthesize speech, pockettts]
---

# UltraVox Voice Studio

Use when asked to operate UltraVox Pro Voice Studio via CLI or UI. Requires macOS and an official build with an active UltraVox Pro license; do not attempt to bypass license checks.

## Safety and setup

- Clone only voices the user has permission to use; ask if consent is unclear. Do not impersonate others deceptively.
- Never casually touch the production data directory. Confirm the target and requested writes before operating an existing library; do not run concurrent UI/CLI mutations on one store.
- Use a fresh `ULTRAVOX_DATA_DIR` for tests. CLI defaults to the OS temporary directory's `ultravox-control`, NOT the desktop data directory. Even status opens/initializes a store.
- Audio, text, and derived biometric embeddings stay local to Voice Studio processing. First-use models download from the network; the Studio override does not promise isolation of model caches. Protect references, embeddings, metadata, outputs, and backups.
- Locate `ultravox-control` on PATH or in the official build's `bin/` or repository `target/release/`; stop if unavailable rather than installing or rebuilding without permission.

```bash
export ULTRAVOX_DATA_DIR="$(mktemp -d "${TMPDIR:-/tmp}/ultravox-studio.XXXXXX")"
chmod 700 "$ULTRAVOX_DATA_DIR"
ultravox-control studio-status
ultravox-control studio-voices
```

`models_ready: false` means unknown readiness, not proof of missing models.

## CLI quick reference

Use ONE cloning source mode. A scratch store has no recordings: import a permissioned WAV, or obtain explicit authorization to use an existing corpus. Replace ID variables with real UUIDs; `VOICE_ID` comes from clone/list output.

```bash
ultravox-control studio-clone --name "My voice" --auto --language english
ultravox-control studio-clone --name "Selected" --recordings "$RECORDING_ID_1,$RECORDING_ID_2"
ultravox-control studio-clone --name "Imported" --import "/absolute/path/permissioned.wav"
ultravox-control studio-speak "$VOICE_ID" "A short voice test." --language english --out "$ULTRAVOX_DATA_DIR/test.wav"
ultravox-control studio-speak builtin:alba "Hello from Voice Studio." --language english
ultravox-control studio-speak builtin:hugo "Guten Tag." --language german_24l
ultravox-control studio-rename-voice "$VOICE_ID" "New name"
# Destructive: require explicit approval first.
ultravox-control studio-remove-voice "$VOICE_ID"
```

CLI output is plain text. Language defaults to `english`; other pack IDs: `german`, `german_24l`, `italian`, `italian_24l`, `portuguese`, `portuguese_24l`, `spanish`, `spanish_24l`, `french_24l`. Built-ins offered in the UI: `alba`, `mara`, `jessica`, `ryan`, `hugo`. CLI synthesis uses INT8. `--out` copies the generated WAV and may overwrite an existing destination; use a fresh path whose parent exists.

## Cloning quality and UI

Prefer 3–10 seconds of clean single-speaker speech, without background music, clipping, or long silence. Consider 2+ short sources for varied delivery, but the combined prepared reference is capped at ten seconds; later sources may not contribute. Two sources are advice, not a requirement. At least one second of usable audio must remain after trimming. Supported input is mono/stereo RIFF WAV including extensible PCM/float; convert other containers before import. Audition multilingual results: English-trained built-in prompts do not promise equal quality across languages.

In the main window open **Voice Studio → Voices → New voice**. Enter **Name**, **Language pack**, and select recordings or supply/import a WAV path; choose **Create**. **Rename** and **Delete** manage saved profiles. Under **Speak**, choose a cloned or built-in **Voice**, language, **Text**, and **Use INT8 models**, then **Generate**. Listen to the result and retain its saved path. Session history is not reloaded from disk; disk outputs retain only the newest 50 WAVs. Copy important outputs out before retention removes them.

## Synthesis verification recipe

Start with one short sentence. Listen for intelligibility and unwanted artifacts; then do a local synthesis/transcription roundtrip:

```bash
ultravox-control studio-speak builtin:alba "The blue notebook is on the table." --out "$ULTRAVOX_DATA_DIR/roundtrip.wav"
ultravox-control transcribe "$ULTRAVOX_DATA_DIR/roundtrip.wav" v2
```

Compare the printed transcript to the intended words (use `v3` for multilingual transcription). Roundtrip transcription is not identity verification or proof of consent. Report actual output paths, listening/transcription outcomes, and any download failure; never claim success from status alone. Review model-card licensing before redistribution. Delete only the authorized scratch artifacts after review; deleting a voice does not erase originals, exports, generated speech, or backups.

Implementation references and storage layout: `docs/voice-studio.md`, especially its cross-check table.
