---
name: ultravox-dictionary-review
description: >
  Audit local UltraVox transcripts and safely update the
  app's custom dictionary (spoken aliases corrected to canonical terms).
  Strictly local: reads recordings only from a scratch copy, proposes
  conservative verified corrections, edits exactly one settings field, and
  verifies every correction with the bundled ultravox-control CLI before
  finishing.
category: engineering
version: 1.0.0
author: Implose Cybernetics
tags: [ultravox, dictionary, transcription, vocabulary, local]
trigger_keywords: [ultravox dictionary, dictionary review, transcription vocabulary, custom dictionary update, fix transcription names]
---

# UltraVox dictionary review

Review the user's local UltraVox transcripts for recurring transcription
errors and update the custom dictionary so future dictation comes out right.
This skill is for the app the user runs on this machine.

## Guarantees (non-negotiable)

- **Local only.** No network access, no uploads. Transcript text and
  dictionary contents never leave this machine.
- **Read-only over recordings.** The live `recordings.sqlite` is never
  opened; all analysis happens on a scratch copy.
- **One settings field.** Only `custom_dictionary` in `settings.toml` may be
  written. No other settings, files, or system state change.
- **Conservative.** Add only corrections the transcripts verify. Anything
  unverifiable is reported to the user, never guessed. Common English words
  are never used as aliases.
- **Reversible.** Back up `settings.toml` before editing; restore on failure.

## Step 1 — Locate the installation

- Default data directory (macOS):
  `~/Library/Application Support/com.imploselabs.ultravox/`
- Honor `ULTRAVOX_DATA_DIR` if it is set.
- Confirm both `settings.toml` and `recordings.sqlite` exist there.
- If they are missing, ask the user for their data directory. Do not scan the
  whole disk.
- Note whether the app is currently running (`pgrep -f UltraVox.app`); you
  will need that for Step 6.

## Step 2 — Snapshot into a scratch directory

```bash
SCRATCH="$(mktemp -d "${TMPDIR:-/tmp}/ultravox-dict-review.XXXXXX")"
chmod 700 "$SCRATCH"
cp "$DATA_DIR/settings.toml" "$SCRATCH/settings.toml.bak"
cp "$DATA_DIR/recordings.sqlite" "$SCRATCH/"          # plus -wal/-shm if present
```

Work only inside `$SCRATCH`. Keep `settings.toml.bak` as the rollback copy.

## Step 3 — Export transcripts

Query the scratch copy, never the live database:

```bash
sqlite3 -json "$SCRATCH/recordings.sqlite" \
  "SELECT id, timestamp, transcription FROM recordings
   WHERE status = 'completed' AND length(transcription) > 0
   ORDER BY timestamp" > "$SCRATCH/transcripts.json"
```

Load it with a small `python3` script (stdlib `json` only). When quoting
evidence later, use the shortest snippet that proves the point; do not dump
bulk transcript text into your report.

## Step 4 — Analyze for candidates

1. Frequency pass: count words and adjacent word pairs across all
   transcripts; list terms that look like products, people, companies,
   projects, and technical jargon.
2. Check each candidate against these recurring error classes:
   - brand casing (`overseer` → `OverSeer`)
   - split compounds (`open router` → `OpenRouter`, `Ultra Term` → `UltraTerm`)
   - name spelling variants (`Katharina`/`Katerina` → `Katherina`)
   - phonetic garbles (`retext` → `Retex`, `steak pie` → `Speak Pi`,
     `deep sea carnets` → `DeepSeek Harness`)
   - spoken-form model numbers (`GLM53`, `GLM five point three` → `GLM 5.3`)
3. For every candidate, pull at least three context snippets (roughly 60
   characters on each side) and confirm the intended term from context.
4. Evidence bar: recurring errors (2+ occurrences) or spellings the user has
   explicitly stated are verifiable. A single occurrence needs unusually
   clear context or explicit user confirmation.

## Step 5 — Refuse dangerous corrections

Never propose an alias that is a common English word or generic phrase, even
if one transcript misused it. Real examples of refusals:
`resend` (also a verb), `recent`, `fault` (for vault), `sold`/`SALT` alone,
`juice box` (sometimes meant literally). For risky words, only multi-word
phrases are acceptable, e.g. `GPT 5.6 sold` → `GPT 5.6 Sol`, which cannot
fire unless the full phrase appears.

## Step 6 — Draft entries (format and engine semantics)

Dictionary text lives in `settings.toml` as `custom_dictionary = """…"""`:

```
Canonical term = alias one, alias two
Canonical-only term
```

- One entry per line; everything is plain text, no comments.
- Engine semantics you must respect:
  - exact alias matches are case-insensitive, whole-word, longest match
    first, and punctuation is preserved;
  - an alias equal to its canonical term ignoring case is a no-op — never
    add one;
  - alias sources are global across entries: the same text cannot serve two
    entries;
  - fuzzy typo correction applies to canonical terms only and is
    conservative; never rely on it — add the explicit alias;
  - a canonical-only entry still normalizes the casing of its own term
    (`Cherry Pie` corrects `cherry pie`).
- Hard limits: 512 entries, 16 aliases per entry, 128 bytes per field,
  128 KiB total. Count before writing.

## Step 7 — Apply

1. **Quit the app first.** The app owns settings writes; editing the file
   while it runs risks the edit being overwritten. Wait for the process to
   exit.
2. Apply the drafted dictionary to the real `settings.toml`, changing
   nothing else.
3. Relaunch the app and confirm the process is running.

## Step 8 — Verify

- Structural: parse `settings.toml` with a TOML parser; confirm the entry,
  alias, and byte limits hold.
- Behavioral (preferred): locate `ultravox-control` (PATH, an
  `UltraVox-macos-*.zip` payload's `bin/` directory, or a repository
  `target/release/` build), then for each new alias run:

  ```bash
  ultravox-control dictionary-apply "<real transcript snippet containing the alias>" "<full dictionary text>"
  ```

  Require: the intended canonical term is produced and the surrounding text
  is unchanged.
- Interaction tests for overlapping entries: longest match must win
  (e.g. `deep seek harness` must become `DeepSeek Harness`, not
  `DeepSeek harness`).
- Negative controls: generic sentences containing the words you refused in
  Step 5 must pass through completely unchanged.
- If the CLI is unavailable, say so and report that verification was
  structural only (weaker evidence).
- If any check fails: restore `settings.toml.bak`, relaunch the app, fix the
  entries, and repeat.

## Step 9 — Report

Finish with tables only:

| Added | | |
|---|---|---|
| Canonical | Aliases added | Evidence (count + one short snippet) |

| Flagged for the user | | |
|---|---|---|
| Candidate | Why it is unverified | |

| Refused as unsafe | |
|---|---|
| Candidate | Reason |

| Verification | |
|---|---|
| Per-entry result and negative controls | PASS/FAIL |

Close with the rollback path (`$SCRATCH/settings.toml.bak`) and remind the
user to delete the scratch directory when satisfied.
