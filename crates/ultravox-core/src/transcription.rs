use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use thiserror::Error;

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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Segment {
    pub start_ms: u64,
    pub end_ms: u64,
    pub text: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TranscriptionResult {
    pub text: String,
    pub segments: Vec<Segment>,
    pub language: Option<String>,
}
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

#[async_trait]
pub trait TranscriptionEngine: Send + Sync {
    fn engine_id(&self) -> &str;
    async fn load(&mut self, model_path: PathBuf) -> Result<(), TranscriptionError>;
    async fn transcribe(
        &self,
        request: TranscriptionRequest,
    ) -> Result<TranscriptionResult, TranscriptionError>;
    async fn unload(&mut self) -> Result<(), TranscriptionError>;
}
