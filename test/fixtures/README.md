# Dictator QA Audio Fixture

## File

- `jfk-short.wav` — 3-second, 16-bit PCM mono WAV at 16 kHz

## Source

Derived from `jfk.wav` in the repository root, the standard Whisper.cpp sample of
John F. Kennedy's inaugural address. The original is 11 seconds; this clip keeps
the first three seconds so the fixture stays small (~94 KB) while still containing
real, recognizable speech.

## Expected transcription

The clip contains the opening of the famous line:

> "And so, my fellow Americans, ask not what your country can do for you..."

For automated QA, do not require a character-perfect match. A passing check
should verify that the transcription contains the key tokens **"ask not"** and
**"country"** (or a very close phonetic variant). Whisper may emit small
differences in punctuation or capitalization.

## Why real speech

Synthetic sine waves or silence are not useful for validating a real transcription
engine because they contain no language signal. This fixture is real speech with a
well-known reference, so it can catch model-loading, audio-decode, and inference
failures while remaining deterministic and tiny.

## Limitations

- This is English-only. It does not validate multilingual models or non-English
  language detection.
- It is only ~3 seconds, so it cannot test long-form transcription, VAD, or
  streaming behavior.
- The clip is band-limited telephone-quality mono audio, so it does not exercise
  stereo or high-sample-rate input paths.
