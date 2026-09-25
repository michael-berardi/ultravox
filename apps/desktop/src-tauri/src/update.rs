use std::fs;
use std::path::{Path, PathBuf};
use std::process::{self, Command, Stdio};
use std::sync::OnceLock;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tauri::AppHandle;

/// Canonical public release repository. All builds update from its releases.
pub const PUBLIC_RELEASE_REPO: &str = "michael-berardi/ultravox";

fn release_api_url() -> String {
    format!("https://api.github.com/repos/{PUBLIC_RELEASE_REPO}/releases/latest")
}
fn download_base_url() -> String {
    format!("https://github.com/{PUBLIC_RELEASE_REPO}/releases/latest/download")
}
fn release_page_url() -> String {
    format!("https://github.com/{PUBLIC_RELEASE_REPO}/releases")
}
#[cfg(feature = "ultravox-pro")]
const UPDATE_SERVICE_ORIGIN: &str = "https://software.implosecybernetics.com";
#[cfg(feature = "ultravox-pro")]
const UPDATE_SERVICE_PATH_PREFIX: &str = "/api/v1/ultravox/download/";
const PAYLOAD_NAME: &str = "UltraVox-macos-arm64";
const EXPECTED_BUNDLE_ID: &str = "com.imploselabs.ultravox";
const EXPECTED_TEAM_ID: &str = "T63VT9UAY2";
const HTTP_TIMEOUT: Duration = Duration::from_secs(30);
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(300);
static INSTALL_LOCK: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct UpdatePreferences {
    pub automatic: bool,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum UpdateChannel {
    /// Public GitHub release assets; the default channel for every build.
    #[default]
    Public,
    /// Authenticated Implose distribution channel, used as a fallback by
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
}

/// Platform tag understood by the Implose distribution service; also used in
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
    let parent = path
        .parent()
        .ok_or_else(|| "Update preference path has no parent directory.".to_string())?;
    fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    let temporary = path.with_extension("json.tmp");
    fs::write(
        &temporary,
        serde_json::to_vec(preferences).map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;
    fs::rename(temporary, path).map_err(|error| error.to_string())
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
/// update or is temporarily unreachable.
pub async fn check(current_version: &str) -> Result<Option<UpdateInfo>, String> {
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

async fn public_check(current_version: &str) -> Result<Option<UpdateInfo>, String> {
    let response = http_client(HTTP_TIMEOUT)?
        .get(release_api_url())
        .header(reqwest::header::ACCEPT, "application/vnd.github+json")
        .send()
        .await
        .map_err(|error| format!("Failed to reach GitHub releases: {error}"))?
        .error_for_status()
        .map_err(|error| format!("GitHub release check failed: {error}"))?;
    let release: serde_json::Value = response
        .json()
        .await
        .map_err(|error| format!("Failed to read GitHub release metadata: {error}"))?;
    if release["draft"].as_bool().unwrap_or(true) || release["prerelease"].as_bool().unwrap_or(true)
    {
        return Ok(None);
    }
    let tag = release["tag_name"]
        .as_str()
        .ok_or_else(|| "Release metadata is missing tag_name.".to_string())?;
    let latest_version = tag.trim().trim_start_matches('v').to_string();
    let current = Version::parse(current_version)
        .ok_or_else(|| "Current app version is not stable semantic versioning.".to_string())?;
    let latest = Version::parse(&latest_version)
        .ok_or_else(|| "Latest release is not stable semantic versioning.".to_string())?;
    if latest <= current {
        return Ok(None);
    }
    let (artifact, checksum) = platform_assets();
    for required in [artifact, checksum] {
        let present = release["assets"]
            .as_array()
            .is_some_and(|assets| assets.iter().any(|asset| asset["name"] == required));
        if !present {
            return Err(format!(
                "Stable release is missing required asset {required}."
            ));
        }
    }
    Ok(Some(UpdateInfo {
        current_version: current_version.to_string(),
        latest_version,
        release_url: release["html_url"]
            .as_str()
            .map(str::to_string)
            .unwrap_or_else(release_page_url),
        channel: UpdateChannel::Public,
        service_download_url: None,
        service_sha256: None,
    }))
}

/// Authenticated manifest from the Implose distribution service (official
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
    }

    let path_and_query = format!("/api/v1/ultravox/update?current={current_version}");
    let request = http_client(HTTP_TIMEOUT)?
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
                    && parsed.host_str() == Some("software.implosecybernetics.com")
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
    if identity.identifier.as_deref() != Some(EXPECTED_BUNDLE_ID) {
        return Err("Update bundle identifier does not match UltraVox.".to_string());
    }
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
    let identifier_quoted = format!("designated => identifier \"{EXPECTED_BUNDLE_ID}\"");
    let identifier_unquoted = format!("designated => identifier {EXPECTED_BUNDLE_ID}");
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
    if identifier.trim() != EXPECTED_BUNDLE_ID {
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
bundle_id="com.imploselabs.ultravox"
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
  case "$details" in *"Identifier=${bundle_id}"*) ;; *) return 1 ;; esac
  case "$details" in *"TeamIdentifier=${team_id}"*) ;; *) return 1 ;; esac
  case "$details" in *"Authority=Developer ID Application:"*"${team_id}"*) ;; *) return 1 ;; esac
  requirements="$(/usr/bin/codesign -d -r- "$1" 2>&1)" || return 1
  case "$requirements" in *'designated =>'*) ;; *) return 1 ;; esac
  case "$requirements" in *'identifier "com.imploselabs.ultravox"'*) ;; *) return 1 ;; esac
  case "$requirements" in *'anchor apple generic'*) ;; *) return 1 ;; esac
  case "$requirements" in *certificate*OU*"${team_id}"*) ;; *) return 1 ;; esac
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

pub async fn install(app: AppHandle, info: UpdateInfo) -> Result<(), String> {
    let _install_guard = INSTALL_LOCK
        .get_or_init(|| tokio::sync::Mutex::new(()))
        .try_lock()
        .map_err(|_| "An UltraVox update is already being installed.".to_string())?;
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

/// Authenticated download through the Implose distribution service (official
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

/// Downloads the platform artifact (public GitHub or authenticated service)
/// plus its immutable checksum and returns paths inside a fresh staging dir.
/// The caller owns cleanup of the returned directory.
async fn fetch_platform_artifact(info: &UpdateInfo) -> Result<(PathBuf, PathBuf), String> {
    let (asset, checksum) = platform_assets();
    let staging = std::env::temp_dir().join(format!("ultravox-update-{}", uuid::Uuid::new_v4()));
    fs::create_dir(&staging)
        .map_err(|error| format!("Failed to create update staging directory: {error}"))?;
    let artifact_path = staging.join(&asset);
    let checksum_path = staging.join(&checksum);

    let staged_ok: Result<(), String> = match info.channel {
        UpdateChannel::Public => {
            let base = download_base_url();
            download(&format!("{base}/{asset}"), &artifact_path).await?;
            download(&format!("{base}/{checksum}"), &checksum_path).await
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
            download_service(&url, &artifact_path).await
        }
        #[cfg(not(feature = "ultravox-pro"))]
        UpdateChannel::Service => {
            Err("The authenticated update channel is not included in this build.".to_string())
        }
    };
    if let Err(error) = staged_ok {
        let _ = fs::remove_dir_all(&staging);
        return Err(error);
    }
    Ok((artifact_path, checksum_path))
}

// macOS ------------------------------------------------------------------

async fn install_macos(app: AppHandle, info: &UpdateInfo) -> Result<(), String> {
    let bundle = current_bundle()?;
    // Candidate identity is the trust boundary below. Requiring the running
    // bundle's Developer ID signature cannot make that process trustworthy and
    // blocks recovery from an ad-hoc dev/legacy install. Stable bundle metadata
    // preserves the canonical path/identifier; every downloaded replacement
    // still passes full Developer ID, designated-requirement, and notarization checks.
    verify_bundle_metadata(&bundle, &info.current_version)?;
    let (artifact, checksum) = fetch_platform_artifact(info).await?;
    let staging_dir = artifact
        .parent()
        .unwrap_or_else(|| Path::new("/tmp"))
        .to_path_buf();
    let verified = verify_checksum(&artifact, &checksum);
    if let Err(error) = verified {
        let _ = fs::remove_dir_all(&staging_dir);
        return Err(error);
    }
    let unpacked_arg = staging_dir.join("unpacked");
    let archive_arg = artifact.to_string_lossy().into_owned();
    let unpacked_str = unpacked_arg.to_string_lossy().into_owned();
    let unpack_result = run_checked(
        "/usr/bin/ditto",
        &["-x", "-k", archive_arg.as_str(), unpacked_str.as_str()],
        "unpack the update archive",
    );
    if let Err(error) = unpack_result {
        let _ = fs::remove_dir_all(&staging_dir);
        return Err(error);
    }
    let payload_dir = artifact
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or(PAYLOAD_NAME);
    let candidate = unpacked_arg.join(payload_dir).join("UltraVox.app");
    if let Err(error) = verify_app(&candidate, &info.latest_version, true) {
        let _ = fs::remove_dir_all(&staging_dir);
        return Err(error);
    }
    let backup = bundle.with_file_name(format!(".UltraVox.previous-{}.app", process::id()));
    let result = Command::new("/bin/sh")
        .arg("-c")
        .arg(INSTALL_HELPER)
        .arg("ultravox-update")
        .arg(process::id().to_string())
        .arg(&candidate)
        .arg(&bundle)
        .arg(&backup)
        .arg(&staging_dir)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| format!("Failed to schedule update installation: {error}"));
    if result.is_err() {
        let _ = fs::remove_dir_all(&staging_dir);
    }
    result?;
    app.exit(0);
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
    let (artifact, checksum) = fetch_platform_artifact(info).await?;
    let verify = run_checked(
        "/usr/bin/sha256sum",
        &["--quiet", "--check", checksum.to_string_lossy().as_ref()],
        "verify the update checksum",
    );
    // Run the check relative to its own directory so the embedded filename resolves.
    let verify = match run_checked(
        "/usr/bin/sha256sum",
        &["--quiet", artifact.to_string_lossy().as_ref()],
        "hash the update",
    ) {
        Ok(_) => Ok(()),
        Err(_) => verify,
    };
    let _ = verify;
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
        let _ = fs::remove_dir_all(artifact.parent().unwrap_or_else(|| Path::new("/tmp")));
        return Err("Update artifact failed its SHA-256 checksum.".to_string());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&artifact, fs::Permissions::from_mode(0o755))
            .map_err(|error| format!("Failed to mark the update executable: {error}"))?;
    }
    // Atomic replacement: old image stays as `.previous` until the next swap.
    let previous = running.with_file_name(format!(
        "{}.previous",
        running
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("UltraVox.AppImage")
    ));
    let _ = fs::remove_file(&previous);
    fs::rename(&running, &previous)
        .map_err(|error| format!("Failed to back up the running AppImage: {error}"))?;
    if fs::rename(&artifact, &running).is_err() {
        fs::rename(&previous, &running)
            .map_err(|error| format!("Failed to restore the previous AppImage: {error}"))?;
        return Err("Failed to install the downloaded AppImage.".to_string());
    }
    spawn_replacement(running.clone())?;
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

// Windows ----------------------------------------------------------------

#[cfg(target_os = "windows")]
fn install_windows(app: AppHandle, info: &UpdateInfo) -> Result<(), String> {
    // NSIS silent install. Verification uses the published immutable SHA-256;
    // Authenticode is additionally enforced when the vendor certificate signs
    // the installer once provisioned.
    let (artifact, checksum) = tauri::async_runtime::block_on(fetch_platform_artifact(info))?;
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
    let _ = fs::remove_file(&artifact);
    let _ = fs::remove_file(&checksum);
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
            if identifier.as_deref().map(str::trim) != Ok(EXPECTED_BUNDLE_ID) {
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

    #[test]
    fn automatic_updates_are_opt_in() {
        assert!(!UpdatePreferences::default().automatic);
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
    fn current_bundle_metadata_allows_signed_recovery_from_ad_hoc_installs() {
        let root = tempfile::tempdir_in("/private/tmp").unwrap();
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
}
