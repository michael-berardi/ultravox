//! Validated engine input, independent of the caller's filename extension.

use crate::audio::{decode_media_file_to_wav, AudioError, IMPORT_MAX_BYTES, IMPORT_SAMPLE_RATE};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

/// A caller-owned WAV or a private temporary normalized WAV. Dropping this
/// value removes only the temporary directory, never the caller's source.
#[derive(Debug)]
pub struct TranscriptionAudio {
    path: PathBuf,
    temporary_dir: Option<PathBuf>,
}

impl TranscriptionAudio {
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TranscriptionAudio {
    fn drop(&mut self) {
        if let Some(dir) = &self.temporary_dir {
            let _ = std::fs::remove_dir_all(dir);
        }
    }
}

/// Strict RIFF validation shared by transcription and direct media imports.
/// Container size, complete chunks, and sample-frame boundaries must all agree.
fn validate_riff_layout(path: &Path) -> Result<(), AudioError> {
    let invalid = |cause: &str| AudioError::Format(format!("invalid WAV: {cause}"));
    let mut file = std::fs::File::open(path)?;
    let mut header = [0u8; 12];
    file.read_exact(&mut header)
        .map_err(|_| invalid("truncated RIFF header"))?;
    let end = u64::from(u32::from_le_bytes(header[4..8].try_into().unwrap())) + 8;
    if end != file.metadata()?.len() || end < 12 {
        return Err(invalid("RIFF size does not match file length"));
    }
    let mut position = 12u64;
    let mut block_align = None;
    while position < end {
        if end - position < 8 {
            return Err(invalid("truncated chunk header"));
        }
        let mut chunk = [0u8; 8];
        file.read_exact(&mut chunk)?;
        let size = u64::from(u32::from_le_bytes(chunk[4..8].try_into().unwrap()));
        let next = position + 8 + size + (size & 1);
        if next > end {
            return Err(invalid("truncated chunk data"));
        }
        if &chunk[..4] == b"fmt " {
            if size < 16 {
                return Err(invalid("incomplete format chunk"));
            }
            let mut fmt = [0u8; 16];
            file.read_exact(&mut fmt)?;
            let align = u16::from_le_bytes(fmt[12..14].try_into().unwrap());
            if align == 0 {
                return Err(invalid("zero sample-frame alignment"));
            }
            block_align = Some(u64::from(align));
        } else if &chunk[..4] == b"data" {
            let align = block_align.ok_or_else(|| invalid("data precedes format chunk"))?;
            if size % align != 0 {
                return Err(invalid("incomplete audio frame"));
            }
        }
        file.seek(SeekFrom::Start(next))?;
        position = next;
    }
    Ok(())
}

/// WAV identification is based on bytes, not an untrusted filename extension.
/// Non-RIFF containers are left to the media decoder.
pub(crate) fn validate_wav_if_riff(path: &Path) -> Result<Option<hound::WavSpec>, AudioError> {
    let mut file = std::fs::File::open(path)?;
    let mut magic = [0u8; 12];
    let size = file.read(&mut magic)?;
    if size >= 4 && &magic[..4] == b"RIFF" {
        if size < 12 || &magic[8..12] != b"WAVE" {
            return Err(AudioError::Format(
                "invalid WAV: missing WAVE header".into(),
            ));
        }
        return validate_wav(path).map(Some);
    }
    Ok(None)
}

/// Check the entire WAV before handing it to a model: headers alone do not
/// detect truncated sample data, empty data chunks, or non-finite float audio.
fn validate_wav(path: &Path) -> Result<hound::WavSpec, AudioError> {
    validate_riff_layout(path)?;
    let invalid = |cause: String| AudioError::Format(format!("invalid WAV: {cause}"));
    let mut reader = hound::WavReader::open(path).map_err(|e| invalid(e.to_string()))?;
    let spec = reader.spec();
    if spec.channels == 0 || spec.sample_rate == 0 {
        return Err(invalid(
            "sample rate and channel count must be nonzero".into(),
        ));
    }
    let expected = reader.len();
    if expected == 0 {
        return Err(invalid("audio has no samples".into()));
    }
    if expected % u32::from(spec.channels) != 0 {
        return Err(invalid("incomplete audio frame".into()));
    }
    let mut count = 0u32;
    match spec.sample_format {
        hound::SampleFormat::Float if spec.bits_per_sample == 32 => {
            for sample in reader.samples::<f32>() {
                let sample = sample.map_err(|e| invalid(e.to_string()))?;
                if !sample.is_finite() {
                    return Err(invalid("audio contains non-finite samples".into()));
                }
                count += 1;
            }
        }
        hound::SampleFormat::Int if matches!(spec.bits_per_sample, 8 | 16 | 24 | 32) => {
            for sample in reader.samples::<i32>() {
                sample.map_err(|e| invalid(e.to_string()))?;
                count += 1;
            }
        }
        _ => return Err(invalid(format!("unsupported sample format: {spec:?}"))),
    }
    if count != expected {
        return Err(invalid(format!(
            "truncated sample data ({count}/{expected} samples)"
        )));
    }
    Ok(spec)
}

/// Identify RIFF/WAVE by magic bytes, validate it, and normalize other media
/// (including Ogg/Opus voice notes) or noncanonical WAV to mono 16 kHz PCM16.
/// The source is opened read-only; a random, exclusively created directory
/// holds derived audio for the lifetime of the returned guard.
pub fn prepare_transcription_audio(source: &Path) -> Result<TranscriptionAudio, AudioError> {
    let result = (|| {
        let metadata = std::fs::metadata(source)?;
        if !metadata.is_file() {
            return Err(AudioError::Format(
                "input is not a regular audio file".into(),
            ));
        }
        if metadata.len() == 0 {
            return Err(AudioError::Format("audio input is empty".into()));
        }
        if metadata.len() > IMPORT_MAX_BYTES {
            return Err(AudioError::Format(
                "audio input exceeds the 2 GB limit".into(),
            ));
        }
        if let Some(spec) = validate_wav_if_riff(source)? {
            if spec.channels == 1
                && spec.sample_rate == IMPORT_SAMPLE_RATE
                && spec.sample_format == hound::SampleFormat::Int
                && spec.bits_per_sample == 16
            {
                return Ok(TranscriptionAudio {
                    path: source.to_path_buf(),
                    temporary_dir: None,
                });
            }
        }
        let dir = std::env::temp_dir().join(format!("ultravox-audio-{}", uuid::Uuid::new_v4()));
        let mut builder = std::fs::DirBuilder::new();
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        builder.create(&dir)?;
        let audio = TranscriptionAudio {
            path: dir.join("input.wav"),
            temporary_dir: Some(dir),
        };
        decode_media_file_to_wav(source, audio.path())?;
        validate_wav(audio.path())?;
        Ok(audio)
    })();
    result.map_err(|e: AudioError| AudioError::Format(format!("{}: {e}", source.display())))
}
