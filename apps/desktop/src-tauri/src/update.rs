use std::fs;
use std::path::{Path, PathBuf};
use std::process::{self, Command, Stdio};
use std::sync::OnceLock;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tauri::AppHandle;

/// Canonical public release repository. All builds update from its releases.
pub const PUBLIC_RELEASE_REPO: &str = "michael-berardi/ultravox";

/// Private installed-candidate QA only: a staged feed root serving
/// `releases` (GitHub release-list JSON) and `download/<asset>`. Candidates
/// from it still pass every checksum, Developer ID and notarization check.
#[cfg(feature = "local-qa")]
fn qa_feed() -> Option<String> {
    std::env::var("ULTRAVOX_QA_UPDATE_FEED")
        .ok()
        .and_then(|value| loopback_feed(&value))
}

/// A test feed root only when it is plain HTTP on this machine's loopback
/// interface: no other host, no credentials, query or fragment. A prefix check
/// would accept `http://127.0.0.1:1@example.com/`, whose host is not local.
#[cfg_attr(not(feature = "local-qa"), allow(dead_code))]
fn loopback_feed(value: &str) -> Option<String> {
    let url = reqwest::Url::parse(value).ok()?;
    let local = matches!(url.host_str(), Some("127.0.0.1" | "localhost"));
    (url.scheme() == "http"
        && local
        && url.port().is_some()
        && url.username().is_empty()
        && url.password().is_none()
        && url.query().is_none()
        && url.fragment().is_none())
    .then(|| url.as_str().trim_end_matches('/').to_string())
}
#[cfg(not(feature = "local-qa"))]
fn qa_feed() -> Option<String> {
    None
}
fn release_api_url() -> String {
    match qa_feed() {
        Some(feed) => format!("{feed}/releases"),
        // The legacy latest endpoint stays pinned to the identity bridge forever.
        // Bound the new stable feed; missing older entries fail closed, never downgrade.
        None => format!("https://api.github.com/repos/{PUBLIC_RELEASE_REPO}/releases?per_page=100"),
    }
}
fn download_base_url(version: &str) -> Result<String, String> {
    let parsed = Version::parse(version)
        .filter(|v| version == format!("{}.{}.{}", v.0, v.1, v.2))
        .ok_or_else(|| "Update download version is not canonical stable semantic versioning.".to_string())?;
    let tag = format!("v{}.{}.{}", parsed.0, parsed.1, parsed.2);
    Ok(match qa_feed() {
        Some(feed) => format!("{feed}/download"),
        None => format!("https://github.com/{PUBLIC_RELEASE_REPO}/releases/download/{tag}"),
    })
}
#[cfg(feature = "ultravox-pro")]
const UPDATE_SERVICE_ORIGIN: &str = "https://software.paretocybernetics.com";
#[cfg(feature = "ultravox-pro")]
const UPDATE_SERVICE_PATH_PREFIX: &str = "/api/v1/ultravox/download/";
const PAYLOAD_NAME: &str = "UltraVox-macos-arm64";
const EXPECTED_BUNDLE_ID: &str = "com.imploselabs.ultravox";
/// Temporary Pareto-ID releases (including the recovery bridge) are migrated
/// back; the original identity is permanent and preserves its existing grants.
#[cfg(target_os = "macos")]
const LEGACY_BUNDLE_ID: &str = "com.paretocybernetics.ultravox";

/// After recovery, only the permanent original production identity qualifies.
fn accepted_bundle_id(identifier: &str) -> bool {
    identifier == EXPECTED_BUNDLE_ID
}
const EXPECTED_TEAM_ID: &str = "T63VT9UAY2";
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(300);
static INSTALL_LOCK: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();

/// Update contract v1 (docs/updates.md): the automatic release check is one
/// anonymous GET with this fixed User-Agent, no version, id or machine data.
const CHECK_USER_AGENT: &str = "ultravox-update-check";
/// The check never blocks: a short timeout, then the cause is recorded.
const CHECK_TIMEOUT: Duration = Duration::from_secs(4);
/// At most one automatic check per machine per 24 hours.
const CHECK_INTERVAL_SECS: u64 = 24 * 60 * 60;
/// `ULTRAVOX_NO_UPDATE_CHECK=1` turns every update request off.
pub const NO_UPDATE_CHECK_ENV: &str = "ULTRAVOX_NO_UPDATE_CHECK";
/// Agent and automation hosts that mark the processes they start. UltraVox
/// started from one of them never checks for updates on its own.
const AGENT_ENV_MARKERS: &[&str] = &["CLAUDECODE", "CODEX_SANDBOX", "GEMINI_CLI"];
/// Last network check in this process, so an unwritable cache file can never
/// turn the hourly tick into hourly requests.
static LAST_NETWORK_CHECK: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Preferences are written only when the user changes a toggle, so a missing
/// file or field means "never chose". Update contract v1 (2026-11-26): the
/// daily check is on and automatic installation is off until the user turns it
/// on. Saved choices are kept.
#[derive(Debug, Clone, Deserialize, Serialize, Eq, PartialEq)]
pub struct UpdatePreferences {
    /// Install updates automatically (when idle). Opt-in.
    #[serde(default = "install_automatically_by_default")]
    pub automatic: bool,
    /// Gates the automatic daily release check itself, on every channel.
    #[serde(default = "check_automatically_by_default")]
    pub check_automatically: bool,
    /// **Skip This Version**: no notice for exactly this version.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skipped_version: Option<String>,
    /// The version the update notice last showed, and when (Unix seconds).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_notice_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_notice_at: Option<u64>,
}

/// The two Settings switches. Saving them never touches the skip or notice
/// state; a switch the caller leaves out keeps its stored value.
#[derive(Debug, Default, Deserialize)]
pub struct UpdatePreferencesPatch {
    #[serde(default)]
    pub automatic: Option<bool>,
    #[serde(default)]
    pub check_automatically: Option<bool>,
}

impl UpdatePreferences {
    fn apply(mut self, patch: &UpdatePreferencesPatch) -> Self {
        if let Some(automatic) = patch.automatic {
            self.automatic = automatic;
        }
        if let Some(check_automatically) = patch.check_automatically {
            self.check_automatically = check_automatically;
        }
        self
    }
}

/// What the update notice should do for a candidate version.
#[derive(Debug, Clone, Copy, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum UpdateNotice {
    /// Show the notice (and remember that it was shown).
    Show,
    /// Shown for this version within the last 24 hours: stay quiet.
    Recent,
    /// The user skipped exactly this version.
    Skipped,
}

/// A notice is due at most once per version per day; a skipped version stays
/// quiet until a different (newer) release is offered. A timestamp in the
/// future (clock moved back) counts as expired so notices never stall.
fn notice_for(preferences: &UpdatePreferences, version: &str, now: u64) -> UpdateNotice {
    if preferences.skipped_version.as_deref() == Some(version) {
        return UpdateNotice::Skipped;
    }
    match (preferences.last_notice_version.as_deref(), preferences.last_notice_at) {
        (Some(shown), Some(at)) if shown == version && now >= at && now - at < CHECK_INTERVAL_SECS => {
            UpdateNotice::Recent
        }
        _ => UpdateNotice::Show,
    }
}

fn install_automatically_by_default() -> bool {
    false
}

fn check_automatically_by_default() -> bool {
    true
}

impl Default for UpdatePreferences {
    fn default() -> Self {
        Self {
            automatic: install_automatically_by_default(),
            check_automatically: check_automatically_by_default(),
            skipped_version: None,
            last_notice_version: None,
            last_notice_at: None,
        }
    }
}

/// Who asked for a release check. Only `Automatic` is rate limited and
/// governed by **Check for updates automatically**; `Manual` is the user's
/// **Check Now**.
#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum CheckTrigger {
    #[default]
    Automatic,
    Manual,
}

/// Last check, kept in the platform cache directory (0600, atomic writes).
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct UpdateCheckRecord {
    /// Unix seconds of the last network check.
    #[serde(default)]
    pub last_checked_at: Option<u64>,
    /// The public update found by that check, if any.
    #[serde(default)]
    pub info: Option<UpdateInfo>,
    /// Why that check failed, if it did.
    #[serde(default)]
    pub error: Option<String>,
}

/// What Settings → Updates shows about the checker.
#[derive(Debug, Clone, Serialize)]
pub struct UpdateCheckStatus {
    /// Why update requests are off for this process (environment), if they are.
    pub blocked_reason: Option<String>,
    pub last_checked_at: Option<u64>,
    pub last_error: Option<String>,
}

#[derive(Debug, Eq, PartialEq)]
enum CheckDecision {
    /// Make the network request now.
    Network,
    /// Answer from the last check; the daily interval has not passed.
    Cached,
    /// No request: checks are off, with the reason.
    Off(String),
}

fn decide_check(
    trigger: CheckTrigger,
    check_automatically: bool,
    blocked_reason: Option<&str>,
    last_checked_at: Option<u64>,
    now: u64,
) -> CheckDecision {
    if let Some(reason) = blocked_reason {
        return CheckDecision::Off(reason.to_string());
    }
    match trigger {
        CheckTrigger::Manual => CheckDecision::Network,
        CheckTrigger::Automatic if !check_automatically => {
            CheckDecision::Off("Automatic update checks are turned off in Settings.".to_string())
        }
        CheckTrigger::Automatic => match last_checked_at {
            Some(at) if now >= at && now - at < CHECK_INTERVAL_SECS => CheckDecision::Cached,
            _ => CheckDecision::Network,
        },
    }
}

fn env_flag_set(value: Option<std::ffi::OsString>) -> bool {
    value.is_some_and(|value| {
        let value = value.to_string_lossy();
        let value = value.trim();
        !value.is_empty() && value != "0" && !value.eq_ignore_ascii_case("false")
    })
}

/// Environment reasons that turn update requests off: the explicit opt-out,
/// CI, an agent host, or a development mirror build. The private local QA feed
/// (127.0.0.1 only) is not internet egress and stays usable.
fn environment_block_from(
    var: impl Fn(&str) -> Option<std::ffi::OsString>,
    mirror_build: bool,
) -> Option<String> {
    if env_flag_set(var(NO_UPDATE_CHECK_ENV)) {
        return Some(format!("Update checks are turned off by {NO_UPDATE_CHECK_ENV}."));
    }
    if env_flag_set(var("CI")) {
        return Some("Update checks are off in CI.".to_string());
    }
    if let Some(marker) = AGENT_ENV_MARKERS.iter().find(|name| env_flag_set(var(name))) {
        return Some(format!("Update checks are off under an agent host ({marker})."));
    }
    if mirror_build {
        return Some("Update checks are off in development mirror builds.".to_string());
    }
    None
}

pub fn environment_block() -> Option<String> {
    if qa_feed().is_some() {
        return None;
    }
    environment_block_from(|name| std::env::var_os(name), cfg!(feature = "mirror-debug"))
}

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or_default()
}

/// `<platform cache dir>/update-check.json`; inside `ULTRAVOX_DATA_DIR` when
/// that override is set, so isolated profiles never share a production cache.
fn check_record_path(app: &AppHandle) -> Result<PathBuf, String> {
    use tauri::Manager;
    if std::env::var_os("ULTRAVOX_DATA_DIR").is_some() || cfg!(feature = "mirror-debug") {
        return Ok(crate::state::data_dir(app)?.join("update-check.json"));
    }
    Ok(app
        .path()
        .app_cache_dir()
        .map_err(|error| error.to_string())?
        .join("update-check.json"))
}

fn read_check_record(path: &Path) -> UpdateCheckRecord {
    fs::read(path)
        .ok()
        .and_then(|raw| serde_json::from_slice(&raw).ok())
        .unwrap_or_default()
}

fn write_private_json(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| format!("{} has no parent directory.", path.display()))?;
    fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    ultravox_core::private_fs::protect_dir(parent).map_err(|error| error.to_string())?;
    let temporary = path.with_extension("json.tmp");
    let _ = fs::remove_file(&temporary);
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    {
        use std::io::Write;
        let mut file = options.open(&temporary).map_err(|error| error.to_string())?;
        file.write_all(bytes).map_err(|error| error.to_string())?;
        file.sync_all().map_err(|error| error.to_string())?;
    }
    ultravox_core::private_fs::protect_file(&temporary).map_err(|error| error.to_string())?;
    fs::rename(&temporary, path).map_err(|error| error.to_string())?;
    ultravox_core::private_fs::protect_file(path).map_err(|error| error.to_string())
}

fn cached_update(record: &UpdateCheckRecord, current_version: &str) -> Option<UpdateInfo> {
    let current = Version::parse(current_version)?;
    record.info.clone().filter(|info| {
        info.channel == UpdateChannel::Public
            && Version::parse(&info.latest_version).is_some_and(|latest| latest > current)
    })
}

pub fn check_status(app: &AppHandle) -> Result<UpdateCheckStatus, String> {
    let record = read_check_record(&check_record_path(app)?);
    Ok(UpdateCheckStatus {
        blocked_reason: environment_block(),
        last_checked_at: record.last_checked_at,
        last_error: record.error,
    })
}

/// The only entry point the app uses to look for updates. Every request,
/// public or the licensed Pro service channel, passes this gate:
/// environment opt-outs (env, CI, agent hosts, mirror builds) stop all of
/// them; the Settings switch and the 24-hour interval govern automatic checks.
/// Automatic checks are anonymous and use only the public release feed; the
/// authenticated Pro channel is consulted only when the user clicks Check Now.
pub async fn gated_check(
    app: &AppHandle,
    current_version: &str,
    trigger: CheckTrigger,
) -> Result<Option<UpdateInfo>, String> {
    // An unreadable preference file may hold an opt-out: fail closed.
    let check_automatically = read_preferences(app)
        .map(|preferences| preferences.check_automatically)
        .unwrap_or(false);
    let path = check_record_path(app)?;
    let record = read_check_record(&path);
    let blocked = environment_block();
    let in_process = LAST_NETWORK_CHECK.load(std::sync::atomic::Ordering::SeqCst);
    let last_checked_at = record
        .last_checked_at
        .max((in_process > 0).then_some(in_process));
    match decide_check(
        trigger,
        check_automatically,
        blocked.as_deref(),
        last_checked_at,
        unix_now(),
    ) {
        CheckDecision::Off(reason) => {
            if trigger == CheckTrigger::Manual {
                Err(reason)
            } else {
                Ok(None)
            }
        }
        CheckDecision::Cached => Ok(cached_update(&record, current_version)),
        CheckDecision::Network => {
            LAST_NETWORK_CHECK.store(unix_now(), std::sync::atomic::Ordering::SeqCst);
            let result = if may_use_licensed_channel(trigger) {
                check(current_version).await
            } else {
                public_check(current_version).await
            };
            let next = UpdateCheckRecord {
                last_checked_at: Some(unix_now()),
                info: result
                    .as_ref()
                    .ok()
                    .cloned()
                    .flatten()
                    .filter(|info| info.channel == UpdateChannel::Public),
                error: result.as_ref().err().cloned(),
            };
            match serde_json::to_vec(&next) {
                Ok(bytes) => {
                    if let Err(error) = write_private_json(&path, &bytes) {
                        eprintln!("Could not record the update check: {error}");
                    }
                }
                Err(error) => eprintln!("Could not record the update check: {error}"),
            }
            result
        }
    }
}

/// Automatic checks are one anonymous request to the public feed. Only the
/// user's **Check Now** may also ask the licensed service, whose request is
/// signed with the install's credentials and names the running version.
fn may_use_licensed_channel(trigger: CheckTrigger) -> bool {
    trigger == CheckTrigger::Manual
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum UpdateChannel {
    /// Public GitHub release assets; the default channel for every build.
    #[default]
    Public,
    /// Authenticated Pareto distribution channel, used as a fallback by
    /// official builds that hold a stored access key.
    Service,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct UpdateInfo {
    pub current_version: String,
    pub latest_version: String,
    pub release_url: String,
    #[serde(default)]
    pub channel: UpdateChannel,
    /// Authenticated download URL for the service channel (absent on public).
    #[serde(default)]
    pub service_download_url: Option<String>,
    /// Expected SHA-256 of the service-channel update artifact.
    #[serde(default)]
    pub service_sha256: Option<String>,
    /// Short plain-text release notes for the update notice.
    #[serde(default)]
    pub release_notes: String,
}

const RELEASE_NOTES_MAX_CHARS: usize = 600;
const NO_RELEASE_NOTES: &str = "No release notes were published for this version.";

/// Release notes as short plain text: blank lines and Markdown heading,
/// emphasis and code markers are dropped so the notice reads cleanly.
fn plain_release_notes(body: &str) -> String {
    let plain = body
        .lines()
        .map(|line| line.trim_start_matches('#').trim().replace("**", "").replace('`', ""))
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    let plain = plain.trim();
    if plain.is_empty() {
        return NO_RELEASE_NOTES.to_string();
    }
    if plain.chars().count() <= RELEASE_NOTES_MAX_CHARS {
        return plain.to_string();
    }
    let clipped: String = plain.chars().take(RELEASE_NOTES_MAX_CHARS).collect();
    format!("{}…", clipped.trim_end())
}

/// Platform tag understood by the Pareto distribution service; also used in
/// release asset names.
pub(crate) fn distribution_platform() -> &'static str {
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    return "macos-arm64";
    #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
    return "macos-x86_64";
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    return "linux-x86_64";
    #[cfg(all(target_os = "linux", target_arch = "aarch64"))]
    return "linux-aarch64";
    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    return "windows-x86_64";
}

/// Immutable release asset names for this build's platform. Every build uses
/// the single `UltraVox-<platform>` family. deb/rpm ship as release artifacts
/// but are updated by the system package manager.
fn asset_family() -> &'static str {
    "UltraVox"
}

fn platform_assets() -> (String, String) {
    let prefix = asset_family();
    match distribution_platform() {
        "macos-arm64" => (
            format!("{prefix}-macos-arm64.zip"),
            format!("{prefix}-macos-arm64.zip.sha256"),
        ),
        "macos-x86_64" => (
            format!("{prefix}-macos-x86_64.zip"),
            format!("{prefix}-macos-x86_64.zip.sha256"),
        ),
        "linux-x86_64" => (
            format!("{prefix}-linux-x86_64.AppImage"),
            format!("{prefix}-linux-x86_64.AppImage.sha256"),
        ),
        "linux-aarch64" => (
            format!("{prefix}-linux-aarch64.AppImage"),
            format!("{prefix}-linux-aarch64.AppImage.sha256"),
        ),
        "windows-x86_64" => (
            format!("{prefix}-windows-x86_64-setup.exe"),
            format!("{prefix}-windows-x86_64-setup.exe.sha256"),
        ),
        other => unreachable!("unsupported distribution platform: {other}"),
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct Version(u64, u64, u64);

impl Version {
    fn parse(text: &str) -> Option<Self> {
        let trimmed = text.trim().trim_start_matches('v');
        let mut parts = trimmed.split('.');
        let major = parts.next()?.parse().ok()?;
        let minor = parts.next()?.parse().ok()?;
        let patch = parts.next()?.parse().ok()?;
        if parts.next().is_some() {
            return None;
        }
        Some(Self(major, minor, patch))
    }
}

fn preferences_path(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(crate::state::data_dir(app)?.join("update-preferences.json"))
}

pub fn read_preferences(app: &AppHandle) -> Result<UpdatePreferences, String> {
    let path = preferences_path(app)?;
    match fs::read(path) {
        Ok(raw) => serde_json::from_slice(&raw).map_err(|error| error.to_string()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            Ok(UpdatePreferences::default())
        }
        Err(error) => Err(error.to_string()),
    }
}

pub fn write_preferences(app: &AppHandle, preferences: &UpdatePreferences) -> Result<(), String> {
    let path = preferences_path(app)?;
    write_private_json(
        &path,
        &serde_json::to_vec(preferences).map_err(|error| error.to_string())?,
    )
}

/// Settings switches. An unreadable file starts from the defaults (automatic
/// install off); the switches the user sets are what the panel shows.
pub fn patch_preferences(
    app: &AppHandle,
    patch: &UpdatePreferencesPatch,
) -> Result<UpdatePreferences, String> {
    let next = read_preferences(app).unwrap_or_default().apply(patch);
    write_preferences(app, &next)?;
    Ok(next)
}

/// **Skip This Version**.
pub fn skip_version(app: &AppHandle, version: &str) -> Result<(), String> {
    if Version::parse(version).is_none() {
        return Err("Only a stable release version can be skipped.".to_string());
    }
    let mut preferences = read_preferences(app)?;
    preferences.skipped_version = Some(version.to_string());
    write_preferences(app, &preferences)
}

/// Decides whether the notice for `version` shows now, and records it when it
/// does, so it appears at most once a day per version.
pub fn claim_notice(app: &AppHandle, version: &str) -> Result<UpdateNotice, String> {
    let mut preferences = read_preferences(app)?;
    let now = unix_now();
    let notice = notice_for(&preferences, version, now);
    if notice == UpdateNotice::Show {
        preferences.last_notice_version = Some(version.to_string());
        preferences.last_notice_at = Some(now);
        write_preferences(app, &preferences)?;
    }
    Ok(notice)
}

fn http_client(timeout: Duration) -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .user_agent(concat!("ultravox-updater/", env!("CARGO_PKG_VERSION")))
        .timeout(timeout)
        .build()
        .map_err(|error| format!("Failed to create update client: {error}"))
}

/// Updates for every build come from the public GitHub release of the
/// canonical repository. Official builds that hold a stored access key may
/// fall back to the authenticated channel when the public release has no
/// update or is temporarily unreachable. Call through [`gated_check`], which
/// only reaches this on the user's Check Now.
async fn check(current_version: &str) -> Result<Option<UpdateInfo>, String> {
    let public = public_check(current_version).await;
    #[cfg(feature = "ultravox-pro")]
    {
        if !matches!(&public, Ok(Some(_))) && crate::pro_api::distribution::credentials_available()
        {
            return service_check(current_version).await;
        }
    }
    public
}

/// One anonymous GET: fixed User-Agent, no version, id or machine data.
fn check_client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .user_agent(CHECK_USER_AGENT)
        .timeout(CHECK_TIMEOUT)
        .build()
        .map_err(|error| format!("Failed to create update client: {error}"))
}

async fn public_check(current_version: &str) -> Result<Option<UpdateInfo>, String> {
    public_check_from(&release_api_url(), current_version).await
}

async fn public_check_from(url: &str, current_version: &str) -> Result<Option<UpdateInfo>, String> {
    let response = check_client()?
        .get(url)
        .header(reqwest::header::ACCEPT, "application/vnd.github+json")
        .send()
        .await
        .map_err(|error| format!("Failed to reach GitHub releases: {error}"))?
        .error_for_status()
        .map_err(|error| format!("GitHub release check failed: {error}"))?;
    let metadata: serde_json::Value = response
        .json()
        .await
        .map_err(|error| format!("Failed to read GitHub release metadata: {error}"))?;
    public_update_info(&metadata, current_version)
}

/// New stable feed deliberately ignores legacy GitHub "latest". Only final
/// original-ID releases (>=0.13.4) with this platform's immutable assets qualify.
fn select_public_release(metadata: &serde_json::Value) -> Result<Option<&serde_json::Value>, String> {
    let releases = metadata.as_array()
        .ok_or_else(|| "Stable update feed must be a release list.".to_string())?;
    let (artifact, checksum) = platform_assets();
    Ok(releases.iter().filter_map(|release| {
        if release["draft"].as_bool() != Some(false) || release["prerelease"].as_bool() != Some(false) {
            return None;
        }
        let tag = release["tag_name"].as_str()?;
        let version = Version::parse(tag)?;
        if version < Version(0, 13, 4) || tag != format!("v{}.{}.{}", version.0, version.1, version.2) {
            return None;
        }
        let assets = release["assets"].as_array()?;
        if ![&artifact, &checksum].iter().all(|required| assets.iter().any(|asset| asset["name"].as_str() == Some(required.as_str()))) {
            return None;
        }
        Some((version, release))
    }).max_by_key(|(version, _)| *version).map(|(_, release)| release))
}

fn public_update_info(metadata: &serde_json::Value, current_version: &str) -> Result<Option<UpdateInfo>, String> {
    let current = Version::parse(current_version)
        .ok_or_else(|| "Current app version is not stable semantic versioning.".to_string())?;
    let Some(release) = select_public_release(metadata)? else { return Ok(None); };
    let tag = release["tag_name"]
        .as_str()
        .ok_or_else(|| "Release metadata is missing tag_name.".to_string())?;
    let latest_version = tag.trim().trim_start_matches('v').to_string();
    let latest = Version::parse(&latest_version)
        .ok_or_else(|| "Latest release is not stable semantic versioning.".to_string())?;
    if latest <= current { return Ok(None); }
    Ok(Some(UpdateInfo {
        current_version: current_version.to_string(),
        latest_version,
        release_url: format!("https://github.com/{PUBLIC_RELEASE_REPO}/releases/tag/{tag}"),
        channel: UpdateChannel::Public,
        service_download_url: None,
        service_sha256: None,
        release_notes: plain_release_notes(release["body"].as_str().unwrap_or_default()),
    }))
}

/// Authenticated manifest from the Pareto distribution service (official
/// builds with a stored access key only).
#[cfg(feature = "ultravox-pro")]
async fn service_check(current_version: &str) -> Result<Option<UpdateInfo>, String> {
    let credentials = crate::pro_api::distribution::credentials()?;
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct ServiceManifest {
        current_version: String,
        latest_version: String,
        update_available: bool,
        release_url: String,
        download_url: Option<String>,
        sha256: Option<String>,
        #[serde(default)]
        release_notes: Option<String>,
    }

    let path_and_query = format!("/api/v1/ultravox/update?current={current_version}");
    let request = http_client(CHECK_TIMEOUT)?
        .get(format!("{UPDATE_SERVICE_ORIGIN}{path_and_query}"))
        .query(&[("current", current_version)])
        .header(reqwest::header::ACCEPT, "application/json");
    let signed = crate::pro_api::distribution::authorize_request(
        request,
        "GET",
        &path_and_query,
        &credentials.device,
        Some(&credentials.access_key),
    )?;
    let response = signed
        .send()
        .await
        .map_err(|error| format!("Failed to reach the private update service: {error}"))?;
    if response.status() == reqwest::StatusCode::UNAUTHORIZED
        || response.status() == reqwest::StatusCode::FORBIDDEN
    {
        // Revoked/expired access degrades to the public channel instead of
        // blocking updates for features a license no longer covers.
        return Ok(None);
    }
    let payload = response
        .json::<ServiceManifest>()
        .await
        .map_err(|error| format!("Failed to read private release metadata: {error}"))?;
    if payload.current_version != current_version {
        return Err("Private update metadata did not echo the running version.".to_string());
    }
    let current = Version::parse(current_version)
        .ok_or_else(|| "Current app version is not stable semantic versioning.".to_string())?;
    let latest = Version::parse(&payload.latest_version)
        .ok_or_else(|| "Latest release is not stable semantic versioning.".to_string())?;
    if payload.update_available != (latest > current) {
        return Err("Private update metadata returned an inconsistent decision.".to_string());
    }
    if !payload.update_available {
        return Ok(None);
    }
    let service_download_url = payload
        .download_url
        .clone()
        .filter(|url| {
            reqwest::Url::parse(url).is_ok_and(|parsed| {
                parsed.scheme() == "https"
                    && parsed.host_str() == Some("software.paretocybernetics.com")
                    && parsed.path().starts_with(UPDATE_SERVICE_PATH_PREFIX)
            })
        })
        .ok_or_else(|| "Private update metadata is missing a trusted download URL.".to_string())?;
    Ok(Some(UpdateInfo {
        current_version: current_version.to_string(),
        latest_version: payload.latest_version,
        release_url: payload.release_url,
        channel: UpdateChannel::Service,
        service_download_url: Some(service_download_url),
        service_sha256: payload.sha256,
        release_notes: plain_release_notes(payload.release_notes.as_deref().unwrap_or_default()),
    }))
}

fn current_bundle() -> Result<PathBuf, String> {
    let executable = std::env::current_exe()
        .map_err(|error| format!("Failed to resolve the UltraVox executable: {error}"))?;
    let bundle = executable
        .ancestors()
        .find(|path| path.extension().is_some_and(|extension| extension == "app"))
        .map(Path::to_path_buf)
        .ok_or_else(|| "Updates require an installed UltraVox.app bundle.".to_string())?;
    if bundle.file_name().and_then(|name| name.to_str()) != Some("UltraVox.app") {
        return Err("The running application is not the stable UltraVox.app bundle.".to_string());
    }
    Ok(bundle)
}

async fn download(url: &str, target: &Path) -> Result<(), String> {
    let bytes = http_client(DOWNLOAD_TIMEOUT)?
        .get(url)
        .send()
        .await
        .map_err(|error| format!("Download failed for {url}: {error}"))?
        .error_for_status()
        .map_err(|error| format!("Download failed for {url}: {error}"))?
        .bytes()
        .await
        .map_err(|error| format!("Failed to read {url}: {error}"))?;
    fs::write(target, &bytes)
        .map_err(|error| format!("Failed to write {}: {error}", target.display()))
}

fn run_checked(program: &str, args: &[&str], action: &str) -> Result<(), String> {
    let output = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .output()
        .map_err(|error| format!("Failed to {action}: {error}"))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(format_command_failure(action, &output))
    }
}

/// Format a failed command with its exit code and a bounded tail of its
/// output, so update failures are diagnosable from the error alone.
fn format_command_failure(action: &str, output: &std::process::Output) -> String {
    let code = output
        .status
        .code()
        .map(|code| code.to_string())
        .unwrap_or_else(|| "terminated by signal".to_string());
    let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&output.stderr));
    let trimmed = text.trim();
    const MAX_TAIL: usize = 300;
    let tail = if trimmed.len() > MAX_TAIL {
        &trimmed[trimmed.len() - MAX_TAIL..]
    } else {
        trimmed
    };
    if tail.is_empty() {
        format!("Failed to {action} (exit {code}).")
    } else {
        format!("Failed to {action} (exit {code}): {tail}")
    }
}

fn command_output(program: &str, args: &[&str], action: &str) -> Result<String, String> {
    let output = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .output()
        .map_err(|error| format!("Failed to {action}: {error}"))?;
    let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&output.stderr));
    if output.status.success() {
        Ok(text)
    } else {
        Err(format_command_failure(action, &output))
    }
}

#[derive(Debug, Default, Eq, PartialEq)]
struct SignatureIdentity {
    identifier: Option<String>,
    team_identifier: Option<String>,
    developer_id: bool,
    ad_hoc: bool,
}

fn parse_signature_details(details: &str) -> SignatureIdentity {
    let mut identity = SignatureIdentity::default();
    for line in details.lines() {
        if let Some(value) = line.strip_prefix("Identifier=") {
            identity.identifier = Some(value.trim().to_string());
        } else if let Some(value) = line.strip_prefix("TeamIdentifier=") {
            identity.team_identifier = Some(value.trim().to_string());
        } else if line.starts_with("Authority=Developer ID Application:") {
            identity.developer_id = true;
        }
        if line.contains("(adhoc)") || line.contains("Signature=adhoc") {
            identity.ad_hoc = true;
        }
    }
    identity
}

fn validate_signature(details: &str, requirements: &str) -> Result<(), String> {
    let identity = parse_signature_details(details);
    let Some(bundle_id) = identity.identifier.as_deref().filter(|id| accepted_bundle_id(id)) else {
        return Err("Update bundle identifier does not match UltraVox.".to_string());
    };
    if identity.team_identifier.as_deref() != Some(EXPECTED_TEAM_ID) {
        return Err("Update Developer Team does not match UltraVox.".to_string());
    }
    if identity.ad_hoc || !identity.developer_id {
        return Err("Update must use a non-ad-hoc Developer ID signature.".to_string());
    }
    let Some(designated) = requirements
        .lines()
        .map(str::trim)
        .find(|line| line.starts_with("designated =>"))
    else {
        return Err("Update designated requirement does not match UltraVox.".to_string());
    };
    // The designated requirement must name the same identifier the signature carries.
    let identifier_quoted = format!("designated => identifier \"{bundle_id}\"");
    let identifier_unquoted = format!("designated => identifier {bundle_id}");
    let team_quoted = format!("certificate leaf[subject.OU] = \"{EXPECTED_TEAM_ID}\"");
    let team_unquoted = format!("certificate leaf[subject.OU] = {EXPECTED_TEAM_ID}");
    let identifier_matches = designated
        .split(" and ")
        .next()
        .is_some_and(|clause| clause == identifier_quoted || clause == identifier_unquoted);
    let anchor_matches = designated
        .split(" and ")
        .any(|clause| clause == "anchor apple generic");
    let team_matches = designated
        .split(" and ")
        .any(|clause| clause == team_quoted || clause == team_unquoted);
    if !identifier_matches || !anchor_matches || !team_matches {
        return Err("Update designated requirement does not match UltraVox.".to_string());
    }
    Ok(())
}

fn validate_bundle_name(app: &Path) -> Result<(), String> {
    if app.file_name().and_then(|name| name.to_str()) == Some("UltraVox.app") {
        Ok(())
    } else {
        Err("Update archive contained an unexpected app bundle name.".to_string())
    }
}

fn verify_bundle_metadata(app: &Path, expected_version: &str) -> Result<(), String> {
    if !app.is_dir() {
        return Err(format!("Expected UltraVox.app at {}.", app.display()));
    }
    validate_bundle_name(app)?;
    let info = app.join("Contents/Info.plist");
    let info_arg = info.to_string_lossy().into_owned();
    let identifier = command_output(
        "/usr/libexec/PlistBuddy",
        &["-c", "Print :CFBundleIdentifier", info_arg.as_str()],
        "inspect the update bundle identifier",
    )?;
    if !accepted_bundle_id(identifier.trim()) {
        return Err("Update Info.plist bundle identifier does not match UltraVox.".to_string());
    }
    let version = command_output(
        "/usr/libexec/PlistBuddy",
        &["-c", "Print :CFBundleShortVersionString", info_arg.as_str()],
        "inspect the update version",
    )?;
    if version.trim() != expected_version {
        return Err(format!(
            "Update version {} does not match expected {expected_version}.",
            version.trim()
        ));
    }
    Ok(())
}

fn verify_app(
    app: &Path,
    expected_version: &str,
    require_notarization: bool,
) -> Result<(), String> {
    verify_bundle_metadata(app, expected_version)?;
    let app_arg = app.to_string_lossy().into_owned();
    run_checked(
        "/usr/bin/codesign",
        &["--verify", "--deep", "--strict", app_arg.as_str()],
        "verify the update's sealed resources",
    )?;
    let details = command_output(
        "/usr/bin/codesign",
        &["-dv", "--verbose=4", app_arg.as_str()],
        "inspect the update signature",
    )?;
    let requirements = command_output(
        "/usr/bin/codesign",
        &["-d", "-r-", app_arg.as_str()],
        "inspect the update designated requirement",
    )?;
    validate_signature(&details, &requirements)?;
    let plist_id = command_output(
        "/usr/libexec/PlistBuddy",
        &["-c", "Print :CFBundleIdentifier", app.join("Contents/Info.plist").to_string_lossy().as_ref()],
        "compare the update metadata and signature identity",
    )?;
    if parse_signature_details(&details).identifier.as_deref() != Some(plist_id.trim()) {
        return Err("Update metadata and signature identifiers disagree.".to_string());
    }
    if require_notarization {
        verify_notarization(app)?;
    }
    Ok(())
}

/// Verify the update is notarized. `spctl --assess` is tried first with
/// retries because syspolicyd intermittently errors on freshly unpacked
/// bundles. On machines where Gatekeeper assessment is disabled or
/// unreachable, `spctl` fails persistently even for valid notarized apps, so
/// fall back to validating the stapled notarization ticket — Apple's own
/// offline proof embedded in the bundle by the release pipeline.
fn verify_notarization(app: &Path) -> Result<(), String> {
    let app_arg = app.to_string_lossy().into_owned();
    let mut last_error = String::new();
    for attempt in 0..4 {
        if attempt > 0 {
            std::thread::sleep(Duration::from_millis(1500));
        }
        match command_output(
            "/usr/sbin/spctl",
            &["--assess", "--type", "execute", app_arg.as_str()],
            "assess the update's notarization",
        ) {
            Ok(_) => return Ok(()),
            Err(error) => last_error = error,
        }
    }
    if let Ok(status) = command_output("/usr/sbin/spctl", &["--status"], "check Gatekeeper status")
    {
        if status.contains("disabled") {
            // Gatekeeper assessment is disabled system-wide, so macOS would
            // never enforce notarization on launch either. The codesign and
            // designated-requirement checks above still bind the update to the
            // UltraVox Developer ID identity.
            return Ok(());
        }
    }
    if Command::new("/usr/bin/xcrun")
        .args(["stapler", "validate", app_arg.as_str()])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
    {
        return Ok(());
    }
    Err(format!(
        "Failed to verify the update's notarization. {last_error} No valid stapled ticket found either."
    ))
}
fn verify_checksum(archive: &Path, checksum: &Path) -> Result<(), String> {
    let expected = fs::read_to_string(checksum)
        .map_err(|error| format!("Failed to read update checksum: {error}"))?
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    if expected.len() != 64 || !expected.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("Update checksum file is invalid.".to_string());
    }
    let archive_arg = archive.to_string_lossy().into_owned();
    let output = command_output(
        "/usr/bin/shasum",
        &["-a", "256", archive_arg.as_str()],
        "calculate the update checksum",
    )?;
    let actual = output
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    if actual != expected {
        return Err("Update archive failed its SHA-256 checksum.".to_string());
    }
    Ok(())
}

const INSTALL_HELPER: &str = r#"
set -u
pid="$1"
source_app="$2"
target_app="$3"
backup_app="$4"
staging="$5"
required_identifier="${6:-}"
bundle_id=""
team_id="T63VT9UAY2"
verify_notarized() {
  attempt=0
  while [ "$attempt" -lt 4 ]; do
    if /usr/sbin/spctl --assess --type execute "$1" >/dev/null 2>&1; then return 0; fi
    attempt=$((attempt + 1))
    [ "$attempt" -lt 4 ] && /bin/sleep 1.5
  done
  # Gatekeeper assessment can be disabled or unreachable on some machines even
  # for valid notarized apps. When assessment is disabled system-wide, macOS
  # would not enforce notarization on launch either, so the identity checks
  # above are sufficient.
  if /usr/sbin/spctl --status 2>/dev/null | /usr/bin/grep -q "disabled"; then return 0; fi
  # A stapled ticket is Apple's offline notarization proof.
  /usr/bin/xcrun stapler validate "$1" >/dev/null 2>&1
}
verify_identity() {
  /usr/bin/codesign --verify --deep --strict "$1" >/dev/null 2>&1 || return 1
  details="$(/usr/bin/codesign -dv --verbose=4 "$1" 2>&1)" || return 1
  bundle_id=""
  for candidate in com.imploselabs.ultravox; do
    if printf '%s\n' "$details" | /usr/bin/grep -Fxq "Identifier=${candidate}"; then bundle_id="$candidate"; fi
  done
  [ -n "$bundle_id" ] || return 1
  [ -z "$required_identifier" ] || [ "$bundle_id" = "$required_identifier" ] || return 1
  printf '%s\n' "$details" | /usr/bin/grep -Fxq "TeamIdentifier=${team_id}" || return 1
  printf '%s\n' "$details" | /usr/bin/grep -Eq "^Authority=Developer ID Application: .+ \(${team_id}\)$" || return 1
  case "$details" in *'(adhoc)'*|*'Signature=adhoc'*) return 1 ;; esac
  plist_id="$(/usr/libexec/PlistBuddy -c 'Print :CFBundleIdentifier' "$1/Contents/Info.plist")" || return 1
  [ "$plist_id" = "$bundle_id" ] || return 1
  requirements="$(/usr/bin/codesign -d -r- "$1" 2>&1)" || return 1
  designated="$(printf '%s\n' "$requirements" | /usr/bin/grep '^designated =>')"
  first_clause="${designated%% and *}"
  [ "$first_clause" = "designated => identifier \"${bundle_id}\"" ] ||
    [ "$first_clause" = "designated => identifier ${bundle_id}" ] || return 1
  clauses="$(printf '%s\n' "$designated" | /usr/bin/awk -F ' and ' '{for (i=1; i<=NF; i++) print $i}')"
  printf '%s\n' "$clauses" | /usr/bin/grep -Fxq 'anchor apple generic' || return 1
  printf '%s\n' "$clauses" | /usr/bin/grep -Fxq "certificate leaf[subject.OU] = \"${team_id}\"" ||
    printf '%s\n' "$clauses" | /usr/bin/grep -Fxq "certificate leaf[subject.OU] = ${team_id}" || return 1
  verify_notarized "$1" || return 1
}
while /bin/kill -0 "$pid" 2>/dev/null; do /bin/sleep 0.2; done
if ! verify_identity "$source_app"; then /bin/rm -rf "$staging"; exit 1; fi
/bin/rm -rf "$backup_app"
if ! /bin/mv "$target_app" "$backup_app"; then /bin/rm -rf "$staging"; exit 1; fi
if ! /usr/bin/ditto "$source_app" "$target_app"; then
  /bin/rm -rf "$target_app"
  /bin/mv "$backup_app" "$target_app" || exit 1
  /usr/bin/open -n "$target_app"
  /bin/rm -rf "$staging"
  exit 1
fi
if ! verify_identity "$target_app"; then
  /bin/rm -rf "$target_app"
  /bin/mv "$backup_app" "$target_app" || exit 1
  /usr/bin/open -n "$target_app"
  /bin/rm -rf "$staging"
  exit 1
fi
/usr/bin/xattr -dr com.apple.quarantine "$target_app" 2>/dev/null || true
existing_pids="$(/usr/bin/pgrep -x "ultravox" 2>/dev/null || true)"
if ! /usr/bin/open -n "$target_app"; then
  /bin/rm -rf "$target_app"
  /bin/mv "$backup_app" "$target_app" || exit 1
  /usr/bin/open -n "$target_app" || true
  /bin/rm -rf "$staging"
  exit 1
fi
# LaunchServices can accept an app launch while the process immediately fails.
launched=0
attempt=0
while [ "$attempt" -lt 50 ]; do
  current_pids="$(/usr/bin/pgrep -x "ultravox" 2>/dev/null || true)"
  launched_pid=""
  for pid in $current_pids; do
    is_existing=0
    for old_pid in $existing_pids; do
      if [ "$old_pid" = "$pid" ]; then
        is_existing=1
        break
      fi
    done
    if [ "$is_existing" -eq 0 ] && /bin/kill -0 "$pid" 2>/dev/null; then
      launched_pid="$pid"
      break
    fi
  done
  if [ -n "$launched_pid" ]; then
    healthy=1
    health_attempt=0
    while [ "$health_attempt" -lt 10 ]; do
      if ! /bin/kill -0 "$launched_pid" 2>/dev/null; then
        healthy=0
        break
      fi
      /bin/sleep 0.2
      health_attempt=$((health_attempt + 1))
    done
    if [ "$healthy" -eq 1 ]; then
      launched=1
      break
    fi
  fi
  /bin/sleep 0.2
  attempt=$((attempt + 1))
done
if [ "$launched" -ne 1 ]; then
  /bin/rm -rf "$target_app"
  /bin/mv "$backup_app" "$target_app" || exit 1
  /usr/bin/open -n "$target_app" || true
  /bin/rm -rf "$staging"
  exit 1
fi
/bin/rm -rf "$backup_app" "$staging"
"#;

/// Prefix of the error returned while the app is busy; the frontend retries
/// automatic installs later instead of reporting a failure.
pub const UPDATE_BUSY_PREFIX: &str = "Update waits:";

/// Refuses to replace the app while it holds the user's work.
async fn ensure_idle(app: &AppHandle) -> Result<(), String> {
    use tauri::Manager;
    let Some(state) = app.try_state::<crate::state::AppState>() else {
        return Ok(());
    };
    match state.busy_reason().await {
        Some(reason) => Err(format!("{UPDATE_BUSY_PREFIX} {reason}.")),
        None => Ok(()),
    }
}

pub async fn install(app: AppHandle, info: UpdateInfo) -> Result<(), String> {
    let _install_guard = INSTALL_LOCK
        .get_or_init(|| tokio::sync::Mutex::new(()))
        .try_lock()
        .map_err(|_| "An UltraVox update is already being installed.".to_string())?;
    ensure_idle(&app).await?;
    if info.current_version != env!("CARGO_PKG_VERSION") {
        return Err("Update was checked against a different app version.".to_string());
    }
    let current = Version::parse(&info.current_version)
        .ok_or_else(|| "Current app version is invalid.".to_string())?;
    let latest = Version::parse(&info.latest_version)
        .ok_or_else(|| "Update version is invalid.".to_string())?;
    if latest <= current {
        return Err("Update is not newer than the running app.".to_string());
    }
    match std::env::consts::OS {
        "macos" => install_macos(app, &info).await,
        "linux" => install_linux(app, &info).await,
        "windows" => install_windows(app, &info),
        other => Err(format!(
            "Automatic updates are unavailable on {other}; download releases manually."
        )),
    }
}

/// Authenticated download through the Pareto distribution service (official
/// builds with a stored access key only).
#[cfg(feature = "ultravox-pro")]
async fn download_service(url: &str, target: &Path) -> Result<(), String> {
    let credentials = crate::pro_api::distribution::credentials()?;
    let parsed =
        reqwest::Url::parse(url).map_err(|_| "Service download URL is invalid.".to_string())?;
    let path_and_query = parsed.query().map_or_else(
        || parsed.path().to_string(),
        |query| format!("{}?{query}", parsed.path()),
    );
    let request = http_client(DOWNLOAD_TIMEOUT)?.get(url);
    let signed = crate::pro_api::distribution::authorize_request(
        request,
        "GET",
        &path_and_query,
        &credentials.device,
        Some(&credentials.access_key),
    )?;
    let bytes = signed
        .send()
        .await
        .map_err(|error| format!("Private update download failed: {error}"))?
        .error_for_status()
        .map_err(|error| format!("Private update download was rejected: {error}"))?
        .bytes()
        .await
        .map_err(|error| format!("Private update download failed: {error}"))?;
    fs::write(target, &bytes)
        .map_err(|error| format!("Failed to write {}: {error}", target.display()))
}

/// Temporary download directory. Dropping it deletes the directory, so every
/// exit path (an early `?`, a failed check, a panic) cleans up; `keep` hands
/// it to the macOS install helper, which deletes it on every outcome.
struct Staging {
    dir: PathBuf,
    keep: bool,
}

impl Staging {
    fn create() -> Result<Self, String> {
        let dir = std::env::temp_dir().join(format!("ultravox-update-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&dir)
            .map_err(|error| format!("Failed to create update staging directory: {error}"))?;
        Ok(Self { dir, keep: false })
    }

    fn keep(mut self) {
        self.keep = true;
    }
}

impl Drop for Staging {
    fn drop(&mut self) {
        if !self.keep {
            let _ = fs::remove_dir_all(&self.dir);
        }
    }
}

/// Downloads the platform artifact (public GitHub or authenticated service)
/// plus its immutable checksum into a fresh staging directory, which is
/// removed when the returned guard drops.
async fn fetch_platform_artifact(info: &UpdateInfo) -> Result<(Staging, PathBuf, PathBuf), String> {
    let base = match info.channel {
        UpdateChannel::Public => Some(download_base_url(&info.latest_version)?),
        UpdateChannel::Service => None,
    };
    fetch_platform_artifact_from(info, base.as_deref()).await
}

async fn fetch_platform_artifact_from(
    info: &UpdateInfo,
    public_base: Option<&str>,
) -> Result<(Staging, PathBuf, PathBuf), String> {
    let (asset, checksum) = platform_assets();
    let staging = Staging::create()?;
    let artifact_path = staging.dir.join(&asset);
    let checksum_path = staging.dir.join(&checksum);

    match info.channel {
        UpdateChannel::Public => {
            let base = public_base
                .ok_or_else(|| "Public update download has no release URL.".to_string())?;
            download(&format!("{base}/{asset}"), &artifact_path).await?;
            download(&format!("{base}/{checksum}"), &checksum_path).await?;
        }
        #[cfg(feature = "ultravox-pro")]
        UpdateChannel::Service => {
            let url = info
                .service_download_url
                .clone()
                .ok_or_else(|| "Private update metadata is missing a download URL.".to_string())?;
            let expected = info
                .service_sha256
                .clone()
                .filter(|value| {
                    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
                })
                .ok_or_else(|| "Private update metadata is missing a valid SHA-256.".to_string())?
                .to_ascii_lowercase();
            fs::write(&checksum_path, format!("{expected}  {asset}\n"))
                .map_err(|error| error.to_string())?;
            download_service(&url, &artifact_path).await?;
        }
        #[cfg(not(feature = "ultravox-pro"))]
        UpdateChannel::Service => {
            return Err(
                "The authenticated update channel is not included in this build.".to_string(),
            );
        }
    }
    Ok((staging, artifact_path, checksum_path))
}

// macOS ------------------------------------------------------------------

async fn install_macos(app: AppHandle, info: &UpdateInfo) -> Result<(), String> {
    schedule_macos(Some(&app), info, None).await?;
    app.exit(0);
    Ok(())
}

/// Original-only bootstrap: no AppState, preferences, Pro credentials or
/// permission APIs. Explicit consent is obtained by the pre-Builder guard.
#[cfg(target_os = "macos")]
pub(crate) async fn bootstrap_original_update() -> Result<(), String> {
    ensure_recovery_process_is_alone()?;
    let info = public_check(env!("CARGO_PKG_VERSION")).await?
        .ok_or_else(|| "No stable final update is available; manually install v0.13.4.".to_string())?;
    if Version::parse(&info.latest_version).map_or(true, |v| v < Version(0, 13, 4)) {
        return Err("Recovery requires final version 0.13.4 or newer.".into());
    }
    schedule_macos(None, &info, Some("com.imploselabs.ultravox")).await
}

/// The bootstrap owns no sessions, but a separate app copy might. Never replace
/// the bundle while another UltraVox process could be recording user work.
#[cfg(target_os = "macos")]
fn ensure_recovery_process_is_alone() -> Result<(), String> {
    let output = Command::new("/usr/bin/pgrep").args(["-x", "ultravox"]).output()
        .map_err(|error| format!("Cannot inspect other UltraVox processes: {error}"))?;
    if !output.status.success() && output.status.code() != Some(1) {
        return Err("Cannot safely inspect other UltraVox processes; retry or install final manually.".into());
    }
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        let pid = line.trim().parse::<u32>()
            .map_err(|_| "Cannot safely read the UltraVox process list.".to_string())?;
        if pid != process::id() {
            return Err("Quit other UltraVox copies before retrying recovery; they may hold active work.".into());
        }
    }
    Ok(())
}

async fn schedule_macos(app: Option<&AppHandle>, info: &UpdateInfo, required_identifier: Option<&str>) -> Result<(), String> {
    let bundle = current_bundle()?;
    // Candidate identity is the trust boundary below. Requiring the running
    // bundle's Developer ID signature cannot make that process trustworthy and
    // blocks recovery from an ad-hoc dev/legacy install. Stable bundle metadata
    // preserves the canonical path/identifier; every downloaded replacement
    // still passes full Developer ID, designated-requirement, and notarization checks.
    verify_bundle_metadata(&bundle, &info.current_version)?;
    // The guard removes the staging folder on every early return below.
    let (staging, artifact, checksum) = fetch_platform_artifact(info).await?;
    verify_checksum(&artifact, &checksum)?;
    let unpacked_arg = staging.dir.join("unpacked");
    let archive_arg = artifact.to_string_lossy().into_owned();
    let unpacked_str = unpacked_arg.to_string_lossy().into_owned();
    run_checked(
        "/usr/bin/ditto",
        &["-x", "-k", archive_arg.as_str(), unpacked_str.as_str()],
        "unpack the update archive",
    )?;
    let payload_dir = artifact
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or(PAYLOAD_NAME);
    let mut candidate = unpacked_arg.join(payload_dir).join("UltraVox.app");
    if !candidate.is_dir() {
        // Unified, Light and Pro archive names share one payload; accept the
        // single UltraVox* folder whichever asset name it was served under.
        if let Some(found) = single_payload_app(&unpacked_arg) {
            candidate = found;
        }
    }
    verify_app(&candidate, &info.latest_version, true)?;
    if let Some(identifier) = required_identifier {
        let plist_id = command_output(
            "/usr/libexec/PlistBuddy",
            &["-c", "Print :CFBundleIdentifier", candidate.join("Contents/Info.plist").to_string_lossy().as_ref()],
            "inspect the recovery final identity",
        );
        if !matches!(plist_id, Ok(ref value) if value.trim() == identifier) {
            return Err("Recovery update must retain the original application identity.".into());
        }
    }
    // A recording may have started while the update downloaded. Bootstrap
    // cannot be busy: no stores, capture, hotkeys or frontend were initialized.
    if let Some(app) = app {
        ensure_idle(app).await?;
    }
    #[cfg(target_os = "macos")]
    if required_identifier.is_some() {
        ensure_recovery_process_is_alone()?;
    }
    let backup = bundle.with_file_name(format!(".UltraVox.previous-{}.app", process::id()));
    Command::new("/bin/sh")
        .arg("-c")
        .arg(INSTALL_HELPER)
        .arg("ultravox-update")
        .arg(process::id().to_string())
        .arg(&candidate)
        .arg(&bundle)
        .arg(&backup)
        .arg(&staging.dir)
        .arg(required_identifier.unwrap_or(""))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| format!("Failed to schedule update installation: {error}"))?;
    // The helper owns the staging folder now and removes it on every outcome.
    staging.keep();
    Ok(())
}

// Linux ------------------------------------------------------------------

#[cfg(target_os = "linux")]
async fn install_linux(app: AppHandle, info: &UpdateInfo) -> Result<(), String> {
    let running = std::env::var_os("APPIMAGE")
        .map(PathBuf::from)
        .filter(|path| path.exists())
        .ok_or_else(|| {
            "Updates require running an installed UltraVox AppImage; deb/rpm installs update through your package manager.".to_string()
        })?;
    // The guard removes the staging folder on every exit path.
    let (staging, artifact, checksum) = fetch_platform_artifact(info).await?;
    let hash_output = command_output(
        "/usr/bin/sha256sum",
        &[artifact.to_string_lossy().as_ref()],
        "hash the update artifact",
    )?;
    let actual = hash_output.split_whitespace().next().unwrap_or_default();
    let expected = fs::read_to_string(&checksum)
        .map_err(|error| format!("Failed to read update checksum: {error}"))?
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    if actual != expected || actual.len() != 64 {
        return Err("Update artifact failed its SHA-256 checksum.".to_string());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&artifact, fs::Permissions::from_mode(0o755))
            .map_err(|error| format!("Failed to mark the update executable: {error}"))?;
    }
    // Atomic replacement: the old image stays as `.previous` only until the
    // new version has run for a while (see `remove_rollback_copies`).
    let previous = previous_appimage(&running);
    let _ = fs::remove_file(&previous);
    fs::rename(&running, &previous)
        .map_err(|error| format!("Failed to back up the running AppImage: {error}"))?;
    if fs::rename(&artifact, &running).is_err() {
        rollback_previous(&previous, &running)?;
        return Err("Failed to install the downloaded AppImage.".to_string());
    }
    if let Err(error) = spawn_replacement(running.clone()) {
        // rename replaces the new image atomically with the kept one.
        rollback_previous(&previous, &running)?;
        return Err(error);
    }
    drop(staging);
    app.exit(0);
    Ok(())
}

#[cfg(not(target_os = "linux"))]
async fn install_linux(_app: AppHandle, _info: &UpdateInfo) -> Result<(), String> {
    unreachable!("Linux updates dispatch only on Linux")
}

#[cfg(target_os = "linux")]
fn spawn_replacement(target: PathBuf) -> Result<(), String> {
    process::Command::new(target)
        .spawn()
        .map(|_| ())
        .map_err(|error| format!("Failed to relaunch the updated app: {error}"))
}

/// The one rollback copy kept beside a replaced AppImage.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn previous_appimage(running: &Path) -> PathBuf {
    let name = running
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("UltraVox.AppImage");
    running.with_file_name(format!("{name}.previous"))
}

/// Restores the one kept previous release after a failed swap.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn rollback_previous(previous: &Path, target: &Path) -> Result<(), String> {
    fs::rename(previous, target)
        .map_err(|error| format!("Failed to restore the previous AppImage: {error}"))
}

#[cfg_attr(feature = "mirror-debug", allow(dead_code))]
fn is_macos_rollback_copy(name: &str) -> bool {
    name.starts_with(".UltraVox.previous-") && name.ends_with(".app")
}

/// Removes macOS rollback bundles (`.UltraVox.previous-<pid>.app`) in `dir`
/// and returns how many were removed.
#[cfg_attr(feature = "mirror-debug", allow(dead_code))]
fn remove_rollback_bundles_in(dir: &Path) -> usize {
    let Ok(entries) = fs::read_dir(dir) else {
        return 0;
    };
    entries
        .flatten()
        .filter(|entry| is_macos_rollback_copy(&entry.file_name().to_string_lossy()))
        .filter(|entry| match entry.file_type() {
            Ok(kind) if kind.is_dir() => fs::remove_dir_all(entry.path()).is_ok(),
            Ok(_) => fs::remove_file(entry.path()).is_ok(),
            Err(_) => false,
        })
        .count()
}

/// How long this version runs before the previous version's rollback copy is
/// deleted. The install helper finishes its health check well before this.
#[cfg_attr(feature = "mirror-debug", allow(dead_code))]
pub const ROLLBACK_COPY_GRACE: Duration = Duration::from_secs(60);

/// Called once this version has been running for [`ROLLBACK_COPY_GRACE`]: it
/// started successfully, so no rollback copy outlives it. Best effort; an
/// unwritable install folder is left alone.
#[cfg_attr(feature = "mirror-debug", allow(dead_code))]
pub fn remove_rollback_copies() {
    if let Ok(executable) = std::env::current_exe() {
        if let Some(folder) = executable
            .ancestors()
            .find(|path| path.extension().is_some_and(|extension| extension == "app"))
            .and_then(Path::parent)
        {
            remove_rollback_bundles_in(folder);
        }
    }
    if let Some(appimage) = std::env::var_os("APPIMAGE").map(PathBuf::from) {
        let _ = fs::remove_file(previous_appimage(&appimage));
    }
}

// Windows ----------------------------------------------------------------

#[cfg(target_os = "windows")]
fn install_windows(app: AppHandle, info: &UpdateInfo) -> Result<(), String> {
    // NSIS silent install. Verification uses the published immutable SHA-256;
    // Authenticode is additionally enforced when the vendor certificate signs
    // the installer once provisioned.
    let (staging, artifact, checksum) =
        tauri::async_runtime::block_on(fetch_platform_artifact(info))?;
    let script = format!(
        r#"$ErrorActionPreference='Stop'
$actual=(Get-FileHash -Algorithm SHA256 '{artifact}').Hash.ToLowerInvariant()
$expected=('{checksum}')
if ($actual -ne $expected.TrimEnd()) {{ exit 4 }}
$sig=Get-AuthenticodeSignature '{artifact}'
if ($sig.Status -eq 'HashMismatch' -or $sig.Status -eq 'NotTrusted') {{ exit 5 }}
$p=Start-Process -FilePath '{artifact}' -ArgumentList '/S' -Wait -PassThru
exit $p.ExitCode"#,
        artifact = artifact.to_string_lossy(),
        checksum = fs::read_to_string(&checksum)
            .map_err(|e| e.to_string())?
            .split_whitespace()
            .next()
            .unwrap_or_default(),
    );
    let result = process::Command::new("powershell")
        .args([
            "-NoProfile",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            &script,
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|error| format!("Failed to launch the update installer: {error}"))?;
    let code = result.code().unwrap_or(-1);
    drop(staging);
    match code {
        0 => {}
        4 => return Err("Windows update failed its SHA-256 checksum.".to_string()),
        5 => return Err("Windows update failed signature validation.".to_string()),
        n => return Err(format!("Windows update installer exited with code {n}.")),
    }
    app.exit(0);
    Ok(())
}

#[cfg(not(target_os = "windows"))]
fn install_windows(_app: AppHandle, _info: &UpdateInfo) -> Result<(), String> {
    unreachable!("Windows updates dispatch only on Windows")
}

/// The one `UltraVox*/UltraVox.app` payload inside an unpacked update archive.
fn single_payload_app(root: &Path) -> Option<PathBuf> {
    let mut found = fs::read_dir(root)
        .ok()?
        .filter_map(Result::ok)
        .filter(|entry| entry.file_name().to_string_lossy().starts_with("UltraVox"))
        .map(|entry| entry.path().join("UltraVox.app"))
        .filter(|app| app.is_dir());
    let first = found.next()?;
    found.next().is_none().then_some(first)
}

/// Pre-0.11 installs could leave separate edition bundles beside the unified app.
#[cfg(target_os = "macos")]
const LEGACY_EDITION_BUNDLES: &[&str] = &["UltraVox Light.app", "UltraVox Pro.app"];

/// Moves leftover "UltraVox Light.app" / "UltraVox Pro.app" copies to the Trash
/// so only the unified UltraVox remains. A copy is touched only when it carries
/// the UltraVox bundle identifier, is not this running app, and is not running.
#[cfg(target_os = "macos")]
pub fn reconcile_legacy_editions() {
    let running_bundle = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.ancestors().nth(3).map(Path::to_path_buf));
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let mut roots = vec![PathBuf::from("/Applications")];
    if let Some(home) = &home {
        roots.push(home.join("Applications"));
    }
    let Some(trash) = home.map(|home| home.join(".Trash")) else {
        return;
    };
    for root in roots {
        for name in LEGACY_EDITION_BUNDLES {
            let app = root.join(name);
            if !app.is_dir() || running_bundle.as_deref() == Some(app.as_path()) {
                continue;
            }
            let info = app.join("Contents/Info.plist");
            let info_arg = info.to_string_lossy().into_owned();
            let identifier = command_output(
                "/usr/libexec/PlistBuddy",
                &["-c", "Print :CFBundleIdentifier", info_arg.as_str()],
                "inspect a legacy UltraVox bundle",
            );
            if !matches!(
                identifier.as_deref().map(str::trim),
                Ok(EXPECTED_BUNDLE_ID) | Ok(LEGACY_BUNDLE_ID)
            ) {
                eprintln!(
                    "legacy edition cleanup: skipping {} (not UltraVox)",
                    app.display()
                );
                continue;
            }
            if bundle_is_running(&app) {
                eprintln!(
                    "legacy edition cleanup: {} is running; will retry next launch",
                    app.display()
                );
                continue;
            }
            match move_to_trash(&app, &trash) {
                Ok(target) => eprintln!(
                    "legacy edition cleanup: moved {} to {}",
                    app.display(),
                    target.display()
                ),
                Err(error) => eprintln!(
                    "legacy edition cleanup: {} left in place: {error}",
                    app.display()
                ),
            }
        }
    }
}

#[cfg(target_os = "macos")]
fn bundle_is_running(app: &Path) -> bool {
    let prefix = format!("{}/Contents/MacOS/", app.display());
    match command_output("/bin/ps", &["-axo", "comm="], "list running processes") {
        Ok(list) => list
            .lines()
            .any(|line| line.trim_start().starts_with(&prefix)),
        // Unknown means possibly running: never move a bundle we cannot vouch for.
        Err(_) => true,
    }
}

#[cfg(target_os = "macos")]
fn move_to_trash(app: &Path, trash: &Path) -> Result<PathBuf, String> {
    let name = app
        .file_name()
        .ok_or_else(|| "bundle has no file name".to_string())?
        .to_string_lossy()
        .into_owned();
    let mut target = trash.join(&name);
    let mut suffix = 1;
    while target.exists() {
        target = trash.join(format!("{} {suffix}.app", name.trim_end_matches(".app")));
        suffix += 1;
    }
    fs::rename(app, &target).map_err(|error| error.to_string())?;
    Ok(target)
}

#[cfg(test)]
mod tests {

    #[test]
    fn update_payload_is_found_under_any_single_ultravox_folder() {
        let root = std::env::temp_dir().join(format!("uv-payload-{}", std::process::id()));
        fs::create_dir_all(root.join("UltraVox-Pro-macos-arm64/UltraVox.app")).unwrap();
        assert_eq!(
            super::single_payload_app(&root),
            Some(root.join("UltraVox-Pro-macos-arm64/UltraVox.app"))
        );
        fs::create_dir_all(root.join("UltraVox-macos-arm64/UltraVox.app")).unwrap();
        assert_eq!(super::single_payload_app(&root), None);
        fs::remove_dir_all(&root).unwrap();
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn legacy_edition_bundles_move_to_trash_without_clobbering() {
        let root = std::env::temp_dir().join(format!("uv-legacy-{}", std::process::id()));
        let apps = root.join("Applications");
        let trash = root.join("Trash");
        fs::create_dir_all(apps.join("UltraVox Light.app/Contents")).unwrap();
        fs::create_dir_all(trash.join("UltraVox Light.app")).unwrap();
        let moved = super::move_to_trash(&apps.join("UltraVox Light.app"), &trash).unwrap();
        assert_eq!(moved, trash.join("UltraVox Light 1.app"));
        assert!(moved.join("Contents").is_dir());
        assert!(!apps.join("UltraVox Light.app").exists());
        fs::remove_dir_all(&root).unwrap();
    }

    use super::*;

    #[test]
    fn versions_require_exact_stable_semver() {
        assert_eq!(Version::parse("v0.2.2"), Some(Version(0, 2, 2)));
        assert!(Version::parse("0.2").is_none());
        assert!(Version::parse("0.2.2-beta").is_none());
        assert!(Version::parse("0.2.2.1").is_none());
    }

    #[cfg(not(feature = "local-qa"))]
    #[test]
    fn consumer_builds_ignore_the_qa_update_feed() {
        std::env::set_var("ULTRAVOX_QA_UPDATE_FEED", "http://127.0.0.1:9/");
        assert!(release_api_url().starts_with("https://api.github.com/repos/"));
        assert!(download_base_url("0.13.4").unwrap().starts_with("https://github.com/"));
        std::env::remove_var("ULTRAVOX_QA_UPDATE_FEED");
    }

    #[test]
    fn stable_feed_bypasses_the_pinned_bridge_and_selects_highest_supported_version() {
        let (artifact, checksum) = platform_assets();
        let release = |tag: &str| serde_json::json!({
            "tag_name": tag, "draft": false, "prerelease": false,
            "assets": [{"name": artifact}, {"name": checksum}]
        });
        let mut draft = release("v99.0.0"); draft["draft"] = true.into();
        let mut prerelease = release("v98.0.0"); prerelease["prerelease"] = true.into();
        let mut unsupported = release("v97.0.0"); unsupported["assets"] = serde_json::json!([]);
        let metadata = serde_json::json!([
            release("v0.13.4"), release("v0.13.3"), release("v0.14.0"), draft,
            prerelease, unsupported, release("v0.15.0-beta"), release("v00.16.0")
        ]);
        let update = public_update_info(&metadata, "0.13.3").unwrap().unwrap();
        assert_eq!(update.latest_version, "0.14.0");
        assert_eq!(update.release_url, format!("https://github.com/{PUBLIC_RELEASE_REPO}/releases/tag/v0.14.0"));
        assert!(public_update_info(&metadata, "0.14.0").unwrap().is_none());
        assert!(public_update_info(&metadata, "0.15.0").unwrap().is_none());
        assert!(public_update_info(&serde_json::json!([release("v0.13.3")]), "0.13.3").unwrap().is_none());
        assert_eq!(public_update_info(&serde_json::json!([release("v0.13.4")]), "0.13.3").unwrap().unwrap().latest_version, "0.13.4");
        assert!(public_update_info(&serde_json::json!({}), "0.13.3").is_err());
        assert!(public_update_info(&metadata, "invalid").is_err());
        assert!(public_update_info(&serde_json::json!([]), "0.13.3").unwrap().is_none());
    }

    #[cfg(not(feature = "local-qa"))]
    #[test]
    fn stable_download_uses_exact_selected_tag_not_legacy_latest() {
        assert_eq!(release_api_url(), format!("https://api.github.com/repos/{PUBLIC_RELEASE_REPO}/releases?per_page=100"));
        assert_eq!(download_base_url("0.13.4").unwrap(), format!("https://github.com/{PUBLIC_RELEASE_REPO}/releases/download/v0.13.4"));
        for invalid in ["../latest", "v0.13.4", "0.13.04", "0.13.4-beta", "0.13.4/asset", " 0.13.4"] {
            assert!(download_base_url(invalid).is_err(), "{invalid}");
        }
    }

    #[test]
    fn busy_refusals_carry_the_prefix_the_frontend_waits_on() {
        // apps/desktop/src/lib/updater.ts UPDATE_BUSY_PREFIX
        assert_eq!(UPDATE_BUSY_PREFIX, "Update waits:");
    }

    #[test]
    fn checks_default_on_and_auto_install_defaults_off() {
        let defaults = super::UpdatePreferences::default();
        assert!(!defaults.automatic, "auto-install is opt-in");
        assert!(defaults.check_automatically, "the daily check is on by default");
        let never_chose: super::UpdatePreferences = serde_json::from_str("{}").unwrap();
        assert!(!never_chose.automatic);
        assert!(never_chose.check_automatically);
        let legacy_on: super::UpdatePreferences =
            serde_json::from_str(r#"{"automatic":true}"#).unwrap();
        assert!(legacy_on.automatic, "a saved opt-in is kept");
        assert!(legacy_on.check_automatically);
        let checks_off: super::UpdatePreferences =
            serde_json::from_str(r#"{"automatic":true,"check_automatically":false}"#).unwrap();
        assert!(checks_off.automatic);
        assert!(!checks_off.check_automatically);
    }

    #[test]
    fn automatic_checks_run_at_most_daily_and_respect_the_opt_out() {
        use super::{decide_check, CheckDecision, CheckTrigger, CHECK_INTERVAL_SECS};
        let now = 1_000_000;
        let auto = CheckTrigger::Automatic;
        assert_eq!(decide_check(auto, true, None, None, now), CheckDecision::Network);
        assert_eq!(decide_check(auto, true, None, Some(now - 60), now), CheckDecision::Cached);
        assert_eq!(
            decide_check(auto, true, None, Some(now - CHECK_INTERVAL_SECS + 1), now),
            CheckDecision::Cached
        );
        assert_eq!(
            decide_check(auto, true, None, Some(now - CHECK_INTERVAL_SECS), now),
            CheckDecision::Network
        );
        assert!(matches!(decide_check(auto, false, None, None, now), CheckDecision::Off(_)));
        // Check Now ignores the interval and the automatic switch...
        let manual = CheckTrigger::Manual;
        assert_eq!(decide_check(manual, false, None, Some(now - 1), now), CheckDecision::Network);
        // ...but never an environment opt-out.
        assert!(matches!(
            decide_check(manual, true, Some("CI"), None, now),
            CheckDecision::Off(_)
        ));
        assert!(matches!(decide_check(auto, true, Some("CI"), None, now), CheckDecision::Off(_)));
    }

    #[test]
    fn ci_agents_env_opt_out_and_mirror_builds_silence_checks() {
        use super::environment_block_from;
        use std::ffi::OsString;
        let env = |pairs: &'static [(&'static str, &'static str)]| {
            move |name: &str| {
                pairs
                    .iter()
                    .find(|(key, _)| *key == name)
                    .map(|(_, value)| OsString::from(value))
            }
        };
        assert_eq!(environment_block_from(env(&[]), false), None);
        assert!(environment_block_from(env(&[("ULTRAVOX_NO_UPDATE_CHECK", "1")]), false)
            .unwrap()
            .contains("ULTRAVOX_NO_UPDATE_CHECK"));
        assert_eq!(environment_block_from(env(&[("ULTRAVOX_NO_UPDATE_CHECK", "0")]), false), None);
        assert!(environment_block_from(env(&[("CI", "true")]), false).is_some());
        assert_eq!(environment_block_from(env(&[("CI", "false")]), false), None);
        assert!(environment_block_from(env(&[("CLAUDECODE", "1")]), false).is_some());
        assert!(environment_block_from(env(&[]), true).is_some());
    }

    #[cfg(unix)]
    #[test]
    fn check_record_is_private_atomic_and_keeps_only_public_updates() {
        use std::os::unix::fs::PermissionsExt;
        let root = std::env::temp_dir().join(format!("uv-check-record-{}", uuid::Uuid::new_v4()));
        let path = root.join("update-check.json");
        let public = super::UpdateInfo {
            current_version: "0.13.6".into(),
            latest_version: "0.13.7".into(),
            release_url: "https://github.com/michael-berardi/ultravox/releases/tag/v0.13.7".into(),
            channel: super::UpdateChannel::Public,
            service_download_url: None,
            service_sha256: None,
            release_notes: "Fixes".into(),
        };
        let record = super::UpdateCheckRecord {
            last_checked_at: Some(42),
            info: Some(public.clone()),
            error: Some("Failed to reach GitHub releases: timed out".into()),
        };
        super::write_private_json(&path, &serde_json::to_vec(&record).unwrap()).unwrap();
        assert_eq!(fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        assert_eq!(fs::metadata(&root).unwrap().permissions().mode() & 0o777, 0o700);
        assert!(!path.with_extension("json.tmp").exists());
        let read = super::read_check_record(&path);
        assert_eq!(read.last_checked_at, Some(42));
        assert_eq!(read.error.as_deref(), Some("Failed to reach GitHub releases: timed out"));
        assert_eq!(super::cached_update(&read, "0.13.6").unwrap().latest_version, "0.13.7");
        assert!(super::cached_update(&read, "0.13.7").is_none(), "equal is not newer");
        assert!(super::cached_update(&read, "garbage").is_none());
        let service = super::UpdateCheckRecord {
            info: Some(super::UpdateInfo { channel: super::UpdateChannel::Service, ..public }),
            ..record
        };
        assert!(super::cached_update(&service, "0.13.6").is_none());
        fs::write(&path, b"not json").unwrap();
        assert!(super::read_check_record(&path).last_checked_at.is_none());
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn the_check_is_anonymous_and_bounded() {
        assert_eq!(super::CHECK_USER_AGENT, "ultravox-update-check");
        assert!(!super::CHECK_USER_AGENT.contains(env!("CARGO_PKG_VERSION")));
        assert!(super::CHECK_TIMEOUT <= std::time::Duration::from_secs(4));
    }

    #[test]
    fn signature_validation_requires_stable_identity() {
        let details = "Identifier=com.imploselabs.ultravox\nAuthority=Developer ID Application: Michael Berardi (T63VT9UAY2)\nTeamIdentifier=T63VT9UAY2\n";
        let requirement = "designated => identifier \"com.imploselabs.ultravox\" and anchor apple generic and certificate leaf[subject.OU] = \"T63VT9UAY2\"";
        assert!(validate_signature(details, requirement).is_ok());
        let codesign_requirement = "designated => identifier \"com.imploselabs.ultravox\" and anchor apple generic and certificate leaf[subject.OU] = T63VT9UAY2";
        assert!(validate_signature(details, codesign_requirement).is_ok());
        assert!(validate_signature(
            details,
            &codesign_requirement.replace(
                "identifier \"com.imploselabs.ultravox\"",
                "identifier \"com.imploselabs.ultravox.attacker\""
            )
        )
        .is_err());
        assert!(validate_signature(
            details,
            &codesign_requirement.replace("OU] = T63VT9UAY2", "OU] = T63VT9UAY2ATTACKER")
        )
        .is_err());
        assert!(
            validate_signature(&details.replace("T63VT9UAY2", "OTHERTEAM"), requirement).is_err()
        );
        assert!(validate_signature(
            details,
            "designated => identifier \"com.imploselabs.ultravox\""
        )
        .is_err());
    }

    #[test]
    fn signature_validation_rejects_temporary_identity_after_recovery() {
        let details = "Identifier=com.paretocybernetics.ultravox\nAuthority=Developer ID Application: Michael Berardi (T63VT9UAY2)\nTeamIdentifier=T63VT9UAY2\n";
        let requirement = "designated => identifier \"com.paretocybernetics.ultravox\" and anchor apple generic and certificate leaf[subject.OU] = T63VT9UAY2";
        assert!(validate_signature(details, requirement).is_err());
        // The requirement must name the identifier the signature carries.
        assert!(validate_signature(
            details,
            &requirement.replace("com.paretocybernetics.ultravox", "com.imploselabs.attacker")
        )
        .is_err());
        assert!(validate_signature(
            &details.replace("com.paretocybernetics.ultravox", "com.imploselabs.other"),
            &requirement.replace("com.paretocybernetics.ultravox", "com.imploselabs.other")
        )
        .is_err());
        assert!(
            validate_signature(&details.replace("T63VT9UAY2", "OTHERTEAM"), requirement).is_err()
        );
    }

    #[test]
    fn install_helper_accepts_only_the_original_final_identity() {
        assert!(INSTALL_HELPER.contains("for candidate in com.imploselabs.ultravox; do"));
        assert!(INSTALL_HELPER.contains(r#"designated => identifier \"${bundle_id}\""#));
    }

    #[test]
    fn current_bundle_metadata_allows_signed_recovery_from_ad_hoc_installs() {
        let root = tempfile::tempdir().unwrap();
        let app = root.path().join("UltraVox.app");
        let contents = app.join("Contents");
        std::fs::create_dir_all(&contents).unwrap();
        std::fs::write(
            contents.join("Info.plist"),
            format!(
                r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>CFBundleIdentifier</key><string>{EXPECTED_BUNDLE_ID}</string>
<key>CFBundleShortVersionString</key><string>{}</string>
</dict></plist>"#,
                env!("CARGO_PKG_VERSION")
            ),
        )
        .unwrap();

        assert!(verify_bundle_metadata(&app, env!("CARGO_PKG_VERSION")).is_ok());
    }
    #[test]
    fn update_bundle_name_must_be_exact() {
        assert!(validate_bundle_name(Path::new("/tmp/UltraVox.app")).is_ok());
        assert!(validate_bundle_name(Path::new("/tmp/Impostor.app")).is_err());
    }

    #[test]
    fn update_helper_retries_notarization_and_falls_back_safely() {
        let retry_loop = INSTALL_HELPER
            .find(r#"while [ "$attempt" -lt 4 ]"#)
            .expect("helper must retry spctl assessment");
        let status_fallback = INSTALL_HELPER
            .find("spctl --status")
            .expect("helper must accept updates when Gatekeeper is disabled");
        let stapler_fallback = INSTALL_HELPER
            .find("stapler validate")
            .expect("helper must validate stapled tickets as a fallback");
        assert!(retry_loop < status_fallback);
        assert!(status_fallback < stapler_fallback);
    }

    #[test]
    fn command_failures_include_exit_code_and_output_tail() {
        let output = Command::new("/bin/sh")
            .args(["-c", "echo gatekeeper says no >&2; exit 3"])
            .output()
            .expect("spawn sh");
        let message = format_command_failure("assess the update's notarization", &output);
        assert!(message.contains("exit 3"), "{message}");
        assert!(message.contains("gatekeeper says no"), "{message}");
        assert!(!message.contains("exit exit"), "{message}");
    }

    #[test]
    fn update_helper_requires_new_process_health_before_cleanup() {
        let cleanup = INSTALL_HELPER
            .rfind(r#"/bin/rm -rf "$backup_app" "$staging""#)
            .expect("helper must clean up only after launch");
        let process_snapshot = INSTALL_HELPER
            .find("existing_pids=")
            .expect("helper must snapshot existing processes");
        let health_probe = INSTALL_HELPER
            .find(r#"/bin/kill -0 "$launched_pid""#)
            .expect("helper must probe the new process");
        assert!(process_snapshot < health_probe);
        assert!(health_probe < cleanup);
        assert!(INSTALL_HELPER.contains(r#"if [ "$launched" -ne 1 ]; then"#));
    }

    fn feed_release(tag: &str) -> serde_json::Value {
        let (artifact, checksum) = platform_assets();
        serde_json::json!({
            "tag_name": tag, "draft": false, "prerelease": false,
            "body": "## Fixes\n\n- **Faster** `dictation`.",
            "assets": [{"name": artifact}, {"name": checksum}]
        })
    }

    #[test]
    fn stored_automatic_install_choices_are_kept_and_unset_means_off() {
        for stored in [true, false] {
            let preferences: UpdatePreferences =
                serde_json::from_str(&format!(r#"{{"automatic":{stored}}}"#)).unwrap();
            assert_eq!(preferences.automatic, stored);
            let reread: UpdatePreferences =
                serde_json::from_slice(&serde_json::to_vec(&preferences).unwrap()).unwrap();
            assert_eq!(reread, preferences);
        }
        let never_chose: UpdatePreferences = serde_json::from_str(r#"{"check_automatically":false}"#).unwrap();
        assert!(!never_chose.automatic, "no saved choice means automatic install is off");
        // Writing defaults adds no skip or notice keys.
        let written = serde_json::to_string(&UpdatePreferences::default()).unwrap();
        assert_eq!(written, r#"{"automatic":false,"check_automatically":true}"#);
    }

    #[test]
    fn saving_a_switch_keeps_the_other_switch_and_the_skip_state() {
        let stored = UpdatePreferences {
            automatic: true,
            check_automatically: true,
            skipped_version: Some("0.13.7".into()),
            last_notice_version: Some("0.13.7".into()),
            last_notice_at: Some(42),
        };
        let patch: UpdatePreferencesPatch = serde_json::from_str(r#"{"check_automatically":false}"#).unwrap();
        let next = stored.clone().apply(&patch);
        assert!(next.automatic, "an omitted switch keeps its stored value");
        assert!(!next.check_automatically);
        assert_eq!(next.skipped_version.as_deref(), Some("0.13.7"));
        assert_eq!(next.last_notice_at, Some(42));
        // The Settings panel sends both switches; that never clears the skip.
        let both: UpdatePreferencesPatch =
            serde_json::from_str(r#"{"automatic":false,"check_automatically":true}"#).unwrap();
        let next = stored.apply(&both);
        assert!(!next.automatic && next.check_automatically);
        assert_eq!(next.skipped_version.as_deref(), Some("0.13.7"));
    }

    #[test]
    fn the_notice_shows_once_a_day_per_version_and_never_for_a_skipped_one() {
        let now = 2_000_000_000;
        let mut preferences = UpdatePreferences::default();
        assert_eq!(notice_for(&preferences, "0.13.7", now), UpdateNotice::Show);
        preferences.last_notice_version = Some("0.13.7".into());
        preferences.last_notice_at = Some(now);
        assert_eq!(notice_for(&preferences, "0.13.7", now + CHECK_INTERVAL_SECS - 1), UpdateNotice::Recent);
        assert_eq!(notice_for(&preferences, "0.13.7", now + CHECK_INTERVAL_SECS), UpdateNotice::Show);
        assert_eq!(notice_for(&preferences, "0.13.8", now), UpdateNotice::Show, "a newer version notifies");
        assert_eq!(notice_for(&preferences, "0.13.7", now - 60), UpdateNotice::Show, "clock moved back");
        preferences.skipped_version = Some("0.13.7".into());
        assert_eq!(notice_for(&preferences, "0.13.7", now + 10 * CHECK_INTERVAL_SECS), UpdateNotice::Skipped);
        assert_eq!(notice_for(&preferences, "0.13.8", now), UpdateNotice::Show, "a newer release notifies again");
        assert_eq!(serde_json::to_string(&UpdateNotice::Skipped).unwrap(), r#""skipped""#);
    }

    #[test]
    fn automatic_checks_never_use_the_licensed_channel() {
        assert!(!may_use_licensed_channel(CheckTrigger::Automatic));
        assert!(may_use_licensed_channel(CheckTrigger::Manual));
    }

    #[test]
    fn test_feeds_must_be_loopback_http_without_credentials() {
        assert_eq!(loopback_feed("http://127.0.0.1:8123/").as_deref(), Some("http://127.0.0.1:8123"));
        assert_eq!(loopback_feed("http://localhost:8123/feed").as_deref(), Some("http://localhost:8123/feed"));
        for rejected in [
            "http://127.0.0.1:1@example.com/",
            "http://user:pass@127.0.0.1:8123/",
            "https://127.0.0.1:8123/",
            "http://127.0.0.1/",
            "http://127.0.0.1.example.com:8123/",
            "http://example.com:8123/",
            "http://10.0.0.1:8123/",
            "http://127.0.0.1:8123/?x=1",
            "file:///tmp/feed",
            "not a url",
        ] {
            assert!(loopback_feed(rejected).is_none(), "{rejected}");
        }
    }

    #[test]
    fn release_notes_are_short_plain_text() {
        assert_eq!(plain_release_notes("## Fixes\n\n- **Faster** `dictation`."), "Fixes\n- Faster dictation.");
        assert_eq!(plain_release_notes("  \n"), NO_RELEASE_NOTES);
        let clipped = plain_release_notes(&"x".repeat(2000));
        assert_eq!(clipped.chars().count(), RELEASE_NOTES_MAX_CHARS + 1);
        assert!(clipped.ends_with('…'));
        let info = public_update_info(&serde_json::json!([feed_release("v0.13.7")]), "0.13.6").unwrap().unwrap();
        assert_eq!(info.release_notes, "Fixes\n- Faster dictation.");
        // Cached records written before notes existed still load.
        let legacy: UpdateInfo = serde_json::from_str(
            r#"{"current_version":"0.13.6","latest_version":"0.13.7","release_url":"https://example.invalid"}"#,
        )
        .unwrap();
        assert_eq!(legacy.release_notes, "");
    }

    #[tokio::test]
    async fn the_check_is_one_anonymous_get_with_no_version_or_credentials() {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let payload = serde_json::json!([feed_release("v0.13.7"), feed_release("v0.13.3")]).to_string();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buffer = [0u8; 4096];
            let read = stream.read(&mut buffer).unwrap();
            let request = String::from_utf8_lossy(&buffer[..read]).to_string();
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                payload.len(),
                payload
            );
            stream.write_all(response.as_bytes()).unwrap();
            request
        });
        let found = public_check_from(&format!("http://{address}/releases?per_page=100"), "0.13.6")
            .await
            .unwrap()
            .unwrap();
        let request = server.join().unwrap();
        assert!(request.starts_with("GET /releases?per_page=100 HTTP/1.1\r\n"), "{request}");
        let lower = request.to_ascii_lowercase();
        assert!(lower.contains("user-agent: ultravox-update-check\r\n"), "{request}");
        assert!(!request.contains("0.13.6"), "the request must not carry the app version");
        assert!(!lower.contains("authorization") && !lower.contains("cookie"), "{request}");
        assert_eq!(found.latest_version, "0.13.7");
        println!("NOTICE: UltraVox {} is available — you have {}.", found.latest_version, found.current_version);
        println!("NOTES: {}", found.release_notes.replace('\n', " / "));
    }

    fn staging_dirs() -> Vec<PathBuf> {
        fs::read_dir(std::env::temp_dir())
            .unwrap()
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| path.file_name().is_some_and(|name| name.to_string_lossy().starts_with("ultravox-update-")))
            .collect()
    }

    #[tokio::test]
    async fn staging_never_outlives_a_failed_or_finished_download() {
        let dropped = Staging::create().unwrap();
        let dropped_dir = dropped.dir.clone();
        fs::write(dropped_dir.join("artifact"), "candidate").unwrap();
        drop(dropped);
        assert!(!dropped_dir.exists(), "dropping the guard removes the folder");

        let handed_off = Staging::create().unwrap();
        let kept_dir = handed_off.dir.clone();
        handed_off.keep();
        assert!(kept_dir.exists(), "the install helper owns a kept folder");
        fs::remove_dir_all(&kept_dir).unwrap();

        // A download that fails midway (closed port) removes its folder too.
        let before = staging_dirs();
        let closed = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap();
        let info = public_update_info(&serde_json::json!([feed_release("v0.13.7")]), "0.13.6").unwrap().unwrap();
        let result = fetch_platform_artifact_from(&info, Some(&format!("http://{closed}"))).await;
        assert!(result.is_err());
        let leaked: Vec<_> = staging_dirs().into_iter().filter(|path| !before.contains(path)).collect();
        assert!(leaked.is_empty(), "{leaked:?}");
        // A public candidate without a base URL fails before anything is fetched.
        assert!(fetch_platform_artifact_from(&info, None).await.is_err());
        let leaked: Vec<_> = staging_dirs().into_iter().filter(|path| !before.contains(path)).collect();
        assert!(leaked.is_empty(), "{leaked:?}");
    }

    #[test]
    fn rollback_restores_the_kept_previous_release() {
        let root = tempfile::tempdir().unwrap();
        let target = root.path().join("UltraVox.AppImage");
        let previous = previous_appimage(&target);
        assert_eq!(previous, root.path().join("UltraVox.AppImage.previous"));
        fs::write(&target, "broken").unwrap();
        fs::write(&previous, "working").unwrap();
        rollback_previous(&previous, &target).unwrap();
        assert_eq!(fs::read_to_string(&target).unwrap(), "working");
        assert!(!previous.exists());
    }

    #[test]
    fn rollback_copies_are_removed_once_the_new_version_runs() {
        let root = tempfile::tempdir().unwrap();
        let first = root.path().join(".UltraVox.previous-101.app");
        let second = root.path().join(".UltraVox.previous-102.app");
        fs::create_dir_all(first.join("Contents/MacOS")).unwrap();
        fs::create_dir(&second).unwrap();
        for kept in ["UltraVox.app", "Other.app", ".UltraVox.previous-1.txt", "UltraVox Light.app"] {
            fs::create_dir(root.path().join(kept)).unwrap();
        }
        assert_eq!(remove_rollback_bundles_in(root.path()), 2);
        assert!(!first.exists() && !second.exists());
        for kept in ["UltraVox.app", "Other.app", ".UltraVox.previous-1.txt", "UltraVox Light.app"] {
            assert!(root.path().join(kept).exists(), "{kept}");
        }
        assert_eq!(remove_rollback_bundles_in(&root.path().join("missing")), 0);
        assert!(ROLLBACK_COPY_GRACE <= std::time::Duration::from_secs(7 * 24 * 60 * 60));
        assert!(INSTALL_HELPER.contains(r#"backup_app="$4""#));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn checksum_mismatch_refuses_the_candidate_archive() {
        let root = tempfile::tempdir().unwrap();
        let archive = root.path().join("UltraVox-macos-arm64.zip");
        let checksum = root.path().join("UltraVox-macos-arm64.zip.sha256");
        fs::write(&archive, b"candidate").unwrap();
        fs::write(&checksum, format!("{}  UltraVox-macos-arm64.zip\n", "0".repeat(64))).unwrap();
        assert_eq!(
            verify_checksum(&archive, &checksum).unwrap_err(),
            "Update archive failed its SHA-256 checksum."
        );
        let actual = command_output("/usr/bin/shasum", &["-a", "256", archive.to_string_lossy().as_ref()], "hash the fixture").unwrap();
        let actual = actual.split_whitespace().next().unwrap().to_string();
        fs::write(&checksum, format!("{actual}  UltraVox-macos-arm64.zip\n")).unwrap();
        assert!(verify_checksum(&archive, &checksum).is_ok());
    }
}
