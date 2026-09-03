use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::io::Write;
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
pub enum Engine {
    Whisper,
    FluidAudio,
}
impl Default for Engine {
    fn default() -> Self {
        if cfg!(target_os = "macos") {
            Self::FluidAudio
        } else {
            Self::Whisper
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Language(pub String);
impl Default for Language {
    fn default() -> Self {
        Self("en".to_string())
    }
}

const CURRENT_CONFIG_VERSION: u32 = 5;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AppConfig {
    #[serde(default)]
    pub config_version: u32,
    pub selected_engine: Engine,
    pub fluid_audio_model_version: String,
    pub selected_whisper_model_path: Option<PathBuf>,
    pub models_directory: Option<PathBuf>,
    pub audio_input_device_id: Option<String>,
    pub whisper_language: Language,
    pub translate_to_english: bool,
    pub suppress_blank_audio: bool,
    pub show_timestamps: bool,
    pub temperature: f64,
    pub no_speech_threshold: f64,
    pub initial_prompt: String,
    #[serde(default)]
    pub custom_dictionary: String,
    // Pro-owned fields remain part of the shared v4 schema so opening and
    // saving settings in Light never discards them. Light does not scan or
    // refresh these paths.
    #[serde(default)]
    pub retex_dictionary: String,
    #[serde(default)]
    pub retex_vault_paths: Vec<PathBuf>,
    #[serde(default)]
    pub retex_vault_identities: Vec<String>,
    #[serde(default)]
    pub retex_auto_refresh: bool,
    #[serde(default)]
    pub retex_last_refresh_at: Option<DateTime<Utc>>,
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
    #[serde(default = "default_reactive_visuals_enabled")]
    pub reactive_visuals_enabled: bool,
    #[serde(default = "default_show_meeting_mode")]
    pub show_meeting_mode: bool,
    #[serde(default = "default_show_lecture_mode")]
    pub show_lecture_mode: bool,
    #[serde(default = "default_show_transcribe_url")]
    pub show_transcribe_url: bool,
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
    false
}
fn default_reactive_visuals_enabled() -> bool {
    false
}
fn default_show_meeting_mode() -> bool {
    true
}
fn default_show_lecture_mode() -> bool {
    true
}
fn default_show_transcribe_url() -> bool {
    false
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            config_version: CURRENT_CONFIG_VERSION,
            selected_engine: Engine::default(),
            fluid_audio_model_version: "v2".to_string(),
            selected_whisper_model_path: None,
            models_directory: None,
            audio_input_device_id: None,
            whisper_language: Language::default(),
            translate_to_english: false,
            suppress_blank_audio: true,
            show_timestamps: false,
            temperature: 0.0,
            no_speech_threshold: 0.6,
            initial_prompt: String::new(),
            custom_dictionary: String::new(),
            retex_dictionary: String::new(),
            retex_vault_paths: Vec::new(),
            retex_vault_identities: Vec::new(),
            retex_auto_refresh: false,
            retex_last_refresh_at: None,
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
            reactive_visuals_enabled: default_reactive_visuals_enabled(),
            show_meeting_mode: default_show_meeting_mode(),
            show_lecture_mode: default_show_lecture_mode(),
            show_transcribe_url: default_show_transcribe_url(),
        }
    }
}

#[derive(Debug)]
pub struct ConfigManager {
    config_path: PathBuf,
    config: AppConfig,
}
impl ConfigManager {
    const FILE_NAME: &'static str = "settings.toml";
    pub fn new(app_dir: impl AsRef<Path>) -> Result<Self, ConfigError> {
        let app_dir = app_dir.as_ref();
        std::fs::create_dir_all(app_dir)?;
        let config_path = app_dir.join(Self::FILE_NAME);
        let existed = config_path.exists();
        let mut config = if existed {
            toml::from_str(&std::fs::read_to_string(&config_path)?)?
        } else {
            AppConfig::default()
        };
        let migrated = existed && config.config_version < CURRENT_CONFIG_VERSION;
        if migrated {
            config.config_version = CURRENT_CONFIG_VERSION;
        }
        let manager = Self {
            config_path,
            config,
        };
        if migrated {
            manager.save()?;
        }
        Ok(manager)
    }
    pub fn get(&self) -> &AppConfig {
        &self.config
    }
    pub fn set(&mut self, config: AppConfig) -> Result<(), ConfigError> {
        self.config = config;
        self.save()
    }
    pub fn save(&self) -> Result<(), ConfigError> {
        let contents = toml::to_string_pretty(&self.config)?;
        let temporary_path = self
            .config_path
            .with_extension(format!("toml.tmp-{}", uuid::Uuid::new_v4()));
        let write_result = (|| -> Result<(), std::io::Error> {
            let mut options = std::fs::OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            let mut file = options.open(&temporary_path)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
            }
            file.write_all(contents.as_bytes())?;
            file.sync_all()
        })();
        if let Err(error) = write_result {
            let _ = std::fs::remove_file(&temporary_path);
            return Err(ConfigError::Io(error));
        }
        if let Err(error) = std::fs::rename(&temporary_path, &self.config_path) {
            let _ = std::fs::remove_file(&temporary_path);
            return Err(ConfigError::Io(error));
        }
        Ok(())
    }
    pub fn mutate(&mut self) -> &mut AppConfig {
        &mut self.config
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn defaults_are_light() {
        let cfg = AppConfig::default();
        assert_eq!(cfg.config_version, CURRENT_CONFIG_VERSION);
        assert_eq!(cfg.key_combination, "Option+Backtick");
        assert_eq!(cfg.theme, "midnight");
        assert!(cfg.custom_dictionary.is_empty());
        assert!(cfg.retex_dictionary.is_empty());
        assert!(cfg.retex_vault_paths.is_empty());
        assert!(!cfg.retex_auto_refresh);
        assert!(cfg.retex_last_refresh_at.is_none());
    }
    #[test]
    fn roundtrip_preserves_shared_pro_settings() {
        let mut cfg = AppConfig::default();
        cfg.audio_input_device_id = Some("studio-mic".to_string());
        cfg.meeting_key_combination = "Option+M".to_string();
        cfg.meeting_detection_enabled = false;
        cfg.media_panel_enabled = true;
        cfg.reactive_visuals_enabled = true;
        cfg.show_meeting_mode = false;
        cfg.show_lecture_mode = false;
        cfg.custom_dictionary = "Retex = retext".to_string();
        cfg.retex_dictionary = "Cloudflare\nOpenAI".to_string();
        cfg.retex_vault_paths = vec![PathBuf::from("/tmp/pro-vault")];
        cfg.retex_vault_identities = vec!["1:2".to_string()];
        cfg.retex_auto_refresh = true;
        cfg.retex_last_refresh_at = Some(
            DateTime::parse_from_rfc3339("2026-09-03T20:00:00Z")
                .unwrap()
                .with_timezone(&Utc),
        );
        let parsed: AppConfig = toml::from_str(&toml::to_string(&cfg).unwrap()).unwrap();
        assert_eq!(parsed, cfg);
    }

    #[test]
    fn migrates_old_config_with_empty_dictionary() {
        let directory = std::env::temp_dir().join(format!(
            "ultravox-config-dictionary-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(
            directory.join(ConfigManager::FILE_NAME),
            "config_version = 3\nretex_dictionary = \"Cloudflare\\nOpenAI\"\nretex_vault_paths = [\"/tmp/pro-vault\"]\nretex_auto_refresh = true\nretex_last_refresh_at = \"2026-09-03T20:00:00Z\"\n",
        )
        .unwrap();

        let manager = ConfigManager::new(&directory).unwrap();
        assert_eq!(manager.get().config_version, CURRENT_CONFIG_VERSION);
        assert!(manager.get().custom_dictionary.is_empty());
        assert_eq!(manager.get().retex_dictionary, "Cloudflare\nOpenAI");
        assert_eq!(
            manager.get().retex_vault_paths,
            vec![PathBuf::from("/tmp/pro-vault")]
        );
        assert!(manager.get().retex_auto_refresh);
        assert_eq!(
            manager.get().retex_last_refresh_at,
            Some(
                DateTime::parse_from_rfc3339("2026-09-03T20:00:00Z")
                    .unwrap()
                    .with_timezone(&Utc)
            )
        );
        let persisted = std::fs::read_to_string(directory.join(ConfigManager::FILE_NAME)).unwrap();
        let persisted: AppConfig = toml::from_str(&persisted).unwrap();
        assert_eq!(persisted, *manager.get());

        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn light_load_and_write_preserve_pro_v5_fields() {
        let directory = std::env::temp_dir().join(format!(
            "ultravox-config-pro-roundtrip-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(
            directory.join(ConfigManager::FILE_NAME),
            "config_version = 5\nretex_dictionary = \"Retex\"\nretex_vault_paths = [\"/tmp/one\", \"/tmp/two\"]\nretex_vault_identities = [\"1:2\", \"1:3\"]\nretex_auto_refresh = true\nretex_last_refresh_at = \"2026-09-03T20:00:00Z\"\n",
        )
        .unwrap();

        let mut manager = ConfigManager::new(&directory).unwrap();
        let pro_fields = (
            manager.get().retex_dictionary.clone(),
            manager.get().retex_vault_paths.clone(),
            manager.get().retex_vault_identities.clone(),
            manager.get().retex_auto_refresh,
            manager.get().retex_last_refresh_at,
        );
        manager.mutate().custom_dictionary = "UltraVox = Ultra Box".to_string();
        manager.save().unwrap();

        let reloaded = ConfigManager::new(&directory).unwrap();
        assert_eq!(reloaded.get().retex_dictionary, pro_fields.0);
        assert_eq!(reloaded.get().retex_vault_paths, pro_fields.1);
        assert_eq!(reloaded.get().retex_vault_identities, pro_fields.2);
        assert_eq!(reloaded.get().retex_auto_refresh, pro_fields.3);
        assert_eq!(reloaded.get().retex_last_refresh_at, pro_fields.4);
        assert_eq!(reloaded.get().custom_dictionary, "UltraVox = Ultra Box");

        std::fs::remove_dir_all(directory).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn settings_writes_remain_owner_only() {
        use std::os::unix::fs::PermissionsExt;

        let directory = std::env::temp_dir().join(format!(
            "ultravox-config-permissions-{}",
            uuid::Uuid::new_v4()
        ));
        let manager = ConfigManager::new(&directory).unwrap();
        manager.save().unwrap();
        let settings = directory.join(ConfigManager::FILE_NAME);
        assert_eq!(
            std::fs::metadata(&settings).unwrap().permissions().mode() & 0o777,
            0o600
        );

        std::fs::set_permissions(&settings, std::fs::Permissions::from_mode(0o644)).unwrap();
        manager.save().unwrap();
        assert_eq!(
            std::fs::metadata(&settings).unwrap().permissions().mode() & 0o777,
            0o600
        );

        std::fs::remove_dir_all(directory).unwrap();
    }
}
