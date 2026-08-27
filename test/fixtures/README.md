# UltraVox QA Audio Fixture

## File

- `jfk-short.wav` — 3-second, 16-bit PCM mono WAV at 16 kHz

## Source

Derived from the standard `whisper.cpp` JFK sample of John F. Kennedy's
inaugural address. This three-second excerpt stays small while retaining
recognizable speech for deterministic transcription checks.

## Expected transcription

The clip contains the opening of the famous line:

> "And so, my fellow Americans..."

For automated QA, do not require a character-perfect match. A passing check
verifies the key tokens **"fellow"** and **"Americans"** case-insensitively.
Small punctuation or capitalization differences are acceptable.

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
