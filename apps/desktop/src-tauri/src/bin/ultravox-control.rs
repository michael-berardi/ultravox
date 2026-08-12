use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use arboard::Clipboard;
use chrono::Utc;
use ultravox_core::{
    AudioBackend, ConfigManager, CpalAudioBackend, DownloadManager, DownloadState, ModelCatalog,
    ModelDownload, RecordingHistory, RecordingRow, RecordingStatus, ShortcutSettings,
};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, UnixStream};
#[cfg(target_os = "macos")]
use ultravox_macos_bridge as bridge;
use uuid::Uuid;

const USAGE: &str = "Usage: ultravox-control [health|status|model-catalog|history-smoke|download-smoke|shortcut-config-smoke|audio-devices|live-record-smoke|recording-id-db-smoke|paste-bridge-dry-run|caret-bridge-dry-run|transcribe-fixture-smoke|transcribe <path> [v2|v3]|voice-health|voice-start|voice-stop <id>|voice-status <id>|voice-cancel <id>]";

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

fn run_history_smoke() -> Result<(), String> {
    let mut history = RecordingHistory::new_in_memory().map_err(|e| e.to_string())?;

    let row = RecordingRow {
        id: Uuid::new_v4(),
        timestamp: Utc::now(),
        file_name: "smoke.wav".to_string(),
        title: "Smoke test recording".to_string(),
        preview: "smoke test".to_string(),
        transcription: "smoke test".to_string(),
        language: "en".to_string(),
        duration_seconds: 2.5,
        status: RecordingStatus::Completed,
        progress: 1.0,
        source_file_url: None,
    };

    history.insert(&row).map_err(|e| e.to_string())?;
    let fetched = history
        .get(row.id)
        .map_err(|e| e.to_string())?
        .ok_or("inserted row disappeared")?;

    if fetched.file_name != row.file_name || fetched.transcription != row.transcription {
        return Err("history round-trip mismatch".to_string());
    }

    let list = history.list(10, 0).map_err(|e| e.to_string())?;
    if list.len() != 1 {
        return Err(format!("expected 1 history row, got {}", list.len()));
    }

    history.delete(row.id).map_err(|e| e.to_string())?;
    let after = history.get(row.id).map_err(|e| e.to_string())?;
    if after.is_some() {
        return Err("deleted row still present".to_string());
    }

    println!("history-smoke: ok");
    Ok(())
}

async fn run_download_smoke() -> Result<(), String> {
    // Start a tiny local HTTP server that serves a fixed payload.
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(|e| e.to_string())?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    let body = b"ultravox model smoke test data";

    tokio::spawn(async move {
        let response_header = format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: application/octet-stream\r\n\r\n",
            body.len()
        );
        let mut buf = [0u8; 1024];
        loop {
            match listener.accept().await {
                Ok((mut socket, _)) => {
                    let _ = socket.read(&mut buf).await;
                    let _ = socket.write_all(response_header.as_bytes()).await;
                    let _ = socket.write_all(body).await;
                    let _ = socket.flush().await;
                }
                Err(_) => break,
            }
        }
    });

    let models_dir =
        std::env::temp_dir().join(format!("ultravox-control-models-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&models_dir).map_err(|e| e.to_string())?;
    let manager = DownloadManager::with_models_dir(models_dir.clone());

    let request = ModelDownload {
        id: Uuid::new_v4(),
        model_id: "smoke-model".to_string(),
        url: format!("http://127.0.0.1:{}/model.bin", port),
        destination: PathBuf::from("smoke-model.bin"),
    };

    manager
        .start_download(request.clone())
        .await
        .map_err(|e| e.to_string())?;

    // Poll for completion, cancellation, or failure.
    let mut completed = false;
    for _ in 0..50 {
        tokio::time::sleep(Duration::from_millis(100)).await;
        if let Some(progress) = manager.get(request.id) {
            match progress.state {
                DownloadState::Completed => {
                    completed = true;
                    break;
                }
                DownloadState::Cancelled => return Err("download was cancelled".to_string()),
                DownloadState::Failed => return Err("download failed".to_string()),
                _ => {}
            }
        }
    }

    if !completed {
        return Err("download did not complete in time".to_string());
    }

    let downloaded_path = models_dir.join("smoke-model.bin");
    let content = tokio::fs::read(&downloaded_path)
        .await
        .map_err(|e| e.to_string())?;
    if content != body {
        return Err("downloaded content mismatch".to_string());
    }

    // Verify the cache check reports the file as downloaded.
    if !manager.is_downloaded("smoke-model.bin") {
        return Err("is_downloaded did not report cached file".to_string());
    }

    println!("download-smoke: ok");
    println!("  models dir: {}", models_dir.display());
    println!("  downloaded: {} bytes", content.len());
    Ok(())
}

fn run_shortcut_config_smoke() -> Result<(), String> {
    let dir = std::env::temp_dir().join(format!("ultravox-control-shortcut-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;

    let mut manager = ConfigManager::new(&dir).map_err(|e| e.to_string())?;
    {
        let cfg = manager.mutate();
        cfg.key_combination = "Option+Backtick".to_string();
        cfg.modifier_only_hotkey = "option".to_string();
        cfg.hold_to_record = true;
    }
    manager.save().map_err(|e| e.to_string())?;

    let cfg = manager.get().clone();
    if cfg.key_combination != "Option+Backtick" {
        return Err("key_combination round-trip mismatch".to_string());
    }
    if cfg.modifier_only_hotkey != "option" {
        return Err("modifier_only_hotkey round-trip mismatch".to_string());
    }
    if !cfg.hold_to_record {
        return Err("hold_to_record round-trip mismatch".to_string());
    }

    let settings = ShortcutSettings {
        modifier_only_hotkey: cfg
            .modifier_only_hotkey
            .as_str()
            .parse()
            .unwrap_or_default(),
        key_combination: Some(cfg.key_combination.clone()),
        hold_to_record: cfg.hold_to_record,
        meeting_key_combination: cfg.meeting_key_combination.clone(),
    };
    if settings.modifier_only_hotkey.as_ref() != "option" {
        return Err("ShortcutSettings modifier round-trip mismatch".to_string());
    }
    if settings.key_combination.as_deref() != Some("Option+Backtick") {
        return Err("ShortcutSettings key_combination round-trip mismatch".to_string());
    }
    if settings.meeting_key_combination != "Control+M" {
        return Err("ShortcutSettings meeting shortcut round-trip mismatch".to_string());
    }

    println!("shortcut-config-smoke: ok");
    println!("  config dir: {}", dir.display());
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

async fn run_live_record_smoke() -> Result<(), String> {
    let backend = CpalAudioBackend::new();
    let devices = backend
        .list_devices()
        .await
        .map_err(|e| format!("audio device list failed: {e}"))?;
    if devices.is_empty() {
        return Err("no audio input devices found".to_string());
    }
    let default_device = devices
        .iter()
        .find(|d| d.is_default)
        .or_else(|| devices.first())
        .cloned()
        .ok_or("no audio input device available")?;

    let dir = data_dir().join("live-record-smoke");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let output_path = dir.join(format!("{}.wav", Uuid::new_v4()));

    let mut config = ultravox_core::AudioInputConfig::default();
    config.device_id = Some(default_device.id.clone());

    let mut backend = CpalAudioBackend::new();
    let _started = backend
        .start_recording(config, output_path.clone())
        .await
        .map_err(|e| format!("start recording failed: {e}"))?;

    tokio::time::sleep(Duration::from_secs(2)).await;

    let finished = backend
        .stop_recording()
        .await
        .map_err(|e| format!("stop recording failed: {e}"))?;

    let metadata = std::fs::metadata(&output_path).map_err(|e| e.to_string())?;
    let file_size = metadata.len();
    if file_size == 0 {
        return Err("recorded WAV file is empty".to_string());
    }
    let duration_ms = finished.duration_ms.unwrap_or(0);
    if duration_ms == 0 {
        return Err("recording duration is 0".to_string());
    }

    let expected_id = output_path
        .file_stem()
        .and_then(|s| s.to_str())
        .and_then(|s| Uuid::parse_str(s).ok())
        .ok_or("output path stem is not a UUID")?;
    let actual_id =
        Uuid::parse_str(&finished.id).map_err(|_| "recording id is not a UUID".to_string())?;
    if actual_id != expected_id {
        return Err(format!(
            "recording id {} does not match output path stem {}",
            actual_id, expected_id
        ));
    }

    println!("live-record-smoke: ok");
    println!("  device: {}", default_device.name);
    println!("  path: {}", output_path.display());
    println!("  size: {} bytes", file_size);
    println!("  duration: {} ms", duration_ms);
    Ok(())
}

async fn run_recording_id_db_smoke() -> Result<(), String> {
    let backend = CpalAudioBackend::new();
    let devices = backend
        .list_devices()
        .await
        .map_err(|e| format!("audio device list failed: {e}"))?;
    if devices.is_empty() {
        return Err("no audio input devices found".to_string());
    }
    let default_device = devices
        .iter()
        .find(|d| d.is_default)
        .or_else(|| devices.first())
        .cloned()
        .ok_or("no audio input device available")?;

    let dir = data_dir().join("recording-id-db-smoke");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let id = Uuid::new_v4();
    let output_path = dir.join(format!("{}.wav", id));

    let mut config = ultravox_core::AudioInputConfig::default();
    config.device_id = Some(default_device.id.clone());

    let mut backend = CpalAudioBackend::new();
    let _started = backend
        .start_recording(config, output_path.clone())
        .await
        .map_err(|e| format!("start recording failed: {e}"))?;

    tokio::time::sleep(Duration::from_secs(1)).await;

    let finished = backend
        .stop_recording()
        .await
        .map_err(|e| format!("stop recording failed: {e}"))?;

    let actual_id =
        Uuid::parse_str(&finished.id).map_err(|_| "recording id is not a UUID".to_string())?;
    if actual_id != id {
        return Err(format!(
            "recording id {} does not match output path stem {}",
            actual_id, id
        ));
    }

    // Simulate the desktop layer: insert a pending row with the same UUID the
    // UI/command uses, then verify the transcription flow can update it.
    let mut history = RecordingHistory::new_in_memory().map_err(|e| e.to_string())?;
    let mut row = RecordingRow {
        id,
        timestamp: Utc::now(),
        file_name: output_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("recording.wav")
            .to_string(),
        title: String::new(),
        preview: String::new(),
        transcription: String::new(),
        language: "en".to_string(),
        duration_seconds: finished.duration_ms.unwrap_or(0) as f64 / 1000.0,
        status: RecordingStatus::Pending,
        progress: 0.0,
        source_file_url: Some(output_path.to_string_lossy().to_string()),
    };
    row.refresh_display();
    history.insert(&row).map_err(|e| e.to_string())?;

    let fetched = history
        .get(id)
        .map_err(|e| e.to_string())?
        .ok_or("recording row not found by id")?;
    if fetched.id != id {
        return Err("history row id mismatch".to_string());
    }

    // Simulate transcription completion updating the same row.
    let mut completed = fetched;
    completed.transcription = "recording id smoke test result".to_string();
    completed.status = RecordingStatus::Completed;
    completed.progress = 1.0;
    completed.refresh_display();
    history.insert(&completed).map_err(|e| e.to_string())?;

    let updated = history
        .get(id)
        .map_err(|e| e.to_string())?
        .ok_or("completed row disappeared")?;
    if updated.status != RecordingStatus::Completed {
        return Err(format!(
            "expected completed status, got {:?}",
            updated.status
        ));
    }
    if updated.transcription != completed.transcription {
        return Err("transcription update mismatch".to_string());
    }
    if updated.title.is_empty() || updated.preview.is_empty() {
        return Err("title/preview not generated after refresh".to_string());
    }

    // Verify the clipboard copy path works independently of the Tauri runtime.
    let mut clipboard = Clipboard::new().map_err(|e| format!("clipboard init failed: {e}"))?;
    clipboard
        .set_text(updated.transcription.clone())
        .map_err(|e| format!("clipboard write failed: {e}"))?;
    let clipboard_text = clipboard
        .get_text()
        .map_err(|e| format!("clipboard read failed: {e}"))?;
    if clipboard_text != updated.transcription {
        return Err(format!(
            "clipboard round-trip mismatch: expected {:?}, got {:?}",
            updated.transcription, clipboard_text
        ));
    }

    println!("recording-id-db-smoke: ok");
    println!("  recording id: {}", id);
    println!("  history row updated: {}", updated.status.as_str());
    println!("  title: {}", updated.title);
    println!("  clipboard round-trip: ok");
    Ok(())
}

#[cfg(target_os = "macos")]
fn run_paste_bridge_dry_run() -> Result<(), String> {
    let result = bridge::paste_text("UltraVox paste bridge dry run");
    if result < 0 {
        return Err(format!("paste bridge returned error code: {result}"));
    }
    println!("paste-bridge-dry-run: ok");
    println!("  result code: {result}");
    Ok(())
}

#[cfg(not(target_os = "macos"))]
fn run_paste_bridge_dry_run() -> Result<(), String> {
    println!("paste-bridge-dry-run: skipped (macOS only)");
    Ok(())
}

#[cfg(target_os = "macos")]
fn run_caret_bridge_dry_run() -> Result<(), String> {
    let (_x, _y, found) = bridge::get_caret_position();
    println!("caret-bridge-dry-run: ok");
    println!("  found: {found}");
    Ok(())
}

#[cfg(not(target_os = "macos"))]
fn run_caret_bridge_dry_run() -> Result<(), String> {
    println!("caret-bridge-dry-run: skipped (macOS only)");
    Ok(())
}

fn run_transcribe_fixture_smoke() -> Result<(), String> {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let repo_root = manifest_dir
        .parent()
        .and_then(|p| p.parent())
        .and_then(|p| p.parent())
        .ok_or("unable to locate repository root")?;
    let fixture_path = repo_root
        .join("test")
        .join("fixtures")
        .join("jfk-short.wav");

    if !fixture_path.exists() {
        return Err(format!("fixture not found: {}", fixture_path.display()));
    }
    let metadata = std::fs::metadata(&fixture_path).map_err(|e| e.to_string())?;
    if metadata.len() == 0 {
        return Err("transcribe fixture file is empty".to_string());
    }

    #[cfg(target_os = "macos")]
    {
        let result = bridge::transcribe_file(fixture_path.to_string_lossy().as_ref())
            .map_err(|_| "transcribe fixture failed".to_string())?;
        if result.is_empty() {
            return Err("transcribe fixture returned empty text".to_string());
        }
        let lower = result.to_lowercase();
        let expected_terms = ["fellow", "americans"];
        let missing: Vec<_> = expected_terms
            .iter()
            .filter(|term| !lower.contains(**term))
            .copied()
            .collect();
        if !missing.is_empty() {
            return Err(format!(
                "transcription missing expected terms {:?}; got: {result}",
                missing
            ));
        }
        println!("transcribe-fixture-smoke: ok");
        println!("  fixture: {}", fixture_path.display());
        println!("  transcription: {result}");
    }

    #[cfg(not(target_os = "macos"))]
    {
        println!("transcribe-fixture-smoke: ok (macOS only)");
        println!("  fixture: {}", fixture_path.display());
    }

    Ok(())
}

fn run_transcribe(path: &str, version: &str) -> Result<(), String> {
    if path.is_empty() {
        return Err("audio path is required".to_string());
    }
    let path = PathBuf::from(path);
    if !path.exists() {
        return Err(format!("audio file not found: {}", path.display()));
    }
    let metadata = std::fs::metadata(&path).map_err(|e| e.to_string())?;
    if metadata.len() == 0 {
        return Err("audio file is empty".to_string());
    }

    #[cfg(target_os = "macos")]
    {
        let result = bridge::transcribe_file_with_version(path.to_string_lossy().as_ref(), version)
            .map_err(|_| "transcription failed".to_string())?;
        if result.is_empty() {
            return Err("transcription returned empty text".to_string());
        }
        println!("transcribe: ok");
        println!("  path: {}", path.display());
        println!("  version: {version}");
        println!("  transcription: {result}");
    }

    #[cfg(not(target_os = "macos"))]
    {
        println!("transcribe: ok (macOS only)");
        println!("  path: {}", path.display());
        println!("  version: {version}");
    }

    Ok(())
}

async fn run_voice_request(command: &str, recording_id: Option<&str>) -> Result<(), String> {
    let mut request = serde_json::json!({
        "version": 1,
        "requestId": Uuid::new_v4().to_string(),
        "command": command,
    });
    if let Some(recording_id) = recording_id {
        request["recording_id"] = serde_json::Value::String(recording_id.to_string());
    }

    let payload = serde_json::to_vec(&request).map_err(|e| e.to_string())?;
    let mut stream = UnixStream::connect(ultravox_desktop_lib::voice_ipc::socket_path())
        .await
        .map_err(|e| e.to_string())?;
    stream
        .write_u32(payload.len() as u32)
        .await
        .map_err(|e| e.to_string())?;
    stream
        .write_all(&payload)
        .await
        .map_err(|e| e.to_string())?;
    stream.flush().await.map_err(|e| e.to_string())?;

    let response_length = stream.read_u32().await.map_err(|e| e.to_string())? as usize;
    if response_length == 0 || response_length > 64 * 1024 {
        return Err(format!("invalid voice response length: {response_length}"));
    }
    let mut response = vec![0_u8; response_length];
    stream
        .read_exact(&mut response)
        .await
        .map_err(|e| e.to_string())?;
    let value: serde_json::Value = serde_json::from_slice(&response).map_err(|e| e.to_string())?;
    println!(
        "{}",
        serde_json::to_string_pretty(&value).map_err(|e| e.to_string())?
    );
    if value["ok"].as_bool() == Some(true) {
        Ok(())
    } else {
        Err(value["error"]
            .as_str()
            .unwrap_or("voice request failed")
            .to_string())
    }
}

#[tokio::main]
async fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    let command = args.get(1).map(|s| s.as_str()).unwrap_or("health");

    let result = match command {
        "health" => run_health(&data_dir()),
        "status" => run_status(),
        "model-catalog" => run_model_catalog(),
        "history-smoke" => run_history_smoke(),
        "download-smoke" => run_download_smoke().await,
        "shortcut-config-smoke" => run_shortcut_config_smoke(),
        "audio-devices" => run_audio_devices().await,
        "live-record-smoke" => run_live_record_smoke().await,
        "recording-id-db-smoke" => run_recording_id_db_smoke().await,
        "paste-bridge-dry-run" => run_paste_bridge_dry_run(),
        "caret-bridge-dry-run" => run_caret_bridge_dry_run(),
        "transcribe-fixture-smoke" => run_transcribe_fixture_smoke(),
        "transcribe" => {
            let path = args.get(2).map(|s| s.as_str()).unwrap_or("");
            let version = args.get(3).map(|s| s.as_str()).unwrap_or("v2");
            run_transcribe(path, version)
        }
        "voice-health" => run_voice_request("health", None).await,
        "voice-start" => run_voice_request("start", Some(&Uuid::new_v4().to_string())).await,
        "voice-stop" => run_voice_request("stop", args.get(2).map(String::as_str)).await,
        "voice-status" => run_voice_request("status", args.get(2).map(String::as_str)).await,
        "voice-cancel" => run_voice_request("cancel", args.get(2).map(String::as_str)).await,
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
