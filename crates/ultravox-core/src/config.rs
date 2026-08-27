use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use thiserror::Error;

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Engine { Whisper, FluidAudio }
impl Default for Engine { fn default() -> Self { Self::FluidAudio } }

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Language(pub String);
impl Default for Language { fn default() -> Self { Self("en".to_string()) } }

const CURRENT_CONFIG_VERSION: u32 = 3;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AppConfig {
    #[serde(default)] pub config_version: u32,
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
    pub add_space_after_sentence: bool,
    pub auto_copy_to_clipboard: bool,
    pub auto_paste_transcription: bool,
    #[serde(default)] pub onboarding_completed: bool,
    #[serde(default = "default_model_language")] pub model_language: String,
    #[serde(default = "default_theme")] pub theme: String,
    #[serde(default = "default_reactive_visuals_enabled")] pub reactive_visuals_enabled: bool,
    #[serde(default = "default_show_transcribe_url")] pub show_transcribe_url: bool,
}
fn default_theme() -> String { "midnight".to_string() }
fn default_model_language() -> String { "english".to_string() }
fn default_reactive_visuals_enabled() -> bool { false }
fn default_show_transcribe_url() -> bool { false }

impl Default for AppConfig {
    fn default() -> Self { Self {
        config_version: CURRENT_CONFIG_VERSION,
        selected_engine: Engine::default(), fluid_audio_model_version: "v2".to_string(),
        selected_whisper_model_path: None, models_directory: None, whisper_language: Language::default(),
        translate_to_english: false, suppress_blank_audio: true, show_timestamps: false,
        temperature: 0.0, no_speech_threshold: 0.6, initial_prompt: String::new(), use_beam_search: false,
        beam_size: 5, debug_mode: false, play_sound_on_record_start: true, use_asian_autocorrect: false,
        modifier_only_hotkey: "none".to_string(), key_combination: "Option+Backtick".to_string(), hold_to_record: false,
        add_space_after_sentence: true, auto_copy_to_clipboard: true, auto_paste_transcription: true,
        onboarding_completed: false, model_language: default_model_language(), theme: default_theme(),
        reactive_visuals_enabled: default_reactive_visuals_enabled(), show_transcribe_url: default_show_transcribe_url(),
    } }
}

#[derive(Debug)]
pub struct ConfigManager { config_path: PathBuf, config: AppConfig }
impl ConfigManager {
    const FILE_NAME: &'static str = "settings.toml";
    pub fn new(app_dir: impl AsRef<Path>) -> Result<Self, ConfigError> {
        let app_dir = app_dir.as_ref(); std::fs::create_dir_all(app_dir)?;
        let config_path = app_dir.join(Self::FILE_NAME); let existed = config_path.exists();
        let mut config = if existed { toml::from_str(&std::fs::read_to_string(&config_path)?)? } else { AppConfig::default() };
        let migrated = existed && config.config_version < CURRENT_CONFIG_VERSION;
        if migrated { config.config_version = CURRENT_CONFIG_VERSION; }
        let manager = Self { config_path, config }; if migrated { manager.save()?; } Ok(manager)
    }
    pub fn get(&self) -> &AppConfig { &self.config }
    pub fn set(&mut self, config: AppConfig) -> Result<(), ConfigError> { self.config = config; self.save() }
    pub fn save(&self) -> Result<(), ConfigError> { std::fs::write(&self.config_path, toml::to_string_pretty(&self.config)?)?; Ok(()) }
    pub fn mutate(&mut self) -> &mut AppConfig { &mut self.config }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn defaults_are_light() { let cfg=AppConfig::default(); assert_eq!(cfg.config_version,3); assert_eq!(cfg.key_combination,"Option+Backtick"); assert_eq!(cfg.theme,"midnight"); }
    #[test] fn roundtrip_preserves_settings() { let cfg=AppConfig::default(); let parsed:AppConfig=toml::from_str(&toml::to_string(&cfg).unwrap()).unwrap(); assert_eq!(parsed,cfg); }
}
