use std::path::PathBuf;
use std::process::ExitCode;

use ultravox_core::{
    AudioBackend, ConfigManager, CpalAudioBackend, CustomDictionary, DownloadManager, ModelCatalog,
    RecordingHistory,
};

const APP_IDENTIFIER: &str = "com.imploselabs.ultravox";
const USAGE: &str = "Usage: ultravox-control [health|status|model-catalog|audio-devices|dictionary-smoke]\n       ultravox-control dictionary-apply [--dictionary <entries>] <text>";

fn data_dir() -> Result<PathBuf, String> {
    if let Some(path) = std::env::var_os("ULTRAVOX_DATA_DIR").filter(|value| !value.is_empty()) {
        return Ok(PathBuf::from(path));
    }

    #[cfg(target_os = "macos")]
    {
        return std::env::var_os("HOME")
            .map(PathBuf::from)
            .map(|home| {
                home.join("Library")
                    .join("Application Support")
                    .join(APP_IDENTIFIER)
            })
            .ok_or_else(|| "cannot resolve the UltraVox application data directory".to_string());
    }
    #[cfg(target_os = "windows")]
    {
        return std::env::var_os("APPDATA")
            .map(PathBuf::from)
            .map(|data| data.join(APP_IDENTIFIER))
            .ok_or_else(|| "cannot resolve the UltraVox application data directory".to_string());
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        if let Some(data) = std::env::var_os("XDG_DATA_HOME").filter(|value| !value.is_empty()) {
            return Ok(PathBuf::from(data).join(APP_IDENTIFIER));
        }
        return std::env::var_os("HOME")
            .map(PathBuf::from)
            .map(|home| home.join(".local/share").join(APP_IDENTIFIER))
            .ok_or_else(|| "cannot resolve the UltraVox application data directory".to_string());
    }
    #[allow(unreachable_code)]
    Err("cannot resolve the UltraVox application data directory".to_string())
}

fn run_health(dir: &PathBuf) -> Result<(), String> {
    let config = ConfigManager::new(dir).map_err(|e| e.to_string())?;
    let _ = config.get();

    let history = RecordingHistory::new(dir.clone()).map_err(|e| e.to_string())?;
    let _ = history.list(1, 0).map_err(|e| e.to_string())?;

    let catalog = ModelCatalog::default();
    if catalog.default_model().is_none() {
        return Err("no default model in catalog".to_string());
    }

    let downloads = DownloadManager::new();
    let _ = downloads.list();

    println!("health: ok");
    println!("  config path: {}", dir.join("settings.toml").display());
    println!(
        "  history path: {}",
        dir.join("recordings.sqlite").display()
    );
    println!("  default model: {}", catalog.default_model().unwrap().id);
    Ok(())
}

fn run_status() -> Result<(), String> {
    println!("status: ok");
    println!("  recording: false");
    println!("  transcription: idle");
    println!("  version: {}", env!("CARGO_PKG_VERSION"));
    Ok(())
}

fn run_model_catalog() -> Result<(), String> {
    let catalog = ModelCatalog::default();
    println!("models: {}", catalog.models.len());
    for model in &catalog.models {
        let default_marker = if model.is_default { " (default)" } else { "" };
        println!(
            "  {} - {} [{}]{}",
            model.id, model.name, model.filename, default_marker
        );
    }
    Ok(())
}

fn configured_dictionary(dir: &PathBuf) -> Result<CustomDictionary, String> {
    let config = ConfigManager::new(dir).map_err(|error| error.to_string())?;
    CustomDictionary::parse(&config.get().custom_dictionary).map_err(|error| error.to_string())
}

fn run_dictionary_smoke(dir: &PathBuf) -> Result<(), String> {
    let dictionary = configured_dictionary(dir)?;
    println!("dictionary-smoke: ok");
    println!("  settings path: {}", dir.join("settings.toml").display());
    println!("  entries: {}", dictionary.canonical_terms().len());
    Ok(())
}

fn dictionary_apply_output(args: &[String], dir: &PathBuf) -> Result<String, String> {
    let (dictionary, text_start) = if args.first().map(String::as_str) == Some("--dictionary") {
        let source = args
            .get(1)
            .ok_or_else(|| "--dictionary requires dictionary entries".to_string())?;
        (
            CustomDictionary::parse(source).map_err(|error| error.to_string())?,
            2,
        )
    } else {
        (configured_dictionary(dir)?, 0)
    };
    let text = args[text_start..].join(" ");
    if text.is_empty() {
        return Err(format!("dictionary-apply requires text\n{USAGE}"));
    }
    Ok(dictionary.apply(&text))
}

fn run_dictionary_apply(args: &[String], dir: &PathBuf) -> Result<(), String> {
    println!("{}", dictionary_apply_output(args, dir)?);
    Ok(())
}

async fn run_audio_devices() -> Result<(), String> {
    let backend = CpalAudioBackend::new();
    let devices = backend
        .list_devices()
        .await
        .map_err(|e| format!("audio device list failed: {e}"))?;
    if devices.is_empty() {
        return Err("no audio input devices found".to_string());
    }
    println!("audio-devices: ok");
    println!("  devices: {}", devices.len());
    for device in &devices {
        let default_marker = if device.is_default { " (default)" } else { "" };
        println!("  {} - {}{}", device.id, device.name, default_marker);
    }
    Ok(())
}

#[tokio::main]
async fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    let command = args.get(1).map(|s| s.as_str()).unwrap_or("health");

    let result = match command {
        "health" => data_dir().and_then(|dir| run_health(&dir)),
        "status" => run_status(),
        "model-catalog" => run_model_catalog(),
        "audio-devices" => run_audio_devices().await,
        "dictionary-smoke" => data_dir().and_then(|dir| run_dictionary_smoke(&dir)),
        "dictionary-apply" => data_dir().and_then(|dir| run_dictionary_apply(&args[2..], &dir)),
        "help" | "--help" | "-h" => {
            println!("{}", USAGE);
            Ok(())
        }
        _ => Err(format!("unknown command: {command}\n{USAGE}")),
    };

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;
    use std::sync::Mutex;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    struct EnvironmentGuard {
        previous: Option<OsString>,
    }

    impl EnvironmentGuard {
        fn set_data_dir(path: &std::path::Path) -> Self {
            let previous = std::env::var_os("ULTRAVOX_DATA_DIR");
            std::env::set_var("ULTRAVOX_DATA_DIR", path);
            Self { previous }
        }
    }

    impl Drop for EnvironmentGuard {
        fn drop(&mut self) {
            if let Some(previous) = self.previous.take() {
                std::env::set_var("ULTRAVOX_DATA_DIR", previous);
            } else {
                std::env::remove_var("ULTRAVOX_DATA_DIR");
            }
        }
    }

    #[test]
    fn configured_dictionary_commands_honor_data_dir_override() {
        let _lock = ENV_LOCK.lock().unwrap();
        let directory = tempfile::tempdir().unwrap();
        let _environment = EnvironmentGuard::set_data_dir(directory.path());
        let resolved = data_dir().unwrap();
        assert_eq!(resolved, directory.path());

        let mut config = ConfigManager::new(&resolved).unwrap();
        config.mutate().custom_dictionary = "Retex = retext".to_string();
        config.save().unwrap();

        let dictionary = configured_dictionary(&resolved).unwrap();
        assert_eq!(dictionary.canonical_terms(), vec!["Retex"]);
        assert_eq!(
            dictionary_apply_output(&["retext".to_string()], &resolved).unwrap(),
            "Retex"
        );
    }

    #[test]
    fn inline_dictionary_still_overrides_configured_settings() {
        let directory = tempfile::tempdir().unwrap();
        let args = vec![
            "--dictionary".to_string(),
            "UltraVox = Ultra Box".to_string(),
            "Ultra Box".to_string(),
        ];
        assert_eq!(
            dictionary_apply_output(&args, &directory.path().to_path_buf()).unwrap(),
            "UltraVox"
        );
    }
}
