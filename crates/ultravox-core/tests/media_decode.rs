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
        hound::SampleFormat::Int,
        "{name}: sample format"
    );
    assert_eq!(spec.bits_per_sample, 16, "{name}: PCM16");
    let max_abs = reader
        .samples::<i16>()
        .take(IMPORT_SAMPLE_RATE as usize * 4)
        .filter_map(Result::ok)
        .fold(0.0f32, |acc, s| acc.max((s as f32 / 32768.0).abs()));
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
    assert!(
        !dest.exists(),
        "failed decode must not leave an output file"
    );
    let _ = std::fs::remove_file(&garbage);
}

#[test]
fn rejects_missing_file() {
    let dest = std::env::temp_dir().join("ultravox-decode-test-missing.wav");
    let result = decode_media_file_to_wav(Path::new("/nonexistent/no-such-file.opus"), &dest);
    assert!(result.is_err());
    assert!(!dest.exists());
}

/// A cut-off container page is not a normal EOF and must not become partial success.
#[test]
fn truncated_ogg_opus_does_not_salvage_unexpected_eof() {
    for name in ["speech.opus", "speech.ogg"] {
        let bytes = std::fs::read(fixture(name)).unwrap();
        let cut =
            std::env::temp_dir().join(format!("ultravox-truncated-{}-{name}", std::process::id()));
        std::fs::write(&cut, &bytes[..bytes.len() - 37]).unwrap();
        let dest = std::env::temp_dir().join(format!(
            "ultravox-truncated-{}-{name}.wav",
            std::process::id()
        ));
        let _cut_guard = scopeguard(&cut);
        let _dest_guard = scopeguard(&dest);
        let error = decode_media_file_to_wav(&cut, &dest).unwrap_err();
        assert!(error.to_string().contains("invalid OGG"), "{name}: {error}");
        assert!(!dest.exists(), "{name}: partial output left behind");
    }
}

#[test]
fn complete_ogg_without_eos_decodes_but_bad_checksums_do_not() {
    // Synthetic container variants from public speech fixtures. Telegram's
    // complete pages without an EOS flag exercise this same EOF path.
    for name in ["speech.ogg", "speech.opus"] {
        let mut bytes = std::fs::read(fixture(name)).unwrap();
        let mut offset = 0;
        while offset < bytes.len() {
            assert_eq!(&bytes[offset..offset + 4], b"OggS");
            let segments = bytes[offset + 26] as usize;
            let page_size = 27
                + segments
                + bytes[offset + 27..offset + 27 + segments]
                    .iter()
                    .map(|v| *v as usize)
                    .sum::<usize>();
            bytes[offset + 5] &= !4;
            bytes[offset + 22..offset + 26].fill(0);
            let mut crc = 0u32;
            for byte in &bytes[offset..offset + page_size] {
                crc ^= (*byte as u32) << 24;
                for _ in 0..8 {
                    crc = if crc & 0x8000_0000 != 0 {
                        (crc << 1) ^ 0x04c1_1db7
                    } else {
                        crc << 1
                    };
                }
            }
            bytes[offset + 22..offset + 26].copy_from_slice(&crc.to_le_bytes());
            offset += page_size;
        }
        let root = std::env::temp_dir().join(format!("ultravox-no-eos-{}", uuid::Uuid::new_v4()));
        let source = root.with_extension("ogg");
        let dest = root.with_extension("wav");
        let _source_guard = scopeguard(&source);
        let _dest_guard = scopeguard(&dest);
        std::fs::write(&source, &bytes).unwrap();
        assert!(decode_media_file_to_wav(&source, &dest).unwrap() > 2_500);
        assert_eq!(std::fs::read(&source).unwrap(), bytes);
        std::fs::remove_file(&dest).unwrap();
        bytes.last_mut().map(|last| *last ^= 1);
        std::fs::write(&source, &bytes).unwrap();
        let error = decode_media_file_to_wav(&source, &dest).unwrap_err();
        assert!(error.to_string().contains("checksum mismatch"), "{error}");
        assert!(!dest.exists());
        assert_eq!(std::fs::read(&source).unwrap(), bytes);
    }
}

#[test]
fn normalization_never_overwrites_source_or_existing_output() {
    let root = std::env::temp_dir().join(format!("ultravox-preserve-{}", uuid::Uuid::new_v4()));
    let source = root.with_extension("wav");
    let dest = root.with_extension("output.wav");
    let _source_guard = scopeguard(&source);
    let _dest_guard = scopeguard(&dest);
    let original = std::fs::read(fixture("speech.wav")).unwrap();
    std::fs::write(&source, &original).unwrap();
    assert!(decode_media_file_to_wav(&source, &source).is_err());
    assert_eq!(std::fs::read(&source).unwrap(), original);
    std::fs::write(&dest, b"existing caller-owned output").unwrap();
    assert!(decode_media_file_to_wav(&source, &dest).is_err());
    assert_eq!(
        std::fs::read(&dest).unwrap(),
        b"existing caller-owned output"
    );
    assert_eq!(std::fs::read(&source).unwrap(), original);
    #[cfg(unix)]
    {
        std::fs::remove_file(&dest).unwrap();
        std::fs::hard_link(&source, &dest).unwrap();
        assert!(decode_media_file_to_wav(&source, &dest).is_err());
        assert_eq!(std::fs::read(&source).unwrap(), original);
        assert_eq!(std::fs::read(&dest).unwrap(), original);
    }
}

#[test]
fn malformed_wav_imports_fail_and_preserve_source() {
    let speech = std::fs::read(fixture("speech.wav")).unwrap();
    let mut partial_frame = speech.clone();
    partial_frame.pop();
    let riff_size = (partial_frame.len() - 8) as u32;
    partial_frame[4..8].copy_from_slice(&riff_size.to_le_bytes());
    let data_size = (partial_frame.len() - 44) as u32;
    partial_frame[40..44].copy_from_slice(&data_size.to_le_bytes());
    // Supply the RIFF odd-byte pad so only the incomplete PCM frame is invalid.
    partial_frame.push(0);
    let riff_size = (partial_frame.len() - 8) as u32;
    partial_frame[4..8].copy_from_slice(&riff_size.to_le_bytes());
    for (name, bytes) in [
        ("truncated", speech[..speech.len() - 37].to_vec()),
        ("header-only", speech[..44].to_vec()),
        ("partial-frame", partial_frame),
        ("short-riff", b"RIFF".to_vec()),
    ] {
        let root = std::env::temp_dir().join(format!("ultravox-import-{}", uuid::Uuid::new_v4()));
        let source = root.with_extension("ogg"); // RIFF magic must override extension.
        let dest = root.with_extension("wav");
        let _source_guard = scopeguard(&source);
        let _dest_guard = scopeguard(&dest);
        std::fs::write(&source, &bytes).unwrap();
        let error = decode_media_file_to_wav(&source, &dest).unwrap_err();
        assert!(error.to_string().contains("invalid WAV"), "{name}: {error}");
        assert!(!dest.exists(), "{name}: partial output left behind");
        assert_eq!(std::fs::read(&source).unwrap(), bytes);
    }
}

#[test]
fn nonfinite_float_wav_imports_fail() {
    for sample in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        let root =
            std::env::temp_dir().join(format!("ultravox-nonfinite-{}", uuid::Uuid::new_v4()));
        let source = root.with_extension("wav");
        let dest = root.with_extension("normalized.wav");
        let _source_guard = scopeguard(&source);
        let _dest_guard = scopeguard(&dest);
        let mut wav = hound::WavWriter::create(
            &source,
            hound::WavSpec {
                channels: 1,
                sample_rate: 48_000,
                bits_per_sample: 32,
                sample_format: hound::SampleFormat::Float,
            },
        )
        .unwrap();
        wav.write_sample(sample).unwrap();
        wav.finalize().unwrap();
        let original = std::fs::read(&source).unwrap();
        let error = decode_media_file_to_wav(&source, &dest).unwrap_err();
        assert!(error.to_string().contains("non-finite"), "{error}");
        assert!(!dest.exists());
        assert_eq!(std::fs::read(&source).unwrap(), original);
    }
}

#[test]
fn decoder_rejects_nonfinite_float_caf_before_pcm_cast() {
    // Non-RIFF float PCM exercises the actual decoder finite check, not WAV validation.
    for sample in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        let mut caf = b"caff\x00\x01\x00\x00desc".to_vec();
        caf.extend_from_slice(&32i64.to_be_bytes());
        caf.extend_from_slice(&16_000f64.to_be_bytes());
        caf.extend_from_slice(b"lpcm");
        for field in [1u32, 4, 1, 1, 32] {
            // float, bytes/packet, frames/packet, channels, bits
            caf.extend_from_slice(&field.to_be_bytes());
        }
        caf.extend_from_slice(b"data");
        caf.extend_from_slice(&12i64.to_be_bytes());
        caf.extend_from_slice(&0u32.to_be_bytes()); // edit count
        caf.extend_from_slice(&0.25f32.to_be_bytes());
        caf.extend_from_slice(&sample.to_be_bytes());
        let root = std::env::temp_dir().join(format!("ultravox-caf-{}", uuid::Uuid::new_v4()));
        let source = root.with_extension("caf");
        let dest = root.with_extension("wav");
        let _source_guard = scopeguard(&source);
        let _dest_guard = scopeguard(&dest);
        std::fs::write(&source, &caf).unwrap();
        let error = decode_media_file_to_wav(&source, &dest).unwrap_err();
        assert!(error.to_string().contains("non-finite"), "{error}");
        assert!(!dest.exists());
        assert_eq!(std::fs::read(&source).unwrap(), caf);
    }
}
