//! UltraVox core library.
//!
//! Provides platform-agnostic domain types and services used by the Tauri
//! desktop app and future native tooling: settings/config, model catalog,
//! recording history, model download metadata/progress, transcription engine
//! traits, and audio recording abstractions.

pub mod audio;
pub mod config;
pub mod download;
pub mod history;
pub mod model_catalog;
pub mod shortcuts;
pub mod transcription;

pub use audio::{
    decode_media_file_to_wav, AudioBackend, AudioDeviceInfo, AudioError, AudioInputConfig,
    AudioRecording, CpalAudioBackend, StubAudioBackend, IMPORT_MAX_BYTES, IMPORT_SAMPLE_RATE,
};
pub use config::{AppConfig, ConfigError, ConfigManager, Engine, Language};
pub use download::{
    DownloadError, DownloadHandle, DownloadManager, DownloadProgress, DownloadState, ModelDownload,
};
pub use history::{HistoryError, RecordingHistory, RecordingRow, RecordingStatus};
pub use model_catalog::{ModelCatalog, ModelEntry, ModelFamily, ModelVersion};
pub use shortcuts::{ModifierKey, ShortcutSettings};
pub use transcription::{
    Segment, TranscriptionEngine, TranscriptionError, TranscriptionRequest, TranscriptionResult,
};
