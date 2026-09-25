# Voice Studio

Voice Studio is an UltraVox Pro feature on macOS for on-device voice cloning and text-to-speech through FluidAudio PocketTTS. Clone from local dictation recordings or an imported WAV, or synthesize with a built-in voice. The reference is capped at ten seconds; generated speech is 24 kHz mono WAV. Clone only voices you have permission to use.

## UI walkthrough

Open **Voice Studio** from the main window. **Back to dictation** returns to transcription.

- **Voices** lists saved profiles with name, language, creation date, reference duration, and source count. **Rename** changes the label; **Delete** removes the profile, embeddings, and prepared reference, not original recordings or already generated speech.
- **New voice** opens the creation wizard: enter **Name**, choose **Language pack**, and review the reference-recording checkboxes. Candidates are automatically suggested, favoring natural 3–10-second clips. Alternatively, enter an **Import audio file** WAV path or use its file chooser; import replaces the recording selection. Choose **Create**. A name and usable reference are required.
- **Speak** offers **Voice** (your clones or built-ins), **Language pack**, **Text**, and **Use INT8 models** (enabled initially). Choose **Generate**; the latest result has audio playback, duration, and a saved local path. Built-ins offered are `alba`, `mara`, `jessica`, `ryan`, and `hugo`.
- **Last generations · this session** is an in-memory list, not an index reloaded from disk. The UI keeps up to 50 session results; after successful synthesis, the backend retains the newest 50 WAV files by modification time in `outputs/`. Copy important outputs elsewhere before they age out.

Supported pack IDs (use these exact strings, not ISO language codes):

| Language | Packs |
|---|---|
| English | `english` |
| German | `german`, `german_24l` |
| Italian | `italian`, `italian_24l` |
| Portuguese | `portuguese`, `portuguese_24l` |
| Spanish | `spanish`, `spanish_24l` |
| French | `french_24l` |

## CLI reference

Use an available Pro `ultravox-control` binary on macOS. Studio commands open their own local store; they do not send requests to the running UI. **Set the data directory deliberately**: the CLI defaults to the OS temporary directory's `ultravox-control/`, not the desktop's data directory. For tests:

```bash
export ULTRAVOX_DATA_DIR="$(mktemp -d "${TMPDIR:-/tmp}/ultravox-studio.XXXXXX")"
chmod 700 "$ULTRAVOX_DATA_DIR"
ultravox-control studio-status
ultravox-control studio-voices
```

An empty scratch directory has no recording corpus; import an explicitly permissioned WAV to test cloning. To operate an existing library, set `ULTRAVOX_DATA_DIR` to the intended app data directory only with authorization. Do not run simultaneous CLI/UI mutations against the same store: operation locks are process-local.

```bash
# Choose ONE source mode. --auto also applies when no source option is given.
ultravox-control studio-clone --name "My voice" --auto --language english
# Replace these variables with actual recording UUIDs from your authorized corpus.
ultravox-control studio-clone --name "Selected voice" --recordings "$RECORDING_ID_1,$RECORDING_ID_2"
ultravox-control studio-clone --name "Imported voice" --import "/absolute/path/reference.wav" --language english

# Set VOICE_ID to the voice_id printed by studio-clone (or studio-voices).
ultravox-control studio-speak "$VOICE_ID" "A short test of my voice." --language english --out "$ULTRAVOX_DATA_DIR/test.wav"
ultravox-control studio-speak builtin:alba "Hello from Voice Studio." --language english --out "$ULTRAVOX_DATA_DIR/builtin.wav"
ultravox-control studio-speak builtin:hugo "Guten Tag." --language german_24l
ultravox-control studio-rename-voice "$VOICE_ID" "New name"
# Destructive: only after explicit approval.
ultravox-control studio-remove-voice "$VOICE_ID"
```

`--language` defaults to `english` for both clone and speak; synthesis language is chosen independently of the profile's saved label. `--auto` cannot be combined with `--recordings` or `--import`; recordings and import are mutually exclusive. CLI synthesis uses INT8. `--out` copies the retained output to the destination (parent directory must exist; an existing file may be overwritten); without it, use the printed `wav_path`. Output is human-readable, not JSON. `studio-status` reports voice count and `models_ready: false`: readiness is **unknown**, not proof of missing models, because there is no download-free bridge readiness probe.

### Verification recipe

Synthesize a short, known sentence, listen, then transcribe the WAV locally:

```bash
ultravox-control studio-speak builtin:alba "The blue notebook is on the table." --out "$ULTRAVOX_DATA_DIR/roundtrip.wav"
ultravox-control transcribe "$ULTRAVOX_DATA_DIR/roundtrip.wav" v2
```

Compare the printed transcription with the sentence; use `v3` for multilingual transcription. This checks intelligibility, not speaker identity, permission, or perfect fidelity. Transcription models may also need a first-use download.

## Local data and privacy

The desktop uses its app data directory (normally `~/Library/Application Support/com.imploselabs.ultravox/` on macOS), honoring `ULTRAVOX_DATA_DIR`. The CLI override and distinct default are described above.

```text
<data-dir>/voice-studio/
  voices.json       # profile metadata, source paths/IDs, language, timestamps
  voices/<id>.bin   # cloned voice conditioning embeddings
  refs/<id>.wav     # prepared reference audio, at most ten seconds
  outputs/<id>.wav  # generated speech, newest 50 retained
```

Cloning and synthesis run on-device; their audio, text, and embeddings are not uploaded by Voice Studio. Model downloads require a network connection. Voice embeddings are **derived biometric data** stored locally only by this feature; protect them as sensitive data, alongside source audio, metadata, outputs, and backups. Local storage is not a promise of encryption or secure erasure. Deleting a voice does not erase originals, exported copies, backups, or generated outputs. Tests should use a disposable directory and only permissioned inputs; `ULTRAVOX_DATA_DIR` isolates the Studio store, not necessarily FluidAudio's model cache.

## Limitations, licensing, and troubleshooting

- **macOS and Pro only.** Other platforms and the open-source build reject Studio operations.
- **Ten-second conditioning limit.** More source audio does not create an unlimited reference. Prefer 3–10 seconds of clear, single-speaker speech without music, clipping, or long silence. Consider two or more short sources for variety, but their combined prepared audio still stops at ten seconds; two sources are a quality suggestion, not a requirement.
- **English-biased quality.** Treat multilingual pronunciation and voice similarity as something to audition, not guaranteed parity. Upstream built-in prompts are English-trained; language support does not guarantee equal results.
- **First-use downloads.** Budget roughly hundreds of MB for models (size varies with pack, precision, and upstream revision). Cloning loads the shared Mimi encoder; synthesis initializes its selected language/precision models. New packs may require another download.
- **Downloaded model licensing.** Review the current model cards and license terms at [FluidInference/pocket-tts-coreml](https://huggingface.co/FluidInference/pocket-tts-coreml) and its upstream sources before use or redistribution. The app's license does not replace model terms or a speaker's consent.
- **WAV compatibility.** Reference import supports RIFF WAV, including WAVE_FORMAT_EXTENSIBLE PCM/float subformats, mono or stereo: integer PCM 8/16/24/32-bit or float32. Stereo is averaged to mono; mixed source rates are resampled to the first source rate before bridge processing. Other containers need conversion to a supported WAV first.
- **Too-short/silent audio.** `reference audio must contain at least one second of usable audio` means the prepared reference is below the minimum after quiet-edge trimming. Choose a longer, clearly spoken recording; adding silence does not help. Missing or malformed recordings may be omitted from the candidate list.
- **Model download/load failure.** Read the displayed error, check network access to the model host and available disk space, then retry. Do not delete production voice data to fix a model cache problem. A permanently false `models_ready` flag alone is not a failure. The progress display gives elapsed time, not a completion estimate.

## Implementation cross-checks

These references cover the guide, README feature copy, and bundled/operator skills; advice about consent, backups, and quality is operator guidance, not a claim of enforcement.

| Claims | Source (repository-relative, line numbers at review) |
|---|---|
| Pro/macOS guard; ten-second cap; accepted languages | `apps/desktop/src-tauri/src/voice_studio.rs:20`, `:37–65` |
| UI entry; tabs, fields, defaults, actions, local session results | `apps/desktop/src/pages/MainWindow.tsx:733`; `apps/desktop/src/pages/VoiceStudioPage.tsx:11–41`, `:93–133` |
| CLI defaults, syntax, plain-text output, INT8, copy/overwrite, source exclusivity | `apps/desktop/src-tauri/src/bin/ultravox-control.rs:23–27`, `:872–943` |
| Roundtrip transcription v2/v3 | `apps/desktop/src-tauri/src/bin/ultravox-control.rs:782–805`; `crates/ultravox-macos-bridge/src/lib.rs:382–389` |
| Desktop data override/default | `apps/desktop/src-tauri/src/state.rs:324–339`; app identifier in `apps/desktop/src-tauri/tauri.conf.json:4` |
| Local metadata, embeddings, atomic persistence, process-local locks | `crates/ultravox-core/src/voice_studio.rs:13–25`, `:51–75`, `:109–147`; `apps/desktop/src-tauri/src/voice_studio.rs:22–23` |
| Create/reference paths, local inference, failure cleanup, deletion boundaries | `apps/desktop/src-tauri/src/voice_studio.rs:77–80`, `:114–123`, `:133–221`; `crates/ultravox-core/src/voice_studio.rs:109–124` |
| Outputs and newest-50 disk retention; unknown readiness | `apps/desktop/src-tauri/src/voice_studio.rs:125–132`, `:223–290` |
| WAV formats/extensible; candidate selection; trimming, resampling, minimum | `crates/ultravox-core/src/voice_studio.rs:173–268`, `:320–437` |
| FluidAudio local clone/synthesis, separate encoder, downloads, errors | `crates/ultravox-macos-bridge/native/UltraVoxMacOSBridge/Sources/UltraVoxMacOSBridge/UltraVoxMacOSBridge.swift:2940–3003` |
| 24 kHz mono/10 seconds | `crates/ultravox-macos-bridge/src/lib.rs:1256`; upstream `Documentation/TTS/PocketTTS.md:135`, `:180` |
| Approximate model size, model source, English-trained prompts, license context | upstream `Documentation/TTS/PocketTTS.md:50–83`, `:339–343`, `:399–402` |
| Mirror controller socket override/fallback | `Scripts/mirror-control:121–122` |

Here “upstream” refers to the inspected FluidAudio checkout under `crates/ultravox-macos-bridge/native/UltraVoxMacOSBridge/.build/checkouts/FluidAudio/`; it is a build dependency, not a tracked UltraVox document. Size figures are approximate upstream planning information, not a measured download guarantee.
