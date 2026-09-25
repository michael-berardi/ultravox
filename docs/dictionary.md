# Custom dictionary and Retex vocabulary

UltraVox 0.10.0 applies a compiled, in-memory dictionary after local transcription and before history, clipboard copy, or paste. On Whisper platforms, canonical terms are also appended to the existing initial prompt. No dictionary feature calls a transcription service.

## Manual dictionary (core)

Open **Settings → Dictionary**. Enter one item per line:

```text
# Comments and blank lines are ignored
Retex = retext
UltraVox = Ultra Box, Ultra Vox
Marisol Vega
```

The text before `=` is the canonical output. Aliases are exact, case-insensitive whole-term matches. Canonical custom terms also receive conservative fuzzy matching; short or embedded common-word fragments are not broadly rewritten. Longer matches take priority and surrounding punctuation is preserved.

The parser bounds source size, entry count, aliases per entry, and field length. Duplicate canonical terms and aliases are folded before compilation.

## Agent dictionary review skill (core)

UltraVox bundles an agent skill at `skills/dictionary-review/SKILL.md` in the repository and inside every install at `Resources/skills/dictionary-review/SKILL.md`. **Settings → Dictionary → Agent dictionary review** reveals the file or copies its contents; UltraVox never installs anything into an agent on its own.

Give the file to any AI agent and it will: snapshot the local database read-only into a scratch directory, analyze completed transcripts for recurring mis-transcriptions, refuse common-word aliases, apply verified entries to `custom_dictionary` only (quitting and relaunching the app around the edit), verify every correction and negative control with `ultravox-control dictionary-apply`, and report additions, flagged candidates, and refusals. All processing stays on the device.

## Retex vocabulary (Pro, macOS)

Retex access is off by default. Automated vault scanning is currently available on macOS.

1. Choose **Add vaults…** and explicitly select one or more vault directories.
2. Choose **Scan now**, or separately opt into **Refresh weekly at startup**.
3. Remove a vault to revoke permission. Generated vocabulary is cleared; manual entries are preserved.

UltraVox invokes the installed executable directly, without a shell:

```text
retex vocabulary --vault <selected-directory> --limit 10000 --raw-json
```

The command is read-only. UltraVox does not traverse the vault itself and does not call Obsidian. Retex 1.2.0 or newer deterministically extracts a bounded set of likely person, company, brand, application, product, project, and custom terms from local records. Its response contains only bounded candidate terms and aggregate counts—never raw note bodies, excerpts, or paths. UltraVox then compares those candidates against up to 500 recent transcripts entirely in-process, retains at most 256 terms, to prioritize exact, spacing, one-letter, and adjacent-word corrections. Transcript text is never passed to Retex, and nothing is uploaded by this integration.

Automatic refresh runs only with an UltraVox Pro license, only after opt-in, only when selected vaults exist, and only when the last successful refresh is at least one week old. It starts as detached background work and does not gate app readiness, recording, or transcription.

## Agent-first checks

Build the control binary with its CLI feature, then use:

```bash
cargo run -p ultravox --features cli --bin ultravox-control -- dictionary-smoke
cargo run -p ultravox --features cli --bin ultravox-control -- \
  dictionary-apply "Send retext to Ultra Box." $'Retex\nUltraVox = Ultra Box'
cargo run -p ultravox --features cli --bin ultravox-control -- \
  retex-scan /path/to/disposable-vault
cargo run -p ultravox --features cli --bin ultravox-control -- \
  retex-apply /path/to/disposable-vault "retext and Ultra Box"
```

`retex-scan` prints only vault, structured-record, and generated-term counts. It does not print vault content by default. `retex-apply` prints only the caller-supplied test text after applying scanned terms, so it can verify the end-to-end correction path without revealing vault records. A build without the Pro module rejects the scan command at the compiled feature boundary.

## Accuracy review cadence

For each candidate release and monthly thereafter:

1. Select a fixed, permissioned set of representative recordings and keep human-reviewed reference text outside the repository.
2. Transcribe every recording with the previous release and the candidate using the same model and settings.
3. Measure word error rate before dictionary processing and after it; also review every changed span to catch harmful corrections.
4. Add only confirmed aliases to the manual regression dictionary, then rerun the entire corpus. Never infer an alias from audio or history without a human confirming the intended spelling.
5. Run the unit suite, `dictionary-smoke`, the disposable-vault `retex-scan`/`retex-apply` checks, and the packaged official app build.

The weekly Retex refresh keeps names and product vocabulary current between reviews. It is intentionally separate from transcription, so scan latency or a missing Retex executable cannot slow dictation.
