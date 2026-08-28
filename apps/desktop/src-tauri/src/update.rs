use std::fs;
use std::path::{Path, PathBuf};
use std::process::{self, Command, Stdio};
use std::sync::LazyLock;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

const RELEASE_API_URL: &str =
    "https://api.github.com/repos/michael-berardi/ultravox-light/releases/latest";
const DOWNLOAD_BASE_URL: &str =
    "https://github.com/michael-berardi/ultravox-light/releases/latest/download";
const RELEASE_PAGE_URL: &str = "https://github.com/michael-berardi/ultravox-light/releases";
const APP_BUNDLE_NAME: &str = "UltraVox.app";
const EXPECTED_BUNDLE_ID: &str = "com.imploselabs.ultravox";
const EXPECTED_TEAM_ID: &str = "T63VT9UAY2";
const HTTP_TIMEOUT: Duration = Duration::from_secs(30);
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(300);
static INSTALL_LOCK: LazyLock<tokio::sync::Mutex<()>> =
    LazyLock::new(|| tokio::sync::Mutex::new(()));

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

fn platform() -> &'static str {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => "macos-arm64",
        ("macos", "x86_64") => "macos-x86_64",
        ("linux", "x86_64") => "linux-x86_64",
        ("linux", "aarch64") => "linux-aarch64",
        ("windows", "x86_64") => "windows-x86_64",
        _ => "unsupported",
    }
}

fn platform_assets() -> Result<(String, String), String> {
    let platform = platform();
    if platform == "unsupported" {
        return Err("updates are unavailable on this platform".to_string());
    }
    let artifact = if platform.starts_with("macos-") {
        format!("UltraVox-Light-{platform}.zip")
    } else if platform.starts_with("linux-") {
        format!("UltraVox-Light-{platform}.AppImage")
    } else {
        format!("UltraVox-Light-{platform}-setup.exe")
    };
    Ok((artifact.clone(), format!("{artifact}.sha256")))
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct Version(u64, u64, u64);
impl Version {
    fn parse(text: &str) -> Option<Self> {
        let mut parts = text.trim().trim_start_matches('v').split('.');
        let version = Self(
            parts.next()?.parse().ok()?,
            parts.next()?.parse().ok()?,
            parts.next()?.parse().ok()?,
        );
        parts.next().is_none().then_some(version)
    }
}

fn preferences_path(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(app
        .path()
        .app_data_dir()
        .map_err(|e| e.to_string())?
        .join("update-preferences.json"))
}
pub fn read_preferences(app: &AppHandle) -> Result<UpdatePreferences, String> {
    match fs::read(preferences_path(app)?) {
        Ok(raw) => serde_json::from_slice(&raw).map_err(|e| e.to_string()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(UpdatePreferences::default()),
        Err(e) => Err(e.to_string()),
    }
}
pub fn write_preferences(app: &AppHandle, preferences: &UpdatePreferences) -> Result<(), String> {
    let path = preferences_path(app)?;
    let parent = path
        .parent()
        .ok_or("update preference path has no parent")?;
    fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    let temporary = path.with_extension("json.tmp");
    fs::write(
        &temporary,
        serde_json::to_vec(preferences).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    fs::rename(temporary, path).map_err(|e| e.to_string())
}
fn client(timeout: Duration) -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .user_agent(concat!(
            "ultravox-light-updater/",
            env!("CARGO_PKG_VERSION")
        ))
        .timeout(timeout)
        .build()
        .map_err(|e| e.to_string())
}

pub async fn check(current_version: &str) -> Result<Option<UpdateInfo>, String> {
    let response = client(HTTP_TIMEOUT)?
        .get(RELEASE_API_URL)
        .header(reqwest::header::ACCEPT, "application/vnd.github+json")
        .send()
        .await
        .map_err(|e| format!("failed to reach public releases: {e}"))?
        .error_for_status()
        .map_err(|e| format!("public release check failed: {e}"))?;
    let release: serde_json::Value = response
        .json()
        .await
        .map_err(|e| format!("failed to read release metadata: {e}"))?;
    if release["draft"].as_bool().unwrap_or(true) || release["prerelease"].as_bool().unwrap_or(true)
    {
        return Ok(None);
    }
    let latest_version = release["tag_name"]
        .as_str()
        .ok_or("release metadata is missing tag_name")?
        .trim_start_matches('v')
        .to_string();
    if Version::parse(&latest_version).is_none() || Version::parse(current_version).is_none() {
        return Err("release version is not stable semantic versioning".to_string());
    }
    if Version::parse(&latest_version) <= Version::parse(current_version) {
        return Ok(None);
    }
    let (artifact, checksum) = platform_assets()?;
    let assets = release["assets"]
        .as_array()
        .ok_or("release metadata is missing assets")?;
    for required in [&artifact, &checksum] {
        if !assets.iter().any(|asset| asset["name"] == **required) {
            return Err(format!(
                "public release is missing required asset {required}"
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

async fn download(url: &str, target: &Path) -> Result<(), String> {
    let bytes = client(DOWNLOAD_TIMEOUT)?
        .get(url)
        .send()
        .await
        .map_err(|e| format!("download failed: {e}"))?
        .error_for_status()
        .map_err(|e| format!("download failed: {e}"))?
        .bytes()
        .await
        .map_err(|e| format!("download failed: {e}"))?;
    fs::write(target, &bytes).map_err(|e| format!("failed to write {}: {e}", target.display()))
}
fn command_output(program: &str, args: &[&str], action: &str) -> Result<String, String> {
    let output = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("failed to {action}: {e}"))?;
    let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&output.stderr));
    if !output.status.success() {
        return Err(format!("failed to {action}: {}", text.trim()));
    }
    Ok(text.trim().to_string())
}
fn command(program: &str, args: &[&str], action: &str) -> Result<(), String> {
    command_output(program, args, action).map(|_| ())
}
fn verify_checksum(artifact: &Path, checksum: &Path) -> Result<(), String> {
    let expected = fs::read_to_string(checksum)
        .map_err(|e| e.to_string())?
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    if expected.len() != 64 || !expected.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("public update checksum is invalid".to_string());
    }
    let path = artifact.to_string_lossy();
    let (program, args) = if cfg!(windows) {
        ("certutil", vec!["-hashfile", path.as_ref(), "SHA256"])
    } else if cfg!(target_os = "linux") {
        ("/usr/bin/sha256sum", vec![path.as_ref()])
    } else {
        ("/usr/bin/shasum", vec!["-a", "256", path.as_ref()])
    };
    let output = Command::new(program)
        .args(args)
        .output()
        .map_err(|e| e.to_string())?;
    if !output.status.success() {
        return Err("could not calculate public update checksum".to_string());
    }
    let actual = String::from_utf8_lossy(&output.stdout)
        .split_whitespace()
        .find(|part| part.len() == 64 && part.bytes().all(|b| b.is_ascii_hexdigit()))
        .unwrap_or_default()
        .to_ascii_lowercase();
    (actual == expected)
        .then_some(())
        .ok_or_else(|| "public update failed its SHA-256 checksum".to_string())
}

fn verify_macos_candidate(candidate: &Path, version: &str) -> Result<(), String> {
    let info = candidate.join("Contents/Info.plist");
    let info_arg = info.to_string_lossy();
    let identifier = command_output(
        "/usr/libexec/PlistBuddy",
        &["-c", "Print :CFBundleIdentifier", info_arg.as_ref()],
        "inspect the public update identifier",
    )?;
    if identifier != EXPECTED_BUNDLE_ID {
        return Err("public update bundle identifier does not match UltraVox Light".to_string());
    }
    let candidate_version = command_output(
        "/usr/libexec/PlistBuddy",
        &["-c", "Print :CFBundleShortVersionString", info_arg.as_ref()],
        "inspect the public update version",
    )?;
    if candidate_version != version {
        return Err("public update bundle version does not match the checked release".to_string());
    }
    let candidate_arg = candidate.to_string_lossy();
    command(
        "/usr/bin/codesign",
        &["--verify", "--deep", "--strict", candidate_arg.as_ref()],
        "verify the public update signature",
    )?;
    let signature = command_output(
        "/usr/bin/codesign",
        &["-dv", "--verbose=4", candidate_arg.as_ref()],
        "inspect the public update signature",
    )?;
    if !signature.contains(&format!("Identifier={EXPECTED_BUNDLE_ID}"))
        || !signature.contains(&format!("TeamIdentifier={EXPECTED_TEAM_ID}"))
        || !signature.contains("Authority=Developer ID Application:")
        || signature.contains("Signature=adhoc")
    {
        return Err(
            "public update is not signed by the expected UltraVox Light Developer ID".to_string(),
        );
    }
    let requirement = command_output(
        "/usr/bin/codesign",
        &["-d", "-r-", candidate_arg.as_ref()],
        "inspect the public update requirement",
    )?;
    if !requirement.contains(EXPECTED_BUNDLE_ID)
        || !requirement.contains(EXPECTED_TEAM_ID)
        || !requirement.contains("anchor apple generic")
    {
        return Err(
            "public update designated requirement does not match UltraVox Light".to_string(),
        );
    }
    command(
        "/usr/bin/xcrun",
        &["stapler", "validate", candidate_arg.as_ref()],
        "verify the public update notarization",
    )
}

pub async fn install(app: AppHandle, info: UpdateInfo) -> Result<(), String> {
    let _guard = INSTALL_LOCK
        .try_lock()
        .map_err(|_| "an UltraVox Light update is already being installed".to_string())?;
    if info.current_version != env!("CARGO_PKG_VERSION") {
        return Err("update was checked against a different app version".to_string());
    }
    if Version::parse(&info.latest_version) <= Version::parse(&info.current_version) {
        return Err("update is not newer than the running app".to_string());
    }
    let (artifact_name, checksum_name) = platform_assets()?;
    let staging =
        std::env::temp_dir().join(format!("ultravox-light-update-{}", uuid::Uuid::new_v4()));
    fs::create_dir(&staging).map_err(|e| e.to_string())?;
    let artifact = staging.join(&artifact_name);
    let checksum = staging.join(&checksum_name);
    let result = async {
        download(&format!("{DOWNLOAD_BASE_URL}/{artifact_name}"), &artifact).await?;
        download(&format!("{DOWNLOAD_BASE_URL}/{checksum_name}"), &checksum).await?;
        verify_checksum(&artifact, &checksum)?;
        match platform() {
            "macos-arm64" | "macos-x86_64" => {
                install_macos(&app, &artifact, &staging, &info.latest_version)
            }
            "linux-x86_64" | "linux-aarch64" => install_linux(&app, &artifact, &checksum),
            "windows-x86_64" => install_windows(&app, &artifact, &checksum),
            _ => Err("updates are unavailable on this platform".to_string()),
        }
    }
    .await;
    if result.is_err() {
        let _ = fs::remove_dir_all(&staging);
    }
    result
}

fn install_macos(
    app: &AppHandle,
    artifact: &Path,
    staging: &Path,
    version: &str,
) -> Result<(), String> {
    let unpacked = staging.join("unpacked");
    command(
        "/usr/bin/ditto",
        &[
            "-x",
            "-k",
            artifact.to_string_lossy().as_ref(),
            unpacked.to_string_lossy().as_ref(),
        ],
        "unpack the public update",
    )?;
    let candidate =
        find_bundle(&unpacked).ok_or("public update archive did not contain UltraVox.app")?;
    verify_macos_candidate(&candidate, version)?;
    let current = std::env::current_exe().map_err(|e| e.to_string())?;
    let target = current
        .ancestors()
        .find(|path| path.extension().is_some_and(|e| e == "app"))
        .ok_or("updates require an installed app bundle")?
        .to_path_buf();
    let backup = target.with_file_name(format!(".UltraVox-Light.previous-{}.app", process::id()));
    fs::rename(&target, &backup).map_err(|e| format!("failed to stage current app: {e}"))?;
    if let Err(error) = fs::rename(&candidate, &target) {
        let _ = fs::rename(&backup, &target);
        return Err(format!("failed to install public update: {error}"));
    }
    let pid = process::id().to_string();
    let target_arg = target.to_string_lossy().into_owned();
    Command::new("/bin/sh")
        .args(["-c", "while /bin/kill -0 \"$1\" 2>/dev/null; do /bin/sleep 0.2; done; /usr/bin/open -n \"$2\"", "ultravox-light-updater", &pid, &target_arg])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| {
            let _ = fs::remove_dir_all(&target);
            let _ = fs::rename(&backup, &target);
            format!("failed to schedule public update relaunch: {error}")
        })?;
    app.exit(0);
    Ok(())
}
fn find_bundle(root: &Path) -> Option<PathBuf> {
    if root.file_name().and_then(|n| n.to_str()) == Some(APP_BUNDLE_NAME) {
        return Some(root.to_path_buf());
    }
    fs::read_dir(root).ok()?.flatten().find_map(|entry| {
        entry
            .file_type()
            .ok()
            .filter(|t| t.is_dir())
            .and_then(|_| find_bundle(&entry.path()))
    })
}

#[cfg(target_os = "linux")]
fn install_linux(app: &AppHandle, artifact: &Path, _checksum: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    let running = std::env::var_os("APPIMAGE")
        .map(PathBuf::from)
        .filter(|p| p.exists())
        .ok_or("updates require a running AppImage")?;
    fs::set_permissions(artifact, fs::Permissions::from_mode(0o755)).map_err(|e| e.to_string())?;
    let previous = running.with_extension("AppImage.previous");
    fs::rename(&running, &previous).map_err(|e| e.to_string())?;
    if let Err(e) = fs::rename(artifact, &running) {
        let _ = fs::rename(previous, &running);
        return Err(e.to_string());
    }
    Command::new(&running).spawn().map_err(|e| e.to_string())?;
    app.exit(0);
    Ok(())
}
#[cfg(not(target_os = "linux"))]
fn install_linux(_app: &AppHandle, _artifact: &Path, _checksum: &Path) -> Result<(), String> {
    Err("AppImage updates require Linux".to_string())
}
#[cfg(target_os = "windows")]
fn install_windows(app: &AppHandle, artifact: &Path, _checksum: &Path) -> Result<(), String> {
    Command::new(artifact)
        .args(["/S"])
        .spawn()
        .map_err(|e| e.to_string())?;
    app.exit(0);
    Ok(())
}
#[cfg(not(target_os = "windows"))]
fn install_windows(_app: &AppHandle, _artifact: &Path, _checksum: &Path) -> Result<(), String> {
    Err("installer updates require Windows".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn versions_are_stable() {
        assert_eq!(Version::parse("v0.2.2"), Some(Version(0, 2, 2)));
        assert!(Version::parse("0.2").is_none());
        assert!(Version::parse("0.2.2-beta").is_none());
    }
}
