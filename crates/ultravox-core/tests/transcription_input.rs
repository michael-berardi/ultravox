//! Validation requires no model. Ignored inference gates require explicit model/private
//! paths and fail (never silently skip) when run with missing prerequisites.
use std::path::{Path, PathBuf};
use ultravox_core::{phonon, prepare_transcription_audio};

struct Workspace(PathBuf);
impl Workspace {
    fn new() -> Self {
        let path =
            std::env::temp_dir().join(format!("ultravox-input-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn path(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}
impl Drop for Workspace {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}
fn assert_pcm16(path: &Path) {
    let mut wav = hound::WavReader::open(path).unwrap();
    let spec = wav.spec();
    assert_eq!(
        (spec.sample_rate, spec.channels, spec.bits_per_sample),
        (16_000, 1, 16)
    );
    assert_eq!(spec.sample_format, hound::SampleFormat::Int);
    assert!(wav
        .samples::<i16>()
        .any(|s| s.unwrap().unsigned_abs() > 300));
}

#[test]
fn ogg_magic_normalizes_despite_wav_extension_and_preserves_source() {
    let workspace = Workspace::new();
    for name in ["speech.ogg", "speech.opus"] {
        let original = std::fs::read(fixture(name)).unwrap();
        let source = workspace.path(&format!("{name}.wav"));
        std::fs::write(&source, &original).unwrap();
        let input = prepare_transcription_audio(&source).unwrap();
        assert_ne!(input.path(), source);
        assert_pcm16(input.path());
        assert_eq!(std::fs::read(&source).unwrap(), original);
        let normalized = input.path().to_path_buf();
        let directory = normalized.parent().unwrap().to_path_buf();
        drop(input);
        assert!(!normalized.exists());
        assert!(!directory.exists());
        assert_eq!(std::fs::read(&source).unwrap(), original);
    }
}

#[test]
fn wav_magic_loads_despite_ogg_extension_without_removing_source() {
    let workspace = Workspace::new();
    let original = std::fs::read(fixture("speech.wav")).unwrap();
    let source = workspace.path("actually-wav.ogg");
    std::fs::write(&source, &original).unwrap();
    let input = prepare_transcription_audio(&source).unwrap();
    assert_eq!(input.path(), source);
    assert_pcm16(input.path());
    drop(input);
    assert_eq!(std::fs::read(&source).unwrap(), original);
}

#[test]
fn noncanonical_wav_normalizes_to_mono_16k_pcm() {
    let workspace = Workspace::new();
    let source = workspace.path("stereo-48k.wav");
    let mut wav = hound::WavWriter::create(
        &source,
        hound::WavSpec {
            channels: 2,
            sample_rate: 48_000,
            bits_per_sample: 32,
            sample_format: hound::SampleFormat::Float,
        },
    )
    .unwrap();
    for i in 0..48_000 {
        let sample = (i as f32 * 0.02).sin() * 0.2;
        wav.write_sample(sample).unwrap();
        wav.write_sample(sample).unwrap();
    }
    wav.finalize().unwrap();
    let original = std::fs::read(&source).unwrap();
    let input = prepare_transcription_audio(&source).unwrap();
    assert_ne!(input.path(), source);
    assert_pcm16(input.path());
    assert_eq!(std::fs::read(&source).unwrap(), original);
}

#[test]
fn malformed_inputs_error_before_model_loading_and_preserve_bytes() {
    let workspace = Workspace::new();
    let speech = std::fs::read(fixture("speech.wav")).unwrap();
    let opus = std::fs::read(fixture("speech.opus")).unwrap();
    for (name, bytes, cause) in [
        ("empty.ogg", Vec::new(), "empty"),
        (
            "garbage.wav",
            b"this is not audio".to_vec(),
            "unsupported or unreadable",
        ),
        ("short-riff.wav", b"RIFF".to_vec(), "missing WAVE header"),
        (
            "truncated.wav",
            speech[..speech.len() - 37].to_vec(),
            "invalid WAV",
        ),
        (
            "truncated.ogg",
            opus[..20].to_vec(),
            "invalid OGG",
        ),
        ("header-only.wav", speech[..44].to_vec(), "invalid WAV"),
    ] {
        let source = workspace.path(name);
        std::fs::write(&source, &bytes).unwrap();
        let error = prepare_transcription_audio(&source)
            .unwrap_err()
            .to_string();
        assert!(error.contains(cause), "{name}: {error}");
        assert!(error.contains(name), "missing source name: {error}");
        let error = phonon::transcribe_file(&workspace.path("no-model"), &source, None)
            .unwrap_err()
            .to_string();
        assert!(error.contains(cause), "{name}: {error}");
        eprintln!("{name}: {error}");
        assert_eq!(std::fs::read(&source).unwrap(), bytes);
    }
}

#[test]
fn silent_wav_fails_before_model_loading() {
    let workspace = Workspace::new();
    let source = workspace.path("silent.wav");
    let mut wav = hound::WavWriter::create(
        &source,
        hound::WavSpec {
            channels: 1,
            sample_rate: 16_000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        },
    )
    .unwrap();
    for _ in 0..16_000 {
        wav.write_sample(0i16).unwrap();
    }
    wav.finalize().unwrap();
    let original = std::fs::read(&source).unwrap();
    let models = workspace.path("no-installed-model");
    assert!(!phonon::is_installed(&models));
    let error = phonon::transcribe_file(&models, &source, None).unwrap_err();
    assert!(matches!(error, phonon::PhononError::Audio(_)), "{error}");
    assert!(error.to_string().contains("no speech"), "{error}");
    assert!(error.to_string().contains("silent"), "{error}");
    assert_eq!(std::fs::read(&source).unwrap(), original);
}

#[test]
fn rejects_nonfinite_and_empty_wav_samples() {
    let workspace = Workspace::new();
    for (name, sample, cause) in [
        ("nan.wav", Some(f32::NAN), "non-finite"),
        ("empty.wav", None, "no samples"),
    ] {
        let source = workspace.path(name);
        let mut wav = hound::WavWriter::create(
            &source,
            hound::WavSpec {
                channels: 1,
                sample_rate: 16_000,
                bits_per_sample: 32,
                sample_format: hound::SampleFormat::Float,
            },
        )
        .unwrap();
        if let Some(sample) = sample {
            wav.write_sample(sample).unwrap();
        }
        wav.finalize().unwrap();
        assert!(prepare_transcription_audio(&source)
            .unwrap_err()
            .to_string()
            .contains(cause));
    }
}

#[test]
#[ignore = "required inference gate: set ULTRAVOX_MODELS_DIR and run with --ignored"]
fn valid_wav_ogg_and_opus_produce_real_text_without_changing_inputs() {
    let models = model_dir();
    for name in ["speech.wav", "speech.ogg", "speech.opus"] {
        let source = fixture(name);
        let original = std::fs::read(&source).unwrap();
        let original_hash = sha256(&original);
        let text = phonon::transcribe_file(&models, &source, None).unwrap();
        eprintln!("{name}: {text}");
        assert!(
            text.to_lowercase().contains("fellow americans"),
            "{name}: {text}"
        );
        let after = std::fs::read(&source).unwrap();
        assert_eq!(sha256(&after), original_hash);
        assert_eq!(after, original);
        eprintln!("{name}: SHA-256 {original_hash} preserved");
    }
}

fn sha256(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::digest(bytes))
}

fn model_dir() -> PathBuf {
    let dir = PathBuf::from(
        std::env::var_os("ULTRAVOX_MODELS_DIR")
            .expect("required inference gate: set ULTRAVOX_MODELS_DIR"),
    );
    assert!(
        phonon::is_installed(&dir),
        "required inference gate: Phonon-2 missing at {}",
        dir.display()
    );
    dir
}

#[test]
#[ignore = "required private inference gate: set ULTRAVOX_MODELS_DIR and ULTRAVOX_PRIVATE_OGG_DIR; run with --ignored"]
fn private_telegram_ogg_notes_normalize_and_transcribe() {
    let directory = std::env::var_os("ULTRAVOX_PRIVATE_OGG_DIR")
        .expect("required private inference gate: set ULTRAVOX_PRIVATE_OGG_DIR");
    let models = model_dir();
    let mut count = 0;
    for entry in std::fs::read_dir(directory).unwrap() {
        let source = entry.unwrap().path();
        if source.extension().and_then(|e| e.to_str()) != Some("ogg") {
            continue;
        }
        count += 1;
        let original = std::fs::read(&source).unwrap();
        let original_hash = sha256(&original);
        let input = prepare_transcription_audio(&source).unwrap();
        assert_pcm16(input.path());
        let text = phonon::transcribe_file(&models, &source, None).unwrap();
        assert!(
            !text.trim().is_empty(),
            "{}: empty transcript",
            source.display()
        );
        eprintln!("{}: {text}", source.display());
        let after = std::fs::read(&source).unwrap();
        assert_eq!(sha256(&after), original_hash);
        assert_eq!(after, original);
        eprintln!("{}: SHA-256 {original_hash} preserved", source.display());
    }
    assert!(count > 0, "no private OGG notes found");
}
