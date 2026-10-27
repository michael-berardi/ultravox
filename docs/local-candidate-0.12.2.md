# UltraVox 0.12.2 local candidate

This candidate is installed locally. Public downloads and the updater feed remain unchanged pending approval.

- Removes the music player, its controls, and the dynamic equalizers.
- Adds the Libraries.dev voice animation around the recording panel, using the existing microphone level.
- Fixes the player hook-order crash that could leave a transparent, empty window. Theme backplates and a readable error fallback protect the window.
- Normalizes supported audio formats locally, including Telegram OGG voice notes. Original files are unchanged.
- Rejects malformed, nonfinite, silent and empty transcription results with named errors.
- Makes an explicit short-recording cancellation succeed without requiring captured speech.

## Verification

The signed app and bundled CLI identify as 0.12.2. The stable 0.12.1 rollback is retained. Supported recording, model, dictionary and clipboard preferences are preserved; only retired player settings were removed.

Real saved OGG notes and WAV audio produced text through the candidate CLI. Native layout fixtures cover all fourteen themes. Fixtures and scripted clicks are synthetic; hidden captures do not establish visible motion, GPU efficiency, latency or real key routing.

Live recording and speech-reactive animation acceptance remain blocked because macOS microphone permission is not granted. Allow UltraVox microphone access before those checks. The required Sonnet review has not run successfully, and public release remains held.

Private diagnostic builds can include `local-qa`, but the route additionally requires `ULTRAVOX_LOCAL_QA=1` in both client and app. Normal consumer builds omit the feature. It provides fixed, bounded read-only numeric observations, not arbitrary evaluation or window/input control. Primary image capture is refused to protect private content; visual captures use the isolated native mirror.
