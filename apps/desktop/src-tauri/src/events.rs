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
pub const MEETING_DETECTION_PENDING: &str = "meeting-detection-pending";
pub const MEETING_DETECTION_TTL_MS: u64 = 10 * 60 * 1_000;
pub const MAX_MEETING_DETECTION_ID_BYTES: usize = 128;
pub const MAX_MEETING_DETECTION_SKEW_MS: u64 = 5 * 60 * 1_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MeetingProvider {
    GoogleMeet,
    Zoom,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MeetingDetection {
    pub version: u8,
    pub detection_id: String,
    pub provider: MeetingProvider,
    pub meeting_key: String,
    pub detected_at_ms: u64,
}

impl MeetingDetection {
    pub fn validate(&self, now_ms: u64) -> Result<(), String> {
        if self.version != 1 {
            return Err(format!(
                "unsupported meeting detection version {}; expected 1",
                self.version
            ));
        }
        if self.detection_id.is_empty()
            || self.detection_id.len() > MAX_MEETING_DETECTION_ID_BYTES
            || !self
                .detection_id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b".-_".contains(&byte))
        {
            return Err("meeting detection id is invalid".to_string());
        }
        if self.meeting_key.len() != 64
            || !self
                .meeting_key
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err("meeting key must be 64 lowercase hexadecimal characters".to_string());
        }
        let skew = now_ms.abs_diff(self.detected_at_ms);
        if skew > MAX_MEETING_DETECTION_SKEW_MS {
            return Err(
                "meeting detection timestamp is outside the allowed clock skew".to_string(),
            );
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct MeetingDetectionPendingPayload {
    pub version: u8,
    pub detection_id: String,
    pub provider: MeetingProvider,
    pub detected_at_ms: u64,
    pub expires_at_ms: u64,
}

impl MeetingDetectionPendingPayload {
    pub fn from_detection(detection: &MeetingDetection, expires_at_ms: u64) -> Self {
        Self {
            version: detection.version,
            detection_id: detection.detection_id.clone(),
            provider: detection.provider,
            detected_at_ms: detection.detected_at_ms,
            expires_at_ms,
        }
    }
}

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
#[cfg(test)]
mod tests {
    use super::*;

    fn detection() -> MeetingDetection {
        MeetingDetection {
            version: 1,
            detection_id: "det-1".to_string(),
            provider: MeetingProvider::GoogleMeet,
            meeting_key: "a".repeat(64),
            detected_at_ms: 1_000,
        }
    }

    #[test]
    fn detection_validation_accepts_opaque_key_and_rejects_raw_shapes() {
        assert!(detection().validate(1_000).is_ok());
        let mut invalid = detection();
        invalid.meeting_key = "A".repeat(64);
        assert!(invalid.validate(1_000).is_err());
        invalid = detection();
        invalid.detection_id = "meeting id".to_string();
        assert!(invalid.validate(1_000).is_err());
    }

    #[test]
    fn detection_validation_rejects_clock_skew() {
        assert!(detection()
            .validate(MAX_MEETING_DETECTION_SKEW_MS + 1_001)
            .is_err());
    }

    #[test]
    fn pending_payload_does_not_expose_meeting_key() {
        let payload = MeetingDetectionPendingPayload::from_detection(&detection(), 2_000);
        let value = serde_json::to_value(payload).unwrap();
        assert!(value.get("meeting_key").is_none());
    }
}
