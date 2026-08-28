use std::path::PathBuf;
use std::process::ExitCode;

use ultravox_core::{
    AudioBackend, ConfigManager, CpalAudioBackend, DownloadManager, ModelCatalog, RecordingHistory,
};

const USAGE: &str = "Usage: ultravox-control [health|status|model-catalog|audio-devices]";

fn data_dir() -> PathBuf {
    std::env::var("ULTRAVOX_DATA_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir().join("ultravox-control"))
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
        "health" => run_health(&data_dir()),
        "status" => run_status(),
        "model-catalog" => run_model_catalog(),
        "audio-devices" => run_audio_devices().await,
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
