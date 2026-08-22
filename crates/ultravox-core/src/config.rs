use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use thiserror::Error;

/// Errors returned by the configuration subsystem.
#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("serialization error: {0}")]
    Serialize(#[from] toml::ser::Error),
    #[error("deserialization error: {0}")]
    Deserialize(#[from] toml::de::Error),
    #[error("missing application directory")]
    MissingAppDir,
}

/// Active transcription backend engine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Engine {
    Whisper,
    FluidAudio,
}

impl Default for Engine {
    fn default() -> Self {
        Engine::FluidAudio
    }
}

/// Per-language selection for Whisper-based transcription.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Language(pub String);

impl Default for Language {
    fn default() -> Self {
        Language("en".to_string())
    }
}

/// Top-level application settings persisted across launches.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AppConfig {
    pub selected_engine: Engine,
    pub fluid_audio_model_version: String,
    pub selected_whisper_model_path: Option<PathBuf>,
    pub models_directory: Option<PathBuf>,
    pub whisper_language: Language,
    pub translate_to_english: bool,
    pub suppress_blank_audio: bool,
    pub show_timestamps: bool,
    pub temperature: f64,
    pub no_speech_threshold: f64,
    pub initial_prompt: String,
    pub use_beam_search: bool,
    pub beam_size: u32,
    pub debug_mode: bool,
    pub play_sound_on_record_start: bool,
    pub use_asian_autocorrect: bool,
    pub modifier_only_hotkey: String,
    pub key_combination: String,
    pub hold_to_record: bool,
    #[serde(default = "default_meeting_key_combination")]
    pub meeting_key_combination: String,
    #[serde(default = "default_meeting_detection_enabled")]
    pub meeting_detection_enabled: bool,
    pub add_space_after_sentence: bool,
    pub auto_copy_to_clipboard: bool,
    pub auto_paste_transcription: bool,
    #[serde(default)]
    pub onboarding_completed: bool,
    #[serde(default = "default_model_language")]
    pub model_language: String,
    #[serde(default = "default_theme")]
    pub theme: String,
    #[serde(default = "default_media_panel_enabled")]
    pub media_panel_enabled: bool,
}

fn default_theme() -> String {
    "midnight".to_string()
}

fn default_model_language() -> String {
    "english".to_string()
}

fn default_meeting_key_combination() -> String {
    "Control+M".to_string()
}
fn default_meeting_detection_enabled() -> bool {
    true
}

fn default_media_panel_enabled() -> bool {
    true
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            selected_engine: Engine::default(),
            fluid_audio_model_version: "v2".to_string(),
            selected_whisper_model_path: None,
            models_directory: None,
            whisper_language: Language::default(),
            translate_to_english: false,
            suppress_blank_audio: true,
            show_timestamps: false,
            temperature: 0.0,
            no_speech_threshold: 0.6,
            initial_prompt: String::new(),
            use_beam_search: false,
            beam_size: 5,
            debug_mode: false,
            play_sound_on_record_start: true,
            use_asian_autocorrect: false,
            modifier_only_hotkey: "none".to_string(),
            key_combination: "Option+Backtick".to_string(),
            hold_to_record: false,
            meeting_key_combination: default_meeting_key_combination(),
            meeting_detection_enabled: default_meeting_detection_enabled(),
            add_space_after_sentence: true,
            auto_copy_to_clipboard: true,
            auto_paste_transcription: true,
            onboarding_completed: false,
            model_language: default_model_language(),
            theme: default_theme(),
            media_panel_enabled: default_media_panel_enabled(),
        }
    }
}

/// Manages loading, saving, and updating `AppConfig`.
#[derive(Debug)]
pub struct ConfigManager {
    config_path: PathBuf,
    config: AppConfig,
}

impl ConfigManager {
    const FILE_NAME: &'static str = "settings.toml";

    /// Load or create the configuration in the given application directory.
    pub fn new(app_dir: impl AsRef<Path>) -> Result<Self, ConfigError> {
        let app_dir = app_dir.as_ref();
        std::fs::create_dir_all(app_dir)?;
        let config_path = app_dir.join(Self::FILE_NAME);
        let config = if config_path.exists() {
            let contents = std::fs::read_to_string(&config_path)?;
            toml::from_str(&contents)?
        } else {
            AppConfig::default()
        };
        Ok(Self {
            config_path,
            config,
        })
    }

    /// Read the current configuration.
    pub fn get(&self) -> &AppConfig {
        &self.config
    }

    /// Replace the current configuration and persist it.
    pub fn set(&mut self, config: AppConfig) -> Result<(), ConfigError> {
        self.config = config;
        self.save()
    }

    /// Persist the current configuration to disk.
    pub fn save(&self) -> Result<(), ConfigError> {
        let contents = toml::to_string_pretty(&self.config)?;
        std::fs::write(&self.config_path, contents)?;
        Ok(())
    }

    /// Mutable access to the current configuration; callers must call `save`.
    pub fn mutate(&mut self) -> &mut AppConfig {
        &mut self.config
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_engine_is_fluidaudio_v2() {
        let cfg = AppConfig::default();
        assert!(matches!(cfg.selected_engine, Engine::FluidAudio));
        assert_eq!(cfg.fluid_audio_model_version, "v2");
    }

    #[test]
    fn default_settings_match_swift_baseline() {
        let cfg = AppConfig::default();
        assert!(!cfg.hold_to_record);
        assert!(cfg.auto_copy_to_clipboard);
        assert!(cfg.auto_paste_transcription);
        assert!(cfg.add_space_after_sentence);
        assert_eq!(cfg.whisper_language.0, "en");
        assert_eq!(cfg.key_combination, "Option+Backtick");
        assert_eq!(cfg.meeting_key_combination, "Control+M");
        assert!(cfg.meeting_detection_enabled);
        assert!(!cfg.onboarding_completed);
        assert_eq!(cfg.model_language, "english");
        assert!(cfg.media_panel_enabled);
    }

    #[test]
    fn roundtrip_toml_preserves_every_setting() {
        let cfg = AppConfig {
            selected_engine: Engine::Whisper,
            fluid_audio_model_version: "v3".to_string(),
            selected_whisper_model_path: Some(PathBuf::from("/tmp/whisper.bin")),
            models_directory: Some(PathBuf::from("/tmp/ultravox-models")),
            whisper_language: Language("es".to_string()),
            translate_to_english: true,
            suppress_blank_audio: false,
            show_timestamps: true,
            temperature: 0.25,
            no_speech_threshold: 0.73,
            initial_prompt: "domain vocabulary".to_string(),
            use_beam_search: true,
            beam_size: 9,
            debug_mode: true,
            play_sound_on_record_start: false,
            use_asian_autocorrect: true,
            modifier_only_hotkey: "rightCommand".to_string(),
            key_combination: "Control+J".to_string(),
            hold_to_record: true,
            meeting_key_combination: "Option+M".to_string(),
            meeting_detection_enabled: false,
            add_space_after_sentence: false,
            auto_copy_to_clipboard: false,
            auto_paste_transcription: false,
            onboarding_completed: true,
            model_language: "multilingual".to_string(),
            theme: "winamp".to_string(),
            media_panel_enabled: false,
        };
        let serialized = toml::to_string(&cfg).unwrap();
        let parsed: AppConfig = toml::from_str(&serialized).unwrap();
        assert_eq!(parsed, cfg);
    }

    #[test]
    fn old_settings_without_meeting_detection_use_safe_default() {
        let parsed: AppConfig = toml::from_str(
            r#"
selected_engine = "whisper"
fluid_audio_model_version = "v2"
selected_whisper_model_path = "/tmp/whisper.bin"
whisper_language = "en"
translate_to_english = false
suppress_blank_audio = true
show_timestamps = false
temperature = 0.0
no_speech_threshold = 0.6
initial_prompt = ""
use_beam_search = false
beam_size = 5
debug_mode = false
play_sound_on_record_start = true
use_asian_autocorrect = false
modifier_only_hotkey = "none"
key_combination = "Option+Backtick"
hold_to_record = false
meeting_key_combination = "Control+M"
add_space_after_sentence = true
auto_copy_to_clipboard = true
auto_paste_transcription = true
onboarding_completed = false
model_language = "english"
"#,
        )
        .unwrap();
        assert!(parsed.meeting_detection_enabled);
        assert!(parsed.media_panel_enabled);
    }
}
