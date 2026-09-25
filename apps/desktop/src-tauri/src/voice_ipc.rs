use std::collections::HashSet;
use std::io;
#[cfg(target_os = "macos")]
use std::os::unix::ffi::OsStringExt;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::os::unix::io::AsRawFd;
use std::path::PathBuf;
use std::sync::{Arc, LazyLock, Mutex};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::broadcast;
use uuid::Uuid;

use crate::events::MeetingDetection;
use crate::state::{AppState, MeetingDetectionRegistration, MEETING_DETECTION_TTL};

const PROTOCOL_VERSION: u8 = 1;
const MAX_FRAME_BYTES: usize = 64 * 1024;
const MAX_REQUEST_ID_BYTES: usize = 128;
const SOCKET_ENV: &str = "ULTRAVOX_VOICE_SOCKET";
const RECORDING_TRIGGERED_KIND: &str = "recordingTriggered";

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
    ShowSettings,
    Listen,
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
    MeetingDetected {
        detection: MeetingDetection,
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

/// Push frames buffered per listening connection before a lagged subscriber
/// is dropped so UltraTerm reconnects cleanly instead of desyncing down/up
/// bookkeeping.
const PUSH_CHANNEL_CAPACITY: usize = 64;

#[derive(Clone)]
struct VoiceTriggerHub {
    tx: broadcast::Sender<serde_json::Value>,
}

impl VoiceTriggerHub {
    fn new() -> Self {
        let (tx, _) = broadcast::channel(PUSH_CHANNEL_CAPACITY);
        Self { tx }
    }

    /// Registers an additional live receiver; unregistering happens when the
    /// receiver (its owning connection task) is dropped.
    fn subscribe(&self) -> broadcast::Receiver<serde_json::Value> {
        self.tx.subscribe()
    }

    /// Publishes `frame` to every live subscriber. Returns false when nobody
    /// was listening so callers fall back to the native flow.
    fn publish(&self, frame: serde_json::Value) -> bool {
        self.tx.send(frame).is_ok_and(|delivered| delivered > 0)
    }
}

static VOICE_TRIGGER_HUB: LazyLock<VoiceTriggerHub> = LazyLock::new(VoiceTriggerHub::new);

fn voice_trigger_hub() -> &'static VoiceTriggerHub {
    &VOICE_TRIGGER_HUB
}

/// Builds a `recordingTriggered` push frame exactly as contract v1 defines it.
fn recording_triggered_frame(action: &str, combo: &str) -> serde_json::Value {
    serde_json::json!({
        "version": PROTOCOL_VERSION,
        "kind": RECORDING_TRIGGERED_KIND,
        "action": action,
        "combo": combo,
        "triggerId": Uuid::new_v4().to_string(),
    })
}

/// Publishes a `recordingTriggered` push frame to every live `listen`
/// subscriber over the shared voice IPC hub.
///
/// Returns true iff at least one live subscriber existed at publish time so
/// callers suppress UltraVox's native recorder flow only when the trigger was
/// actually forwarded (contract v1 routing decision).
pub fn publish_voice_trigger(combo: &str, action: &str) -> bool {
    voice_trigger_hub().publish(recording_triggered_frame(action, combo))
}

#[derive(Default)]
struct VoiceIpcState {
    cancelled: Mutex<HashSet<Uuid>>,
}

#[cfg(target_os = "macos")]
fn darwin_user_temp_dir() -> Option<PathBuf> {
    let length = unsafe { libc::confstr(libc::_CS_DARWIN_USER_TEMP_DIR, std::ptr::null_mut(), 0) };
    if length <= 1 {
        return None;
    }
    let mut buffer = vec![0_u8; length];
    let written = unsafe {
        libc::confstr(
            libc::_CS_DARWIN_USER_TEMP_DIR,
            buffer.as_mut_ptr().cast(),
            buffer.len(),
        )
    };
    if written <= 1 {
        return None;
    }
    buffer.truncate(written - 1);
    Some(PathBuf::from(std::ffi::OsString::from_vec(buffer)))
}

fn shared_temp_dir() -> &'static PathBuf {
    static TEMP_DIR: LazyLock<PathBuf> = LazyLock::new(|| {
        #[cfg(target_os = "macos")]
        if let Some(path) = darwin_user_temp_dir() {
            return path;
        }
        std::env::temp_dir()
    });
    &TEMP_DIR
}

fn app_socket_dir() -> PathBuf {
    shared_temp_dir().join("com.imploselabs.ultravox")
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
fn ensure_managed_socket_dir(path: &std::path::Path) -> Result<(), String> {
    if !is_managed_socket_dir(path) {
        return Ok(());
    }
    prepare_managed_socket_dir(path)
}

fn prepare_managed_socket_dir(path: &std::path::Path) -> Result<(), String> {
    let metadata = std::fs::symlink_metadata(path).map_err(|error| error.to_string())?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err("managed voice socket directory is not a real directory".to_string());
    }
    if metadata.uid() != unsafe { libc::geteuid() } {
        return Err("managed voice socket directory has the wrong owner".to_string());
    }
    if metadata.mode() & 0o077 != 0 {
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
            .map_err(|error| error.to_string())?;
    }

    let metadata = std::fs::symlink_metadata(path).map_err(|error| error.to_string())?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err("managed voice socket directory is not a real directory".to_string());
    }
    if metadata.uid() != unsafe { libc::geteuid() } {
        return Err("managed voice socket directory has the wrong owner".to_string());
    }
    if metadata.mode() & 0o077 != 0 {
        return Err("managed voice socket directory permissions are too broad".to_string());
    }
    Ok(())
}

fn ensure_managed_entry(path: &std::path::Path) -> Result<(), String> {
    let metadata = std::fs::symlink_metadata(path).map_err(|error| error.to_string())?;
    if metadata.file_type().is_symlink()
        || metadata.uid() != unsafe { libc::geteuid() }
        || metadata.mode() & 0o077 != 0
    {
        return Err("managed voice IPC path has unsafe ownership or permissions".to_string());
    }
    Ok(())
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
    let managed_dir = is_managed_socket_dir(parent);
    std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    ensure_managed_socket_dir(parent)?;
    if managed_dir && path.exists() {
        ensure_managed_entry(&path)?;
    }
    if managed_dir && owner_path().exists() {
        ensure_managed_entry(&owner_path())?;
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
fn verify_peer_uid(stream: &UnixStream) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        let mut effective_uid = 0;
        let mut effective_gid = 0;
        let result =
            unsafe { libc::getpeereid(stream.as_raw_fd(), &mut effective_uid, &mut effective_gid) };
        if result != 0 || effective_uid != unsafe { libc::geteuid() } {
            return Err("voice IPC rejected a client owned by another user".to_string());
        }
        return Ok(());
    }
    #[cfg(not(target_os = "macos"))]
    {
        let credentials = stream.peer_cred().map_err(|e| e.to_string())?;
        if credentials.uid() != unsafe { libc::geteuid() } {
            return Err("voice IPC rejected a client owned by another user".to_string());
        }
        Ok(())
    }
}

async fn serve_connection(
    mut stream: UnixStream,
    app: AppHandle,
    service: Arc<VoiceIpcState>,
) -> Result<(), String> {
    verify_peer_uid(&stream)?;

    let request: VoiceRequest = read_json_frame(&mut stream)
        .await
        .map_err(|e| e.to_string())?;
    let request_id = request.request_id.clone();
    if !valid_request_id(&request.request_id) {
        let response = VoiceResponse::failure(String::new(), "invalid request id");
        return write_json_frame(&mut stream, &response)
            .await
            .map_err(|e| e.to_string());
    }
    if request.version != PROTOCOL_VERSION {
        let response = VoiceResponse::failure(
            request_id,
            format!(
                "unsupported voice protocol version {}; expected {PROTOCOL_VERSION}",
                request.version
            ),
        );
        return write_json_frame(&mut stream, &response)
            .await
            .map_err(|e| e.to_string());
    }
    if matches!(request.command, VoiceCommand::Listen) {
        // A valid `listen` request holds this connection open as a push
        // subscriber instead of using the one-request-per-connection path.
        return serve_voice_trigger_listener(stream, request_id, voice_trigger_hub()).await;
    }
    let response = match handle_request(app, service, request).await {
        Ok(response) => response,
        Err(error) => VoiceResponse::failure(request_id.clone(), error),
    };
    write_json_frame(&mut stream, &response)
        .await
        .map_err(|e| e.to_string())
}

/// Handles a validated `listen` command: registers the connection as a live
/// recording-trigger subscriber, replies once with the listening ack, then
/// holds the connection open pushing every published `recordingTriggered`
/// frame.
///
/// Registration happens before the ack so triggers published immediately
/// afterward are buffered rather than lost. Returning drops the receiver,
/// which unregisters the subscriber: unexpected additional request frames are
/// a client bug per contract v1 and close the connection, while a clean client
/// EOF simply ends it without logging a failure.
async fn serve_voice_trigger_listener(
    mut stream: UnixStream,
    request_id: String,
    hub: &VoiceTriggerHub,
) -> Result<(), String> {
    let mut rx = hub.subscribe();
    let ack = VoiceResponse::success(request_id, "listening");
    write_json_frame(&mut stream, &ack)
        .await
        .map_err(|e| format!("failed to acknowledge voice IPC listen command: {e}"))?;

    loop {
        tokio::select! {
            biased;

            push = rx.recv() => {
                match push {
                    Ok(frame) => {
                        write_json_frame(&mut stream, &frame)
                            .await
                            .map_err(|e| format!("failed to deliver voice trigger push frame: {e}"))?;
                    }
                    Err(broadcast::error::RecvError::Lagged(dropped)) => {
                        // Missed down/up pairs corrupt UltraTerm's press
                        // bookkeeping; dropping the connection forces a clean
                        // reconnect instead of delivering stale state.
                        return Err(format!(
                            "voice trigger subscriber fell behind by {dropped} push frames"
                        ));
                    }
                    Err(broadcast::error::RecvError::Closed) => {
                        return Ok(());
                    }
                }
            }
            incoming = read_json_frame::<serde_json::Value>(&mut stream) => {
                match incoming {
                    // Contract v1: any further request frame on a listening
                    // connection is a client bug; close after unregistering.
                    Ok(_) => {
                        return Err(
                            "listening voice IPC connection sent an unexpected additional request"
                                .to_string(),
                        );
                    }
                    Err(error)
                        if matches!(
                            error.kind(),
                            io::ErrorKind::UnexpectedEof
                                | io::ErrorKind::ConnectionReset
                                | io::ErrorKind::BrokenPipe
                        ) =>
                    {
                        // Normal disconnect; EOF alone unregisters via drop.
                        return Ok(());
                    }
                    Err(error) => {
                        return Err(format!("voice trigger subscriber disconnected: {error}"));
                    }
                }
            }
        }
    }
}

fn valid_request_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_REQUEST_ID_BYTES
        && value.bytes().all(|byte| !byte.is_ascii_control())
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
        VoiceCommand::ShowSettings => {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.unminimize();
                let _ = window.show();
                let _ = window.set_focus();
            }
            let _ = app.emit("navigate-to", "settings");
            Ok(VoiceResponse::success(request_id, "settings"))
        }
        VoiceCommand::MeetingDetected { detection } => {
            let _transition = state.activity_transition.lock().await;
            let detection_id = detection.detection_id.clone();
            let rollback_detection = detection.clone();
            let (registration, payload) = state.register_meeting_detection(detection).await?;
            if registration == MeetingDetectionRegistration::Prompt {
                let payload = payload.ok_or_else(|| {
                    "meeting detection prompt payload was not created".to_string()
                })?;
                if let Err(error) = crate::show_meeting_reminder(&app) {
                    state.rollback_meeting_detection(&rollback_detection).await;
                    return Err(error);
                }
                if let Err(error) = state
                    .app
                    .emit(crate::events::MEETING_DETECTION_PENDING, &payload)
                {
                    state.rollback_meeting_detection(&rollback_detection).await;
                    crate::close_meeting_reminder(&app);
                    return Err(error.to_string());
                }
                let expiry_app = app.clone();
                tauri::async_runtime::spawn(async move {
                    tokio::time::sleep(MEETING_DETECTION_TTL).await;
                    let expiry_state = expiry_app.state::<AppState>();
                    if expiry_state.expire_meeting_detection(&detection_id).await {
                        crate::close_meeting_reminder(&expiry_app);
                    }
                });
            }
            let response_state = match registration {
                MeetingDetectionRegistration::Prompt => "prompted",
                MeetingDetectionRegistration::Disabled => "disabled",
                MeetingDetectionRegistration::Active => "active",
                MeetingDetectionRegistration::Duplicate => "duplicate",
                MeetingDetectionRegistration::Pending => "pending",
            };
            Ok(VoiceResponse::success(request_id, response_state))
        }
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
        // Dispatched to serve_voice_trigger_listener before requests ever
        // reach here; kept unreachable-by-contract and defensive.
        VoiceCommand::Listen => {
            Err("voice IPC listen command cannot be handled as a one-shot request".to_string())
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

    fn isolated_test_dir() -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!(
            "ultravox-voice-ipc-test-{}-{}",
            std::process::id(),
            Uuid::new_v4()
        ));
        std::fs::create_dir(&path).unwrap();
        path
    }

    #[test]
    fn managed_directory_tightens_existing_permissions() {
        let path = isolated_test_dir();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();

        prepare_managed_socket_dir(&path).unwrap();

        let mode = std::fs::symlink_metadata(&path).unwrap().mode() & 0o777;
        assert_eq!(mode, 0o700);
        std::fs::remove_dir(&path).unwrap();
    }

    #[test]
    fn managed_directory_rejects_unsafe_path_types() {
        let root = isolated_test_dir();
        let file = root.join("file");
        std::fs::write(&file, b"not a directory").unwrap();
        assert!(prepare_managed_socket_dir(&file).is_err());

        let symlink = root.join("symlink");
        std::os::unix::fs::symlink(&file, &symlink).unwrap();
        assert!(prepare_managed_socket_dir(&symlink).is_err());

        std::fs::remove_dir_all(&root).unwrap();
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
            "recordingId": id
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
    fn meeting_detection_request_round_trips_without_legacy_changes() {
        let request = serde_json::json!({
            "version": PROTOCOL_VERSION,
            "requestId": "meeting-request",
            "command": "meeting_detected",
            "detection": {
                "version": 1,
                "detection_id": "det-1",
                "provider": "google_meet",
                "meeting_key": "a".repeat(64),
                "detected_at_ms": 1_000
            }
        });
        let parsed: VoiceRequest = serde_json::from_value(request).unwrap();
        assert!(matches!(
            parsed.command,
            VoiceCommand::MeetingDetected { .. }
        ));
    }

    #[test]
    fn request_id_bounds_are_enforced() {
        assert!(valid_request_id("ok"));
        assert!(!valid_request_id(""));
        assert!(!valid_request_id(&"x".repeat(MAX_REQUEST_ID_BYTES + 1)));
    }
    #[test]
    fn invalid_request_id_response_uses_bounded_sentinel() {
        let response = VoiceResponse::failure(String::new(), "invalid request id");
        let value = serde_json::to_value(response).unwrap();
        assert_eq!(value["requestId"], "");
        assert!(serde_json::to_vec(&value).unwrap().len() < MAX_FRAME_BYTES);
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

    static ENV_SOCKET_LOCK: Mutex<()> = Mutex::new(());

    /// Overrides ULTRAVOX_VOICE_SOCKET for the duration of the returned guard.
    /// Env mutation is process-global, so env-touching tests serialize behind
    /// one lock; the remaining socket-path tests pass values as parameters and
    /// never read the environment.
    fn isolate_socket_env(dir: &std::path::Path) -> SocketEnvGuard {
        let lock = ENV_SOCKET_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let previous = std::env::var_os(SOCKET_ENV);
        std::env::set_var(SOCKET_ENV, dir.join("voice-v1.sock"));
        SocketEnvGuard {
            previous,
            _lock: lock,
        }
    }

    struct SocketEnvGuard {
        previous: Option<std::ffi::OsString>,
        _lock: std::sync::MutexGuard<'static, ()>,
    }

    impl Drop for SocketEnvGuard {
        fn drop(&mut self) {
            match self.previous.take() {
                Some(previous) => std::env::set_var(SOCKET_ENV, previous),
                None => std::env::remove_var(SOCKET_ENV),
            }
        }
    }

    #[tokio::test]
    async fn listen_command_roundtrips_pushes_through_real_socket() {
        // Unix-domain paths must fit SUN_LEN, so this test uses a compact
        // directory name instead of the longer isolated_test_dir pattern.
        let root = std::env::temp_dir().join(format!("uvvxs-{}", Uuid::new_v4().simple()));
        std::fs::create_dir(&root).expect("create isolated test dir");
        let _env = isolate_socket_env(&root);
        // Honors the ULTRAVOX_VOICE_SOCKET override plumbed through socket_path().
        let path = socket_path();
        assert!(path.starts_with(&root));

        let listener = UnixListener::bind(&path).expect("bind isolated test socket");
        let hub = Arc::new(VoiceTriggerHub::new());

        let hub_for_task = Arc::clone(&hub);

        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.expect("accept test client");
            verify_peer_uid(&stream).expect("same-uid client passes peer verification");
            let mut stream = stream;
            let request: VoiceRequest = read_json_frame(&mut stream)
                .await
                .expect("listen request parses through framing");
            assert!(matches!(request.command, VoiceCommand::Listen));
            assert_eq!(request.version, PROTOCOL_VERSION);
            serve_voice_trigger_listener(stream, request.request_id.clone(), &hub_for_task).await
        });

        let mut client = UnixStream::connect(&path)
            .await
            .expect("connect through the real filesystem socket node");
        write_json_frame(
            &mut client,
            &serde_json::json!({
                "version": PROTOCOL_VERSION,
                "requestId": "listen-roundtrip",
                "command": "listen",
            }),
        )
        .await
        .unwrap();

        let ack_value: serde_json::Value = read_json_frame(&mut client)
            .await
            .expect("listening ack frame");
        assert_eq!(
            ack_value,
            serde_json::json!({
                "version": PROTOCOL_VERSION,
                "requestId": "listen-roundtrip",
                "ok": true,
                "state": "listening",
            })
        );

        // One push per hotkey event, FIFO delivery over the held-open socket.
        hub.publish(recording_triggered_frame("down", "Option+Backtick"));
        let down: serde_json::Value = read_json_frame(&mut client)
            .await
            .expect("first push frame");
        assert_eq!(down["kind"], RECORDING_TRIGGERED_KIND);
        assert_eq!(down["action"], "down");
        assert_eq!(down["combo"], "Option+Backtick");
        Uuid::parse_str(down["triggerId"].as_str().expect("trigger id")).unwrap();

        hub.publish(recording_triggered_frame("up", "Option+Backtick"));
        let up: serde_json::Value = read_json_frame(&mut client)
            .await
            .expect("second push frame");
        assert_eq!(up["action"], "up");

        // EOF disconnect unregisters automatically.
        drop(client);
        let outcome = server.await.expect("server task joined");
        assert!(
            outcome.is_ok(),
            "a clean disconnect should end quietly, got {outcome:?}"
        );
        assert!(!hub.publish(recording_triggered_frame("down", "Option+Backtick")));

        std::fs::remove_file(&path).ok();
        std::fs::remove_dir_all(&root).ok();
    }

    #[tokio::test]
    async fn extra_request_frame_closes_listening_connection() {
        let hub = Arc::new(VoiceTriggerHub::new());
        let (mut client, server_stream) = tokio::net::UnixStream::pair().expect("socketpair");

        let hub_for_task = Arc::clone(&hub);
        let server = tokio::spawn(async move {
            serve_voice_trigger_listener(server_stream, "listen-extra".to_string(), &hub_for_task)
                .await
        });

        let ack: serde_json::Value = read_json_frame(&mut client)
            .await
            .expect("listening ack arrives first");
        assert_eq!(ack["state"], "listening");

        // Contract v1: an additional request frame on a listening connection is
        // a client bug; close it after unregistering the subscriber.
        write_json_frame(
            &mut client,
            &serde_json::json!({
                "version": PROTOCOL_VERSION,
                "requestId": "extra-health",
                "command": "health",
            }),
        )
        .await
        .unwrap();
        drop(client);

        let outcome = server.await.expect("server task joined");
        let closed_for_unexpected_request = matches!(
            &outcome,
            Err(message) if message.contains("unexpected additional request")
        );
        assert!(
            closed_for_unexpected_request,
            "expected unexpected-request closure, got {outcome:?}"
        );
        assert!(!hub.publish(recording_triggered_frame("up", "Option+Backtick")));
    }

    #[test]
    fn publishing_without_live_listener_reports_unhandled() {
        // forward_voice_trigger's decline path stays reachable without an
        // AppHandle: with no live subscribers, handled must be false.
        assert!(!publish_voice_trigger("Option+Backtick", "down"));

        // A subscribed-then-dropped receiver does not resurrect handling.
        let hub = VoiceTriggerHub::new();
        drop(hub.subscribe());
        assert!(!hub.publish(recording_triggered_frame("down", "Option+Backtick")));
    }
}
