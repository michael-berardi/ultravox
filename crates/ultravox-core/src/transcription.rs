use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use thiserror::Error;

/// Errors from the transcription subsystem.
#[derive(Debug, Error)]
pub enum TranscriptionError {
    #[error("model not loaded")]
    ModelNotLoaded,
    #[error("audio decode error: {0}")]
    AudioDecode(String),
    #[error("transcription engine error: {0}")]
    Engine(String),
    #[error("cancelled")]
    Cancelled,
}

/// A single transcription segment with optional timing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Segment {
    pub start_ms: u64,
    pub end_ms: u64,
    pub text: String,
}

/// Result of a transcription job.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TranscriptionResult {
    pub text: String,
    pub segments: Vec<Segment>,
    pub language: Option<String>,
}

/// Request to transcribe an audio source.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TranscriptionRequest {
    pub audio_path: PathBuf,
    pub language: Option<String>,
    pub translate_to_english: bool,
    pub initial_prompt: Option<String>,
    pub temperature: Option<f64>,
    pub suppress_blank_audio: Option<bool>,
    pub show_timestamps: Option<bool>,
    pub use_beam_search: Option<bool>,
    pub beam_size: Option<u32>,
}

/// Abstraction over a transcription engine (Whisper, FluidAudio, etc.).
#[async_trait]
pub trait TranscriptionEngine: Send + Sync {
    /// Human-readable engine identifier.
    fn engine_id(&self) -> &str;

    /// Load the engine with the given model path.
    async fn load(&mut self, model_path: PathBuf) -> Result<(), TranscriptionError>;

    /// Transcribe the given audio file.
    async fn transcribe(
        &self,
        request: TranscriptionRequest,
    ) -> Result<TranscriptionResult, TranscriptionError>;

    /// Unload the engine and free resources.
    async fn unload(&mut self) -> Result<(), TranscriptionError>;
}

/// A stub Whisper engine that compiles but does not actually transcribe.
#[derive(Debug, Default)]
pub struct WhisperEngine {
    model_path: Option<PathBuf>,
}

#[async_trait]
impl TranscriptionEngine for WhisperEngine {
    fn engine_id(&self) -> &str {
        "whisper"
    }

    async fn load(&mut self, model_path: PathBuf) -> Result<(), TranscriptionError> {
        self.model_path = Some(model_path);
        Ok(())
    }

    async fn transcribe(
        &self,
        request: TranscriptionRequest,
    ) -> Result<TranscriptionResult, TranscriptionError> {
        if self.model_path.is_none() {
            return Err(TranscriptionError::ModelNotLoaded);
        }
        // Skeleton: return the audio path as a placeholder text.
        Ok(TranscriptionResult {
            text: format!("(skeleton) {}", request.audio_path.display()),
            segments: vec![],
            language: request.language,
        })
    }

    async fn unload(&mut self) -> Result<(), TranscriptionError> {
        self.model_path = None;
        Ok(())
    }
}

/// A stub FluidAudio engine.
#[derive(Debug, Default)]
pub struct FluidAudioEngine {
    version: Option<String>,
}

#[async_trait]
impl TranscriptionEngine for FluidAudioEngine {
    fn engine_id(&self) -> &str {
        "fluidaudio"
    }

    async fn load(&mut self, model_path: PathBuf) -> Result<(), TranscriptionError> {
        self.version = model_path
            .file_stem()
            .and_then(|s| s.to_str())
            .map(|s| s.to_string());
        Ok(())
    }

    async fn transcribe(
        &self,
        request: TranscriptionRequest,
    ) -> Result<TranscriptionResult, TranscriptionError> {
        if self.version.is_none() {
            return Err(TranscriptionError::ModelNotLoaded);
        }
        Ok(TranscriptionResult {
            text: format!("(skeleton) fluidaudio {}", request.audio_path.display()),
            segments: vec![],
            language: request.language,
        })
    }

    async fn unload(&mut self) -> Result<(), TranscriptionError> {
        self.version = None;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn whisper_engine_load_and_transcribe() {
        let mut engine = WhisperEngine::default();
        engine.load(PathBuf::from("model.bin")).await.unwrap();
        let result = engine
            .transcribe(TranscriptionRequest {
                audio_path: PathBuf::from("audio.wav"),
                language: Some("en".to_string()),
                translate_to_english: false,
                initial_prompt: None,
                temperature: None,
                suppress_blank_audio: None,
                show_timestamps: None,
                use_beam_search: None,
                beam_size: None,
            })
            .await
            .unwrap();
        assert!(result.text.contains("audio.wav"));
    }

    #[tokio::test]
    async fn engine_unload_clears_state() {
        let mut engine = WhisperEngine::default();
        engine.load(PathBuf::from("model.bin")).await.unwrap();
        engine.unload().await.unwrap();
        let result = engine
            .transcribe(TranscriptionRequest {
                audio_path: PathBuf::from("audio.wav"),
                language: None,
                translate_to_english: false,
                initial_prompt: None,
                temperature: None,
                suppress_blank_audio: None,
                show_timestamps: None,
                use_beam_search: None,
                beam_size: None,
            })
            .await;
        assert!(matches!(result, Err(TranscriptionError::ModelNotLoaded)));
    }
}
