use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::io::Write;
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
        if cfg!(target_os = "macos") {
            Engine::FluidAudio
        } else {
            Engine::Whisper
        }
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

const CURRENT_CONFIG_VERSION: u32 = 5;

/// Top-level application settings persisted across launches.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AppConfig {
    /// Persisted schema version for one-time settings migrations.
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
    /// User-authored custom terms. Available in every edition.
    #[serde(default)]
    pub custom_dictionary: String,
    /// Generated canonical terms from explicitly selected Retex vaults.
    #[serde(default)]
    pub retex_dictionary: String,
    /// Vault directories for which the user granted read-only vocabulary access.
    #[serde(default)]
    pub retex_vault_paths: Vec<PathBuf>,
    /// Stable filesystem identities paired with the authorized vault paths.
    #[serde(default)]
    pub retex_vault_identities: Vec<String>,
    /// Opt in to a non-blocking weekly refresh at application startup.
    #[serde(default)]
    pub retex_auto_refresh: bool,
    /// Time of the last successful scan of every selected vault.
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
    /// Show the Meeting mode secondary action (Pro feature surface).
    #[serde(default = "default_show_meeting_mode")]
    pub show_meeting_mode: bool,
    /// Show the system-audio-only Lecture mode secondary action.
    #[serde(default = "default_show_lecture_mode")]
    pub show_lecture_mode: bool,
    /// Show the Transcribe URL secondary action (free feature surface).
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
        let existed = config_path.exists();
        let mut config = if existed {
            let contents = std::fs::read_to_string(&config_path)?;
            toml::from_str(&contents)?
        } else {
            AppConfig::default()
        };
        let previous_version = config.config_version;
        let migrated = existed && previous_version < CURRENT_CONFIG_VERSION;
        if existed && previous_version < 2 {
            // Media and capture-backed reactive visuals became opt-in in v2.
            config.media_panel_enabled = false;
            config.reactive_visuals_enabled = false;
        }
        if existed && previous_version < 5 {
            // v5 moves Retex vault grants behind a native backend chooser. Old
            // renderer-authored paths are deliberately not grandfathered in.
            config.retex_vault_paths.clear();
            config.retex_vault_identities.clear();
            config.retex_dictionary.clear();
            config.retex_auto_refresh = false;
            config.retex_last_refresh_at = None;
        }
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

    /// Read the current configuration.
    pub fn get(&self) -> &AppConfig {
        &self.config
    }

    /// Replace the current configuration and persist it.
    pub fn set(&mut self, config: AppConfig) -> Result<(), ConfigError> {
        let previous = std::mem::replace(&mut self.config, config);
        if let Err(error) = self.save() {
            self.config = previous;
            return Err(error);
        }
        Ok(())
    }

    /// Persist the current configuration to disk.
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
        assert_eq!(cfg.config_version, CURRENT_CONFIG_VERSION);
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
        assert!(!cfg.media_panel_enabled);
        assert!(!cfg.reactive_visuals_enabled);
    }

    #[test]
    fn roundtrip_toml_preserves_every_setting() {
        let cfg = AppConfig {
            config_version: CURRENT_CONFIG_VERSION,
            selected_engine: Engine::Whisper,
            fluid_audio_model_version: "v3".to_string(),
            selected_whisper_model_path: Some(PathBuf::from("/tmp/whisper.bin")),
            models_directory: Some(PathBuf::from("/tmp/ultravox-models")),
            audio_input_device_id: Some("cpal:Studio Microphone".to_string()),
            whisper_language: Language("es".to_string()),
            translate_to_english: true,
            suppress_blank_audio: false,
            show_timestamps: true,
            temperature: 0.25,
            no_speech_threshold: 0.73,
            initial_prompt: "domain vocabulary".to_string(),
            custom_dictionary: "UltraVox = Ultra Box".to_string(),
            retex_dictionary: "Retex".to_string(),
            retex_vault_paths: vec![PathBuf::from("/tmp/vault")],
            retex_vault_identities: vec!["1:2".to_string()],
            retex_auto_refresh: true,
            retex_last_refresh_at: Some(Utc::now()),
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
            reactive_visuals_enabled: false,
            show_meeting_mode: false,
            show_lecture_mode: false,
            show_transcribe_url: true,
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
        assert_eq!(parsed.config_version, 0);
        assert!(!parsed.media_panel_enabled);
        assert!(!parsed.reactive_visuals_enabled);
    }

    #[test]
    fn pre_opt_in_settings_are_migrated_and_persisted_disabled() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory = std::env::temp_dir().join(format!(
            "ultravox-config-media-opt-in-{}-{unique}",
            std::process::id()
        ));
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(
            directory.join(ConfigManager::FILE_NAME),
            "config_version = 1\nmedia_panel_enabled = true\nreactive_visuals_enabled = true\n",
        )
        .unwrap();

        let manager = ConfigManager::new(&directory).unwrap();
        assert!(!manager.get().media_panel_enabled);
        assert!(!manager.get().reactive_visuals_enabled);
        assert_eq!(manager.get().config_version, CURRENT_CONFIG_VERSION);
        let persisted = std::fs::read_to_string(directory.join(ConfigManager::FILE_NAME)).unwrap();
        assert!(persisted.contains("config_version = 5"));
        assert!(persisted.contains("media_panel_enabled = false"));
        assert!(persisted.contains("reactive_visuals_enabled = false"));
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn old_settings_add_dictionary_permissions_without_enabling_scans() {
        let directory = std::env::temp_dir().join(format!(
            "ultravox-config-dictionary-{}-{}",
            std::process::id(),
            Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(
            directory.join(ConfigManager::FILE_NAME),
            "config_version = 3\ninitial_prompt = \"existing prompt\"\n",
        )
        .unwrap();

        let manager = ConfigManager::new(&directory).unwrap();
        assert_eq!(manager.get().config_version, CURRENT_CONFIG_VERSION);
        assert_eq!(manager.get().initial_prompt, "existing prompt");
        assert!(manager.get().custom_dictionary.is_empty());
        assert!(manager.get().retex_dictionary.is_empty());
        assert!(manager.get().retex_vault_paths.is_empty());
        assert!(!manager.get().retex_auto_refresh);
        assert!(manager.get().retex_last_refresh_at.is_none());
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn version_two_settings_keep_pro_preferences_during_shared_identity_upgrade() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory = std::env::temp_dir().join(format!(
            "ultravox-config-shared-identity-{}-{unique}",
            std::process::id()
        ));
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(
            directory.join(ConfigManager::FILE_NAME),
            "config_version = 2\nmeeting_key_combination = \"Option+M\"\nmeeting_detection_enabled = false\nmedia_panel_enabled = true\nreactive_visuals_enabled = true\nshow_meeting_mode = false\n",
        )
        .unwrap();

        let manager = ConfigManager::new(&directory).unwrap();
        assert_eq!(manager.get().config_version, CURRENT_CONFIG_VERSION);
        assert_eq!(manager.get().meeting_key_combination, "Option+M");
        assert!(!manager.get().meeting_detection_enabled);
        assert!(manager.get().media_panel_enabled);
        assert!(manager.get().reactive_visuals_enabled);
        assert!(!manager.get().show_meeting_mode);
        assert!(manager.get().show_lecture_mode);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn renderer_authored_retex_grants_are_revoked_by_v5_migration() {
        let directory = std::env::temp_dir().join(format!(
            "ultravox-config-retex-consent-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(
            directory.join(ConfigManager::FILE_NAME),
            "config_version = 4\nretex_vault_paths = [\"/tmp/untrusted\"]\nretex_dictionary = \"Private Name\"\nretex_auto_refresh = true\n",
        )
        .unwrap();
        let manager = ConfigManager::new(&directory).unwrap();
        assert_eq!(manager.get().config_version, CURRENT_CONFIG_VERSION);
        assert!(manager.get().retex_vault_paths.is_empty());
        assert!(manager.get().retex_vault_identities.is_empty());
        assert!(manager.get().retex_dictionary.is_empty());
        assert!(!manager.get().retex_auto_refresh);
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
