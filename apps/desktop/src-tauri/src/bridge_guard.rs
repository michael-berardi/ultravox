//! The temporary Pareto identity must never initialize an original-only user's
//! stores or ask for new grants. Run BEFORE constructing Tauri/webviews.
use std::fs;
use std::path::Path;

const ORIGINAL_ID: &str = "com.imploselabs.ultravox";
const BRIDGE_ID: &str = "com.paretocybernetics.ultravox";
const FINAL_DOWNLOAD: &str = "https://github.com/michael-berardi/ultravox/releases/tag/v0.13.4";

/// Read-only, fail-closed inspection; even an empty original directory counts.
/// Existing Pareto users keep the normal bridge/final-migration path.
fn needs_final(identifier: &str, base: &Path) -> Result<bool, String> {
    if identifier != BRIDGE_ID {
        return Ok(false);
    }
    let exists = |id: &str| match fs::symlink_metadata(base.join(id)) {
        Ok(meta) if meta.is_dir() => Ok(true),
        Ok(_) => Err(format!("{id} data is not a plain directory; install the final release manually.")),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(format!("Cannot inspect {id} data: {error}")),
    };
    Ok(exists(ORIGINAL_ID)? && !exists(BRIDGE_ID)?)
}

#[cfg(target_os = "macos")]
fn dialog(message: &str, retry: bool) -> Result<String, String> {
    // Message is an argv value, never executable AppleScript (network errors
    // may contain quotes). This asks for update consent, NOT OS permissions.
    let script = if retry {
        "on run argv\nreturn button returned of (display dialog (item 1 of argv) with title \"UltraVox recovery\" buttons {\"Quit\", \"Manual download\", \"Retry\"} default button \"Retry\")\nend run"
    } else {
        "on run argv\nreturn button returned of (display dialog (item 1 of argv) with title \"UltraVox recovery\" buttons {\"Quit\", \"Manual download\", \"Install final\"} default button \"Install final\")\nend run"
    };
    let output = std::process::Command::new("/usr/bin/osascript")
        .args(["-e", script, message]).output().map_err(|e| e.to_string())?;
    if !output.status.success() {
        return Err("Recovery dialog unavailable. Download https://github.com/michael-berardi/ultravox/releases/tag/v0.13.4 and relaunch the final app.".into());
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// True means the caller MUST return without constructing the application.
#[cfg(target_os = "macos")]
pub(crate) fn intercept(identifier: &str) -> bool {
    if identifier != BRIDGE_ID {
        return false;
    }
    let inspect = || std::env::var_os("HOME")
        .ok_or_else(|| "Cannot resolve your home directory.".to_string())
        .and_then(|home| needs_final(identifier, &Path::new(&home).join("Library/Application Support")))
        .map(|needed| needed || std::env::var_os("ULTRAVOX_DATA_DIR").is_some());
    let inspection = inspect();
    // ULTRAVOX_DATA_DIR is deliberately NOT a bypass: identity/grants still
    // belong to the original app, even with a user-selected data location.
    // A custom directory cannot establish prior identity, so use final recovery
    // conservatively even if neither default directory exists.
    if matches!(inspection, Ok(false)) {
        return false;
    }
    const CONSENT_MESSAGE: &str = "Your existing or user-selected UltraVox data needs the permanent original-identity application. This temporary bridge will not open stores or request permissions. Install the verified final update (0.13.4 or newer) now? Your settings, history, models and existing original-app grants stay in place.";
    let inspection_error = |error: &str| format!("{error}\nThis bridge cannot safely open. Retry inspection or download the final release manually. Your data has not been changed.");
    #[derive(PartialEq)]
    enum RecoveryStep { InspectRetry, InstallConsent, InstallRetry }
    let mut step = if inspection.is_err() { RecoveryStep::InspectRetry } else { RecoveryStep::InstallConsent };
    let mut message = match &inspection {
        Ok(_) => CONSENT_MESSAGE.to_string(),
        Err(error) => inspection_error(error),
    };
    loop {
        match dialog(&message, step != RecoveryStep::InstallConsent) {
            Ok(choice) if choice == "Manual download" => {
                let _ = std::process::Command::new("/usr/bin/open").arg(FINAL_DOWNLOAD).status();
                return true;
            }
            Ok(choice) if choice == "Retry" && step == RecoveryStep::InspectRetry => {
                // Inspection retry is NOT install consent. Recovered inspection
                // either resumes normal startup or presents the explicit offer.
                match inspect() {
                    Ok(false) => return false,
                    Ok(true) => {
                        message = CONSENT_MESSAGE.to_string();
                        step = RecoveryStep::InstallConsent;
                    }
                    Err(error) => message = inspection_error(&error),
                }
            }
            Ok(choice) if (choice == "Install final" && step == RecoveryStep::InstallConsent)
                || (choice == "Retry" && step == RecoveryStep::InstallRetry) => {
                // Only explicit consent (or retry after it) reaches the updater.
                let result = match inspect() {
                    Ok(false) => return false,
                    Ok(true) => tauri::async_runtime::block_on(crate::update::bootstrap_original_update()),
                    Err(error) => Err(error),
                };
                match result {
                    Ok(()) => return true, // helper waits for this process to exit
                    Err(error) => {
                        message = format!("Final update could not be installed: {error}\nNo stores were opened and no permissions requested. Retry or manually download v0.13.4, install it, and reopen UltraVox.");
                        step = RecoveryStep::InstallRetry;
                    }
                }
            }
            Ok(_) => return true,
            Err(error) => {
                eprintln!("{error}");
                // If native dialogs fail, a visible exact-final download page
                // remains usable rather than launching an empty bridge app.
                let _ = std::process::Command::new("/usr/bin/open").arg(FINAL_DOWNLOAD).status();
                return true;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn original_only_is_read_only_and_blocks_bridge_not_final_or_dev() {
        let root = tempfile::tempdir().unwrap();
        let original = root.path().join(ORIGINAL_ID);
        fs::create_dir(&original).unwrap();
        fs::write(original.join("settings.toml"), "automatic=false\nshortcut=custom").unwrap();
        fs::write(original.join("recordings.sqlite"), "history fixture").unwrap();
        fs::create_dir(original.join("models")).unwrap();
        assert!(needs_final(BRIDGE_ID, root.path()).unwrap());
        assert!(!needs_final(ORIGINAL_ID, root.path()).unwrap());
        assert!(!needs_final("com.imploselabs.ultravox.dev", root.path()).unwrap());
        assert!(!root.path().join(BRIDGE_ID).exists());
        assert_eq!(fs::read_to_string(original.join("settings.toml")).unwrap(), "automatic=false\nshortcut=custom");
        assert_eq!(fs::read_to_string(original.join("recordings.sqlite")).unwrap(), "history fixture");
        assert!(original.join("models").is_dir());
    }
    #[test]
    fn pareto_users_and_new_users_use_normal_path() {
        let root = tempfile::tempdir().unwrap();
        assert!(!needs_final(BRIDGE_ID, root.path()).unwrap());
        fs::create_dir(root.path().join(BRIDGE_ID)).unwrap();
        assert!(!needs_final(BRIDGE_ID, root.path()).unwrap());
        fs::create_dir(root.path().join(ORIGINAL_ID)).unwrap();
        assert!(!needs_final(BRIDGE_ID, root.path()).unwrap());
    }
    #[test]
    fn inspection_failure_blocks_instead_of_creating_empty_data() {
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join(ORIGINAL_ID), "not a directory").unwrap();
        assert!(needs_final(BRIDGE_ID, root.path()).is_err());
        assert!(!root.path().join(BRIDGE_ID).exists());
    }
    #[test]
    fn rollback_after_final_moves_pareto_data_is_guarded() {
        let root = tempfile::tempdir().unwrap();
        let pareto = root.path().join(BRIDGE_ID);
        let original = root.path().join(ORIGINAL_ID);
        fs::create_dir(&pareto).unwrap();
        fs::write(pareto.join("recordings.sqlite"), "history fixture").unwrap();
        // Final's rename happens before stores open; app-only rollback must
        // NOT reverse it or overwrite newly written final data.
        fs::rename(&pareto, &original).unwrap();
        assert!(needs_final(BRIDGE_ID, root.path()).unwrap());
        assert_eq!(fs::read_to_string(original.join("recordings.sqlite")).unwrap(), "history fixture");
        assert!(!pareto.exists());
    }
}
