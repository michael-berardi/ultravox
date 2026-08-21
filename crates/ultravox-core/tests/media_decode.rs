//! Media import decoding: every format a user can drop onto UltraVox must
//! decode to a 16 kHz mono WAV with real, non-silent audio content.

use std::path::{Path, PathBuf};
use ultravox_core::{decode_media_file_to_wav, IMPORT_SAMPLE_RATE};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(name)
}

fn decode_to_temp(name: &str) -> (PathBuf, u64) {
    let dest = std::env::temp_dir().join(format!("ultravox-decode-test-{}", name));
    let _ = std::fs::remove_file(&dest);
    let duration_ms =
        decode_media_file_to_wav(&fixture(name), &dest).expect("decode should succeed");
    (dest, duration_ms)
}

fn assert_decoded_wav(name: &str) {
    let (dest, duration_ms) = decode_to_temp(name);
    let _guard = scopeguard(&dest);

    // 3-second fixtures; container padding and codec delay make exact length
    // vary, so assert a generous band around 3000 ms.
    assert!(
        (2_500..=3_500).contains(&duration_ms),
        "{name}: unexpected duration {duration_ms}ms"
    );

    let mut reader = hound::WavReader::open(&dest).expect("output must be a readable WAV");
    let spec = reader.spec();
    assert_eq!(spec.sample_rate, IMPORT_SAMPLE_RATE, "{name}: sample rate");
    assert_eq!(spec.channels, 1, "{name}: channels");
    assert_eq!(
        spec.sample_format,
        hound::SampleFormat::Float,
        "{name}: sample format"
    );
    let max_abs = reader
        .samples::<f32>()
        .take(IMPORT_SAMPLE_RATE as usize * 4)
        .filter_map(Result::ok)
        .fold(0.0f32, |acc, s| acc.max(s.abs()));
    assert!(max_abs > 0.01, "{name}: decoded audio is silent");
}

fn scopeguard(path: &Path) -> impl Drop {
    struct Guard(PathBuf);
    impl Drop for Guard {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }
    Guard(path.to_path_buf())
}

#[test]
fn decodes_wav() {
    assert_decoded_wav("speech.wav");
}

#[test]
fn decodes_mp3() {
    assert_decoded_wav("speech.mp3");
}

#[test]
fn decodes_m4a_aac() {
    assert_decoded_wav("speech.m4a");
}

#[test]
fn decodes_mp4_audio() {
    assert_decoded_wav("speech.mp4");
}

#[test]
fn decodes_flac() {
    assert_decoded_wav("speech.flac");
}

#[test]
fn decodes_ogg_vorbis() {
    assert_decoded_wav("speech.ogg");
}

#[test]
fn decodes_whatsapp_opus() {
    assert_decoded_wav("speech.opus");
}

#[test]
fn decodes_webm_opus() {
    assert_decoded_wav("speech.webm");
}

#[test]
fn rejects_garbage_file() {
    let garbage = std::env::temp_dir().join("ultravox-decode-test-garbage.mp3");
    std::fs::write(&garbage, b"this is not audio at all, just text bytes").unwrap();
    let dest = std::env::temp_dir().join("ultravox-decode-test-garbage.wav");
    let result = decode_media_file_to_wav(&garbage, &dest);
    assert!(result.is_err(), "garbage input must fail");
    assert!(!dest.exists(), "failed decode must not leave an output file");
    let _ = std::fs::remove_file(&garbage);
}

#[test]
fn rejects_missing_file() {
    let dest = std::env::temp_dir().join("ultravox-decode-test-missing.wav");
    let result =
        decode_media_file_to_wav(Path::new("/nonexistent/no-such-file.opus"), &dest);
    assert!(result.is_err());
    assert!(!dest.exists());
}
