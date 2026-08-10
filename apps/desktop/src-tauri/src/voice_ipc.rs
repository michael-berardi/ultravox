use std::collections::HashSet;
use std::io;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{UnixListener, UnixStream};
use uuid::Uuid;

use crate::state::AppState;

const PROTOCOL_VERSION: u8 = 1;
const MAX_FRAME_BYTES: usize = 64 * 1024;
const SOCKET_ENV: &str = "ULTRAVOX_VOICE_SOCKET";

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct VoiceRequest {
    version: u8,
    request_id: String,
    #[serde(flatten)]
    command: VoiceCommand,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(tag = "command", rename_all = "snake_case")]
enum VoiceCommand {
    Health,
    Start {
        #[serde(rename = "recordingId", alias = "recording_id")]
        recording_id: String,
    },
    Stop {
        #[serde(rename = "recordingId", alias = "recording_id")]
        recording_id: String,
    },
    Status {
        #[serde(rename = "recordingId", alias = "recording_id")]
        recording_id: String,
    },
    Cancel {
        #[serde(rename = "recordingId", alias = "recording_id")]
        recording_id: String,
    },
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct VoiceResponse {
    version: u8,
    request_id: String,
    ok: bool,
    state: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    recording_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    transcript: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    audio_level: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

impl VoiceResponse {
    fn success(request_id: String, state: impl Into<String>) -> Self {
        Self {
            version: PROTOCOL_VERSION,
            request_id,
            ok: true,
            state: state.into(),
            recording_id: None,
            transcript: None,
            audio_level: None,
            error: None,
        }
    }

    fn failure(request_id: String, error: impl Into<String>) -> Self {
        Self {
            version: PROTOCOL_VERSION,
            request_id,
            ok: false,
            state: "error".to_string(),
            recording_id: None,
            transcript: None,
            audio_level: None,
            error: Some(error.into()),
        }
    }
}

#[derive(Default)]
struct VoiceIpcState {
    cancelled: Mutex<HashSet<Uuid>>,
}

fn app_socket_dir() -> PathBuf {
    std::env::temp_dir().join("com.imploselabs.ultravox")
}

fn socket_path_from_env(env: Option<&std::ffi::OsStr>) -> PathBuf {
    env.map(PathBuf::from)
        .unwrap_or_else(|| app_socket_dir().join("voice-v1.sock"))
}

pub fn socket_path() -> PathBuf {
    socket_path_from_env(std::env::var_os(SOCKET_ENV).as_deref())
}

fn is_managed_socket_dir(path: &std::path::Path) -> bool {
    path == app_socket_dir()
}

fn owner_path() -> PathBuf {
    socket_path().with_extension("pid")
}

pub fn start(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        if let Err(error) = serve(app).await {
            eprintln!("UltraVox voice IPC stopped: {error}");
        }
    });
}

pub fn cleanup_socket() {
    let owner = owner_path();
    let owned_by_this_process = std::fs::read_to_string(&owner)
        .ok()
        .and_then(|value| value.trim().parse::<u32>().ok())
        == Some(std::process::id());
    if !owned_by_this_process {
        return;
    }
    let _ = std::fs::remove_file(socket_path());
    let _ = std::fs::remove_file(owner);
}

async fn serve(app: AppHandle) -> Result<(), String> {
    let path = socket_path();
    let parent = path
        .parent()
        .ok_or_else(|| "voice socket path has no parent directory".to_string())?;
    std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    if is_managed_socket_dir(parent) {
        std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700))
            .map_err(|e| e.to_string())?;
    }

    if path.exists() {
        if UnixStream::connect(&path).await.is_ok() {
            return Err(format!("voice socket already active at {}", path.display()));
        }
        std::fs::remove_file(&path).map_err(|e| e.to_string())?;
        let _ = std::fs::remove_file(owner_path());
    }

    let listener = UnixListener::bind(&path).map_err(|e| e.to_string())?;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
        .map_err(|e| e.to_string())?;
    std::fs::write(owner_path(), std::process::id().to_string()).map_err(|e| e.to_string())?;
    std::fs::set_permissions(owner_path(), std::fs::Permissions::from_mode(0o600))
        .map_err(|e| e.to_string())?;
    let service = Arc::new(VoiceIpcState::default());

    loop {
        let (stream, _) = listener.accept().await.map_err(|e| e.to_string())?;
        let app = app.clone();
        let service = service.clone();
        tokio::spawn(async move {
            if let Err(error) = serve_connection(stream, app, service).await {
                eprintln!("UltraVox voice IPC request failed: {error}");
            }
        });
    }
}

async fn serve_connection(
    mut stream: UnixStream,
    app: AppHandle,
    service: Arc<VoiceIpcState>,
) -> Result<(), String> {
    let credentials = stream.peer_cred().map_err(|e| e.to_string())?;
    if credentials.uid() != unsafe { libc::geteuid() } {
        return Err("voice IPC rejected a client owned by another user".to_string());
    }

    let request: VoiceRequest = read_json_frame(&mut stream)
        .await
        .map_err(|e| e.to_string())?;
    let request_id = request.request_id.clone();
    let response = if request.version != PROTOCOL_VERSION {
        VoiceResponse::failure(
            request_id,
            format!(
                "unsupported voice protocol version {}; expected {PROTOCOL_VERSION}",
                request.version
            ),
        )
    } else {
        match handle_request(app, service, request).await {
            Ok(response) => response,
            Err(error) => VoiceResponse::failure(request_id, error),
        }
    };
    write_json_frame(&mut stream, &response)
        .await
        .map_err(|e| e.to_string())
}

async fn handle_request(
    app: AppHandle,
    service: Arc<VoiceIpcState>,
    request: VoiceRequest,
) -> Result<VoiceResponse, String> {
    let request_id = request.request_id;
    let state = app.state::<AppState>();

    match request.command {
        VoiceCommand::Health => Ok(VoiceResponse::success(request_id, "ready")),
        VoiceCommand::Start { recording_id } => {
            let id = parse_recording_id(&recording_id)?;
            let recording_id = state.begin_with_id(id).await?;
            let mut response = VoiceResponse::success(request_id, "recording");
            response.recording_id = Some(recording_id);
            Ok(response)
        }
        VoiceCommand::Stop { recording_id } => {
            let id = parse_recording_id(&recording_id)?;
            ensure_active_recording(&state, id).await?;
            state.finish_recording(false).await?;
            let mut response = VoiceResponse::success(request_id, "transcribing");
            response.recording_id = Some(recording_id);
            Ok(response)
        }
        VoiceCommand::Status { recording_id } => {
            let id = parse_recording_id(&recording_id)?;

            let is_active = state
                .session
                .lock()
                .await
                .as_ref()
                .is_some_and(|session| session.id == id);
            if is_active {
                let level = state.audio.lock().await.current_input_level();
                let mut response = VoiceResponse::success(request_id, "recording");
                response.recording_id = Some(recording_id);
                response.audio_level = Some(level);
                return Ok(response);
            }

            // Prefer real history so completed/failed rows are not masked by
            // the in-memory cancellation set.
            if let Some(row) = state
                .history
                .lock()
                .map_err(|e| e.to_string())?
                .get(id)
                .map_err(|e| e.to_string())?
            {
                let mut response = VoiceResponse::success(request_id, row.status.as_str());
                response.recording_id = Some(recording_id);
                if row.status == ultravox_core::RecordingStatus::Completed {
                    response.transcript = Some(row.transcription);
                } else if row.status == ultravox_core::RecordingStatus::Failed {
                    response.ok = false;
                    response.error = Some(row.transcription);
                }
                return Ok(response);
            }

            // Only rely on the in-memory set when the row has not reached the
            // history database yet.
            if service
                .cancelled
                .lock()
                .map_err(|e| e.to_string())?
                .contains(&id)
            {
                let mut response = VoiceResponse::success(request_id, "cancelled");
                response.recording_id = Some(recording_id);
                return Ok(response);
            }

            Err(format!("recording {recording_id} was not found"))
        }
        VoiceCommand::Cancel { recording_id } => {
            let id = parse_recording_id(&recording_id)?;
            let cancelled = state.cancel_recording_or_transcription(id).await?;
            if !cancelled {
                // A row that is already cancelled in history is idempotent.
                if let Some(row) = state
                    .history
                    .lock()
                    .map_err(|e| e.to_string())?
                    .get(id)
                    .map_err(|e| e.to_string())?
                {
                    if row.status == ultravox_core::RecordingStatus::Cancelled {
                        let mut response = VoiceResponse::success(request_id, "cancelled");
                        response.recording_id = Some(recording_id);
                        return Ok(response);
                    }
                }
                return Err(format!(
                    "recording {recording_id} is not active or transcribing"
                ));
            }
            // Only remember in-memory cancellations that actually occurred.
            service
                .cancelled
                .lock()
                .map_err(|e| e.to_string())?
                .insert(id);
            let mut response = VoiceResponse::success(request_id, "cancelled");
            response.recording_id = Some(recording_id);
            Ok(response)
        }
    }
}

fn parse_recording_id(value: &str) -> Result<Uuid, String> {
    Uuid::parse_str(value).map_err(|_| format!("invalid recording id: {value}"))
}

async fn ensure_active_recording(state: &AppState, id: Uuid) -> Result<(), String> {
    let session = state.session.lock().await;
    match session.as_ref() {
        Some(session) if session.id == id => Ok(()),
        Some(_) => Err("another recording is active".to_string()),
        None => Err("recording is not active".to_string()),
    }
}

async fn read_json_frame<T: for<'de> Deserialize<'de>>(stream: &mut UnixStream) -> io::Result<T> {
    let length = stream.read_u32().await? as usize;
    if length == 0 || length > MAX_FRAME_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("invalid voice IPC frame length: {length}"),
        ));
    }
    let mut payload = vec![0_u8; length];
    stream.read_exact(&mut payload).await?;
    serde_json::from_slice(&payload).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

async fn write_json_frame<T: Serialize>(stream: &mut UnixStream, value: &T) -> io::Result<()> {
    let payload =
        serde_json::to_vec(value).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    if payload.len() > MAX_FRAME_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "voice IPC response exceeded frame limit",
        ));
    }
    stream.write_u32(payload.len() as u32).await?;
    stream.write_all(&payload).await?;
    stream.flush().await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_socket_path_uses_managed_directory() {
        let path = socket_path_from_env(None);
        assert_eq!(path, app_socket_dir().join("voice-v1.sock"));
    }

    #[test]
    fn custom_socket_path_uses_env_value() {
        let path = socket_path_from_env(Some(std::ffi::OsStr::new("/tmp/custom.sock")));
        assert_eq!(path, std::path::PathBuf::from("/tmp/custom.sock"));
    }

    #[test]
    fn managed_directory_is_recognized() {
        assert!(is_managed_socket_dir(&app_socket_dir()));
        assert!(!is_managed_socket_dir(std::path::Path::new("/tmp")));
    }

    #[test]
    fn protocol_round_trip_preserves_start_request() {
        let request = VoiceRequest {
            version: PROTOCOL_VERSION,
            request_id: "request-1".to_string(),
            command: VoiceCommand::Start {
                recording_id: "550e8400-e29b-41d4-a716-446655440000".to_string(),
            },
        };
        let encoded = serde_json::to_vec(&request).unwrap();
        let decoded: VoiceRequest = serde_json::from_slice(&encoded).unwrap();
        assert_eq!(decoded.version, PROTOCOL_VERSION);
        assert_eq!(decoded.request_id, "request-1");
        match decoded.command {
            VoiceCommand::Start { recording_id } => {
                assert_eq!(recording_id, "550e8400-e29b-41d4-a716-446655440000");
            }
            _ => panic!("expected Start command"),
        }
    }

    #[test]
    fn stop_command_accepts_public_and_legacy_recording_id_fields() {
        for field in ["recordingId", "recording_id"] {
            let mut request = serde_json::json!({
                "version": PROTOCOL_VERSION,
                "requestId": "request-stop",
                "command": "stop",
            });
            request[field] = serde_json::json!("550e8400-e29b-41d4-a716-446655440000");
            let decoded: VoiceRequest = serde_json::from_value(request).unwrap();
            match decoded.command {
                VoiceCommand::Stop { recording_id } => {
                    assert_eq!(recording_id, "550e8400-e29b-41d4-a716-446655440000");
                }
                _ => panic!("expected Stop command"),
            }
        }
    }

    #[test]
    fn status_command_accepts_public_and_legacy_recording_id_fields() {
        for field in ["recordingId", "recording_id"] {
            let mut request = serde_json::json!({
                "version": PROTOCOL_VERSION,
                "requestId": "request-status",
                "command": "status",
            });
            request[field] = serde_json::json!("550e8400-e29b-41d4-a716-446655440000");
            let decoded: VoiceRequest = serde_json::from_value(request).unwrap();
            match decoded.command {
                VoiceCommand::Status { recording_id } => {
                    assert_eq!(recording_id, "550e8400-e29b-41d4-a716-446655440000");
                }
                _ => panic!("expected Status command"),
            }
        }
    }

    #[test]
    fn cancel_command_accepts_public_and_legacy_recording_id_fields() {
        for field in ["recordingId", "recording_id"] {
            let mut request = serde_json::json!({
                "version": PROTOCOL_VERSION,
                "requestId": "request-cancel",
                "command": "cancel",
            });
            request[field] = serde_json::json!("550e8400-e29b-41d4-a716-446655440000");
            let decoded: VoiceRequest = serde_json::from_value(request).unwrap();
            match decoded.command {
                VoiceCommand::Cancel { recording_id } => {
                    assert_eq!(recording_id, "550e8400-e29b-41d4-a716-446655440000");
                }
                _ => panic!("expected Cancel command"),
            }
        }
    }

    #[test]
    fn start_command_accepts_public_and_legacy_recording_id_fields() {
        for field in ["recordingId", "recording_id"] {
            let mut request = serde_json::json!({
                "version": PROTOCOL_VERSION,
                "requestId": "request-1",
                "command": "start",
            });
            request[field] = serde_json::json!("550e8400-e29b-41d4-a716-446655440000");
            let decoded: VoiceRequest = serde_json::from_value(request).unwrap();
            match decoded.command {
                VoiceCommand::Start { recording_id } => {
                    assert_eq!(recording_id, "550e8400-e29b-41d4-a716-446655440000");
                }
                _ => panic!("expected Start command"),
            }
        }
    }

    #[test]
    fn active_status_response_serializes_audio_level() {
        let mut response = VoiceResponse::success("request-level".to_string(), "recording");
        response.recording_id = Some("550e8400-e29b-41d4-a716-446655440000".to_string());
        response.audio_level = Some(0.42);
        let value = serde_json::to_value(response).unwrap();
        assert_eq!(value["state"], "recording");
        assert_eq!(value["recordingId"], "550e8400-e29b-41d4-a716-446655440000");
        assert!((value["audioLevel"].as_f64().unwrap() - 0.42).abs() < 1e-6);
        assert!(value.get("transcript").is_none());
        assert!(value.get("error").is_none());
    }

    #[test]
    fn response_omits_absent_optional_fields() {
        let response = VoiceResponse::success("request-2".to_string(), "ready");
        let value = serde_json::to_value(response).unwrap();
        assert_eq!(value["state"], "ready");
        assert!(value.get("recordingId").is_none());
        assert!(value.get("transcript").is_none());
        assert!(value.get("audioLevel").is_none());
        assert!(value.get("error").is_none());
    }

    #[test]
    fn wrong_id_cancel_request_parses_recording_id() {
        let request = serde_json::json!({
            "version": PROTOCOL_VERSION,
            "requestId": "request-cancel-wrong",
            "command": "cancel",
            "recordingId": "12345678-1234-1234-1234-123456789abc"
        });
        let decoded: VoiceRequest = serde_json::from_value(request).unwrap();
        match decoded.command {
            VoiceCommand::Cancel { recording_id } => {
                assert_eq!(recording_id, "12345678-1234-1234-1234-123456789abc");
            }
            _ => panic!("expected Cancel command"),
        }
    }

    #[test]
    fn duplicate_start_requests_have_same_id_shape() {
        let id = "550e8400-e29b-41d4-a716-446655440000";
        let first = serde_json::json!({
            "version": PROTOCOL_VERSION,
            "requestId": "req-1",
            "command": "start",
            "recording_id": id
        });
        let second = serde_json::json!({
            "version": PROTOCOL_VERSION,
            "requestId": "req-2",
            "command": "start",
            "recording_id": id
        });
        let first_decoded: VoiceRequest = serde_json::from_value(first).unwrap();
        let second_decoded: VoiceRequest = serde_json::from_value(second).unwrap();
        match (first_decoded.command, second_decoded.command) {
            (VoiceCommand::Start { recording_id: a }, VoiceCommand::Start { recording_id: b }) => {
                assert_eq!(a, b);
                assert_eq!(a, id);
            }
            _ => panic!("expected two Start commands"),
        }
    }

    #[test]
    fn cancelled_status_response_serializes_recording_id() {
        let mut response = VoiceResponse::success("request-3".to_string(), "cancelled");
        response.recording_id = Some("550e8400-e29b-41d4-a716-446655440000".to_string());
        let value = serde_json::to_value(response).unwrap();
        assert_eq!(value["state"], "cancelled");
        assert_eq!(value["recordingId"], "550e8400-e29b-41d4-a716-446655440000");
        assert!(value.get("transcript").is_none());
        assert!(value.get("error").is_none());
    }

    #[test]
    fn concurrent_transcription_error_response_shape() {
        // Documents the error returned when finish_recording would orphan a live
        // recording because an earlier transcription is still active. Keeps the
        // serialization contract testable without a full AppState.
        let mut response = VoiceResponse::failure(
            "request-finish".to_string(),
            "another transcription is still active",
        );
        response.recording_id = Some("550e8400-e29b-41d4-a716-446655440000".to_string());
        let value = serde_json::to_value(response).unwrap();
        assert_eq!(value["ok"], false);
        assert_eq!(value["state"], "error");
        assert_eq!(value["error"], "another transcription is still active");
        assert_eq!(value["recordingId"], "550e8400-e29b-41d4-a716-446655440000");
    }
}
