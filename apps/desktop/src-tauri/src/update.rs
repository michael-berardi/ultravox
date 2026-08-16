use std::fs;
use std::path::{Path, PathBuf};
use std::process::{self, Command, Stdio};
use std::sync::OnceLock;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

const RELEASE_API_URL: &str =
    "https://api.github.com/repos/michael-berardi/ultravox/releases/latest";
const DOWNLOAD_BASE_URL: &str =
    "https://github.com/michael-berardi/ultravox/releases/latest/download";
const RELEASE_PAGE_URL: &str = "https://github.com/michael-berardi/ultravox/releases";
const ARCHIVE_NAME: &str = "UltraVox-macos-arm64.zip";
const CHECKSUM_NAME: &str = "UltraVox-macos-arm64.zip.sha256";
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

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct UpdateInfo {
    pub current_version: String,
    pub latest_version: String,
    pub release_url: String,
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
    Ok(app
        .path()
        .app_data_dir()
        .map_err(|error| error.to_string())?
        .join("update-preferences.json"))
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

pub async fn check(current_version: &str) -> Result<Option<UpdateInfo>, String> {
    let response = http_client(HTTP_TIMEOUT)?
        .get(RELEASE_API_URL)
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
    for required in [ARCHIVE_NAME, CHECKSUM_NAME] {
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
            .unwrap_or(RELEASE_PAGE_URL)
            .to_string(),
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
    let status = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|error| format!("Failed to {action}: {error}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("Failed to {action} (exit {status})."))
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
        Err(format!("Failed to {action} (exit {}).", output.status))
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
    if !requirements.contains("designated =>")
        || !requirements.contains(&format!("identifier \"{EXPECTED_BUNDLE_ID}\""))
        || !requirements.contains("anchor apple generic")
        || !requirements.contains("certificate")
        || !requirements.contains(&format!("OU] = \"{EXPECTED_TEAM_ID}\""))
    {
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

fn verify_app(app: &Path, expected_version: &str) -> Result<(), String> {
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
    run_checked(
        "/usr/sbin/spctl",
        &["--assess", "--type", "execute", app_arg.as_str()],
        "verify the update's notarization",
    )
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

async fn stage_update(staging: &Path, expected_version: &str) -> Result<PathBuf, String> {
    let archive = staging.join(ARCHIVE_NAME);
    let checksum = staging.join(CHECKSUM_NAME);
    download(&format!("{DOWNLOAD_BASE_URL}/{ARCHIVE_NAME}"), &archive).await?;
    download(&format!("{DOWNLOAD_BASE_URL}/{CHECKSUM_NAME}"), &checksum).await?;
    verify_checksum(&archive, &checksum)?;
    let unpacked = staging.join("unpacked");
    let archive_arg = archive.to_string_lossy().into_owned();
    let unpacked_arg = unpacked.to_string_lossy().into_owned();
    run_checked(
        "/usr/bin/ditto",
        &["-x", "-k", archive_arg.as_str(), unpacked_arg.as_str()],
        "unpack the update archive",
    )?;
    let candidate = unpacked.join(PAYLOAD_NAME).join("UltraVox.app");
    verify_app(&candidate, expected_version)?;
    Ok(candidate)
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
  /usr/sbin/spctl --assess --type execute "$1" >/dev/null 2>&1 || return 1
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
if ! /usr/bin/open -n "$target_app"; then
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
    let bundle = current_bundle()?;
    verify_app(&bundle, &info.current_version)?;
    let staging =
        std::env::temp_dir().join(format!("ultravox-update-{}", uuid::Uuid::new_v4()));
    fs::create_dir(&staging)
        .map_err(|error| format!("Failed to create update staging directory: {error}"))?;
    let candidate = match stage_update(&staging, &info.latest_version).await {
        Ok(candidate) => candidate,
        Err(error) => {
            let _ = fs::remove_dir_all(&staging);
            return Err(error);
        }
    };
    let backup = bundle.with_file_name(format!(".UltraVox.previous-{}.app", process::id()));
    let result = Command::new("/bin/sh")
        .arg("-c")
        .arg(INSTALL_HELPER)
        .arg("ultravox-update")
        .arg(process::id().to_string())
        .arg(&candidate)
        .arg(&bundle)
        .arg(&backup)
        .arg(&staging)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| format!("Failed to schedule update installation: {error}"));
    if result.is_err() {
        let _ = fs::remove_dir_all(&staging);
    }
    result?;
    app.exit(0);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_require_exact_stable_semver() {
        assert_eq!(Version::parse("v0.2.2"), Some(Version(0, 2, 2)));
        assert!(Version::parse("0.2").is_none());
        assert!(Version::parse("0.2.2-beta").is_none());
        assert!(Version::parse("0.2.2.1").is_none());
    }

    #[test]
    fn signature_validation_requires_stable_identity() {
        let details = "Identifier=com.imploselabs.ultravox\nAuthority=Developer ID Application: Michael Berardi (T63VT9UAY2)\nTeamIdentifier=T63VT9UAY2\n";
        let requirement = "designated => identifier \"com.imploselabs.ultravox\" and anchor apple generic and certificate leaf[subject.OU] = \"T63VT9UAY2\"";
        assert!(validate_signature(details, requirement).is_ok());
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
    fn update_bundle_name_must_be_exact() {
        assert!(validate_bundle_name(Path::new("/tmp/UltraVox.app")).is_ok());
        assert!(validate_bundle_name(Path::new("/tmp/Impostor.app")).is_err());
    }
}
