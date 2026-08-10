use serde::{Deserialize, Serialize};

use ultravox_core::{AppConfig, AudioRecording, RecordingRow};

pub const RECORDING_STARTED: &str = "recording-started";
pub const RECORDING_STOPPED: &str = "recording-stopped";
pub const TRANSCRIPTION_PROGRESS: &str = "transcription-progress";
pub const TRANSCRIPTION_COMPLETED: &str = "transcription-completed";
pub const SHORTCUT_TRIGGERED: &str = "shortcut-triggered";
pub const INDICATOR_SHOW: &str = "indicator-show";
pub const INDICATOR_HIDE: &str = "indicator-hide";
pub const SETTINGS_CHANGED: &str = "settings-changed";
pub const RECORDING_ADDED: &str = "recording-added";
pub const RECORDING_DELETED: &str = "recording-deleted";
pub const MEETING_STATE_CHANGED: &str = "meeting-state-changed";
pub const URL_IMPORT_PROGRESS: &str = "url-import-progress";

#[derive(Debug, Clone, Serialize)]
pub struct RecordingStartedPayload {
    pub recording_id: String,
    pub start_time_ms: u64,
}

impl From<&AudioRecording> for RecordingStartedPayload {
    fn from(rec: &AudioRecording) -> Self {
        Self {
            recording_id: rec.id.clone(),
            start_time_ms: rec.start_time_ms,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct RecordingStoppedPayload {
    pub recording_id: String,
    pub output_path: String,
    pub duration_ms: Option<u64>,
}

impl From<&AudioRecording> for RecordingStoppedPayload {
    fn from(rec: &AudioRecording) -> Self {
        Self {
            recording_id: rec.id.clone(),
            output_path: rec.output_path.to_string_lossy().to_string(),
            duration_ms: rec.duration_ms,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct TranscriptionProgressPayload {
    pub recording_id: String,
    pub progress: f32,
    pub status: String,
}

impl TranscriptionProgressPayload {
    pub fn new(recording_id: impl Into<String>, progress: f32, status: impl Into<String>) -> Self {
        Self {
            recording_id: recording_id.into(),
            progress,
            status: status.into(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct UrlImportProgressPayload {
    pub progress: f32,
    pub status: String,
}

impl UrlImportProgressPayload {
    pub fn new(progress: f32, status: impl Into<String>) -> Self {
        Self {
            progress,
            status: status.into(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct TranscriptionCompletedPayload {
    pub recording_id: String,
    pub text: String,
    pub language: Option<String>,
}

impl TranscriptionCompletedPayload {
    pub fn new(
        recording_id: impl Into<String>,
        text: impl Into<String>,
        language: Option<impl Into<String>>,
    ) -> Self {
        Self {
            recording_id: recording_id.into(),
            text: text.into(),
            language: language.map(Into::into),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShortcutTriggeredPayload {
    pub shortcut: String,
}

impl ShortcutTriggeredPayload {
    pub fn new(shortcut: impl Into<String>) -> Self {
        Self {
            shortcut: shortcut.into(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct IndicatorShowPayload {
    pub x: f64,
    pub y: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct IndicatorHidePayload;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SettingsChangedPayload {
    pub config: AppConfig,
}

impl From<&AppConfig> for SettingsChangedPayload {
    fn from(config: &AppConfig) -> Self {
        Self {
            config: config.clone(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct RecordingAddedPayload {
    pub recording: RecordingRow,
}

impl From<&RecordingRow> for RecordingAddedPayload {
    fn from(recording: &RecordingRow) -> Self {
        Self {
            recording: recording.clone(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct RecordingDeletedPayload {
    pub id: String,
}

impl From<uuid::Uuid> for RecordingDeletedPayload {
    fn from(id: uuid::Uuid) -> Self {
        Self { id: id.to_string() }
    }
}

impl From<&uuid::Uuid> for RecordingDeletedPayload {
    fn from(id: &uuid::Uuid) -> Self {
        Self { id: id.to_string() }
    }
}
