//! One-time move of per-user state back from the identifier UltraVox 0.13.0
//! and 0.13.1 shipped under (com.paretocybernetics.ultravox).
//!
//! Tauri derives the app data directory from the bundle identifier and macOS
//! keys the preferences domain, the privacy (TCC) grants and keychain access
//! to it. Changing it made macOS ask for every permission again, so UltraVox
//! keeps com.imploselabs.ultravox for good and only its name is Pareto
//! Cybernetics. `run` executes before any store opens: it moves the 0.13.0/0.13.1
//! data directory back, rewrites the absolute paths stored inside it, imports
//! that preferences domain and logs every step.

use std::fs;
use std::io::{self, ErrorKind};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

// Identifier prefix UltraVox 0.13.0 and 0.13.1 shipped under.
const LEGACY_IDENTIFIER_PREFIX: &str = "com.paretocybernetics.";
const IDENTIFIER_PREFIX: &str = "com.imploselabs.";
const NOTICE_FILE: &str = "legacy-migration-v1.json";
/// Metadata files larger than this are not scanned for stale absolute paths.
const MAX_REBASED_FILE_BYTES: u64 = 8 * 1024 * 1024;
const MAX_SCAN_DEPTH: usize = 3;
/// Bulk user data that holds no path metadata.
const BULK_DIRS: &[&str] = &["models", "recordings"];

/// What the migration did, persisted in the data directory.
#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct Notice {
    version: u32,
    migrated_at: u64,
    actions: Vec<String>,
    /// Previous data directory whose absolute paths are still being rewritten.
    /// Kept until the rewrite succeeds so an interrupted run retries.
    rebase_from: Option<String>,
}

fn legacy_identifier(identifier: &str) -> Option<String> {
    identifier
        .strip_prefix(IDENTIFIER_PREFIX)
        .map(|rest| format!("{LEGACY_IDENTIFIER_PREFIX}{rest}"))
}

fn read_notice(path: &Path) -> Option<Notice> {
    serde_json::from_slice(&fs::read(path).ok()?).ok()
}

fn write_notice(path: &Path, notice: &Notice) -> Result<(), String> {
    let bytes = serde_json::to_vec_pretty(notice).map_err(|error| error.to_string())?;
    write_atomic(path, &bytes, None).map_err(|error| format!("{}: {error}", path.display()))
}

fn write_atomic(path: &Path, bytes: &[u8], like: Option<&fs::Metadata>) -> io::Result<()> {
    let mut temp = path.as_os_str().to_owned();
    temp.push(".migrate.tmp");
    let temp = Path::new(&temp);
    fs::write(temp, bytes)?;
    if let Some(metadata) = like {
        fs::set_permissions(temp, metadata.permissions())?;
    }
    fs::rename(temp, path)
}

/// Migrate once per launch; every step is logged and failures never block startup.
pub(crate) fn run(app: &AppHandle) {
    let identifier = app.config().identifier.clone();
    let Some(legacy) = legacy_identifier(&identifier) else {
        return;
    };
    let mut actions = Vec::new();

    // An explicit ULTRAVOX_DATA_DIR is the user's own location: nothing moves.
    if std::env::var_os("ULTRAVOX_DATA_DIR").is_none() {
        match app.path().app_data_dir() {
            Ok(current) => migrate_data_dir(&current, &legacy, &mut actions),
            Err(error) => eprintln!("[migrate] cannot resolve the app data directory: {error}"),
        }
    }

    #[cfg(target_os = "macos")]
    match migrate_preferences(&legacy, &identifier) {
        Ok(Some(action)) => actions.push(action),
        Ok(None) => {}
        Err(error) => eprintln!("[migrate] preferences: {error}"),
    }

    if !actions.is_empty() {
        for action in &actions {
            eprintln!("[migrate] {action}");
        }
        // Preferences-only migrations still need a notice, so record them where
        // the data directory would be.
        if let Ok(current) = app.path().app_data_dir() {
            record_actions(&current, actions);
        }
    }
}

fn record_actions(current: &Path, actions: Vec<String>) {
    let path = current.join(NOTICE_FILE);
    let mut notice = read_notice(&path).unwrap_or_default();
    notice.version = 1;
    notice.migrated_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs());
    notice.actions.extend(actions);
    if let Err(error) = fs::create_dir_all(current)
        .map_err(|e| e.to_string())
        .and_then(|_| write_notice(&path, &notice))
    {
        eprintln!("[migrate] could not record the migration notice: {error}");
    }
}

/// Move the previous data directory next to `current`, then finish any pending
/// rewrite of the absolute paths stored inside it.
fn migrate_data_dir(current: &Path, legacy_identifier: &str, actions: &mut Vec<String>) {
    let Some(parent) = current.parent() else {
        return;
    };
    let legacy_dir = parent.join(legacy_identifier);
    let notice_path = current.join(NOTICE_FILE);

    match move_data_dir(&legacy_dir, current) {
        Ok(Some(action)) => {
            actions.push(action);
            // Persist the pending rewrite before attempting it so a failure retries.
            let mut notice = read_notice(&notice_path).unwrap_or_default();
            notice.rebase_from = Some(legacy_dir.to_string_lossy().into_owned());
            if let Err(error) = write_notice(&notice_path, &notice) {
                eprintln!("[migrate] could not record the pending path rewrite: {error}");
            }
        }
        Ok(None) => {}
        Err(error) => {
            eprintln!("[migrate] data directory: {error}");
            return;
        }
    }

    let Some(mut notice) = read_notice(&notice_path) else {
        return;
    };
    let Some(from) = notice.rebase_from.clone() else {
        return;
    };
    match rebase_stored_paths(current, &from) {
        Ok(summary) => {
            actions.push(summary);
            notice.rebase_from = None;
            if let Err(error) = write_notice(&notice_path, &notice) {
                eprintln!("[migrate] could not clear the pending path rewrite: {error}");
            }
        }
        Err(error) => eprintln!(
            "[migrate] stored paths still point at {from}; will retry next launch: {error}"
        ),
    }
}

/// Rename (or merge) `legacy_dir` into `current`. `Ok(None)` means there was
/// nothing to migrate.
fn move_data_dir(legacy_dir: &Path, current: &Path) -> Result<Option<String>, String> {
    let metadata = match fs::symlink_metadata(legacy_dir) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("cannot inspect {}: {error}", legacy_dir.display())),
    };
    if !metadata.is_dir() {
        return Err(format!(
            "{} is not a plain directory; left untouched",
            legacy_dir.display()
        ));
    }

    let current_is_empty = match fs::read_dir(current) {
        Ok(mut entries) => entries.next().is_none(),
        Err(error) if error.kind() == ErrorKind::NotFound => true,
        Err(error) => return Err(format!("cannot inspect {}: {error}", current.display())),
    };
    if current_is_empty {
        match fs::remove_dir(current) {
            Ok(()) => {}
            Err(error) if error.kind() == ErrorKind::NotFound => {}
            Err(error) => return Err(format!("cannot replace {}: {error}", current.display())),
        }
        fs::rename(legacy_dir, current).map_err(|error| {
            format!(
                "cannot move {} to {}: {error}",
                legacy_dir.display(),
                current.display()
            )
        })?;
        return Ok(Some(format!(
            "moved {} to {}",
            legacy_dir.display(),
            current.display()
        )));
    }

    // Both exist: keep everything already in the new directory and bring over
    // only what is missing. The old directory is removed only if nothing
    // conflicted, so no data is ever dropped.
    let mut copied = 0usize;
    let mut kept = 0usize;
    merge_missing(legacy_dir, current, &mut copied, &mut kept).map_err(|error| {
        format!(
            "cannot merge {} into {}: {error}",
            legacy_dir.display(),
            current.display()
        )
    })?;
    if kept == 0 {
        fs::remove_dir_all(legacy_dir).map_err(|error| {
            format!("merged but cannot remove {}: {error}", legacy_dir.display())
        })?;
        Ok(Some(format!(
            "merged {copied} files from {} into {}",
            legacy_dir.display(),
            current.display()
        )))
    } else {
        Err(format!(
            "{} has {kept} entries that conflict with {}; kept both ({copied} files merged)",
            legacy_dir.display(),
            current.display()
        ))
    }
}

fn merge_missing(from: &Path, to: &Path, copied: &mut usize, kept: &mut usize) -> io::Result<()> {
    fs::create_dir_all(to)?;
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        let source = entry.path();
        let target = to.join(entry.file_name());
        let kind = entry.file_type()?;
        if kind.is_dir() {
            merge_missing(&source, &target, copied, kept)?;
        } else if kind.is_file() && fs::symlink_metadata(&target).is_err() {
            fs::copy(&source, &target)?;
            *copied += 1;
        } else if kind.is_file() && same_file_contents(&source, &target)? {
            // Already carried over by an earlier, interrupted merge.
        } else {
            *kept += 1;
        }
    }
    Ok(())
}

/// True when `target` is a regular file with exactly the bytes of `source`.
fn same_file_contents(source: &Path, target: &Path) -> io::Result<bool> {
    let target_meta = fs::symlink_metadata(target)?;
    if !target_meta.is_file() || target_meta.len() != fs::metadata(source)?.len() {
        return Ok(false);
    }
    let (mut a, mut b) = (io::BufReader::new(fs::File::open(source)?), io::BufReader::new(fs::File::open(target)?));
    let (mut x, mut y) = ([0u8; 64 * 1024], [0u8; 64 * 1024]);
    loop {
        let n = io::Read::read(&mut a, &mut x)?;
        if n == 0 {
            return Ok(true);
        }
        io::Read::read_exact(&mut b, &mut y[..n])?;
        if x[..n] != y[..n] {
            return Ok(false);
        }
    }
}

/// Rewrite absolute paths that still point into the previous data directory:
/// recording sources in the history database and path fields in JSON/TOML
/// metadata (settings, voice profiles).
fn rebase_stored_paths(current: &Path, previous: &str) -> Result<String, String> {
    let new_root = current.to_string_lossy().into_owned();
    let old_root = previous.trim_end_matches('/');
    let mut problems = Vec::new();

    let mut rows = 0usize;
    if current.join("recordings.sqlite").is_file() {
        match ultravox_core::RecordingHistory::new(current.to_path_buf())
            .and_then(|mut history| history.rebase_source_paths(old_root, &new_root))
        {
            Ok(changed) => rows = changed,
            Err(error) => problems.push(format!("recording history: {error}")),
        }
    }

    let mut files = 0usize;
    if old_root.contains(['"', '\\']) || new_root.contains(['"', '\\']) {
        problems.push("paths with quotes or backslashes cannot be rewritten in place".to_string());
    } else {
        rebase_text_files(current, 0, old_root, &new_root, &mut files, &mut problems);
    }

    if problems.is_empty() {
        Ok(format!(
            "rewrote stored paths: {rows} recording rows, {files} metadata files"
        ))
    } else {
        Err(problems.join("; "))
    }
}

fn rebase_text_files(
    directory: &Path,
    depth: usize,
    old_root: &str,
    new_root: &str,
    files: &mut usize,
    problems: &mut Vec<String>,
) {
    let entries = match fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) => {
            problems.push(format!("{}: {error}", directory.display()));
            return;
        }
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        if kind.is_dir() {
            let bulk = depth == 0 && BULK_DIRS.contains(&name.as_str());
            if !bulk && depth < MAX_SCAN_DEPTH {
                rebase_text_files(&path, depth + 1, old_root, new_root, files, problems);
            }
            continue;
        }
        let extension = path.extension().and_then(|value| value.to_str());
        if !kind.is_file()
            || name == NOTICE_FILE
            || !matches!(extension, Some("json") | Some("toml"))
        {
            continue;
        }
        match rebase_text_file(&path, old_root, new_root) {
            Ok(true) => *files += 1,
            Ok(false) => {}
            Err(error) => problems.push(format!("{}: {error}", path.display())),
        }
    }
}

fn rebase_text_file(path: &Path, old_root: &str, new_root: &str) -> io::Result<bool> {
    let metadata = fs::metadata(path)?;
    if metadata.len() > MAX_REBASED_FILE_BYTES {
        return Ok(false);
    }
    let Ok(text) = String::from_utf8(fs::read(path)?) else {
        return Ok(false);
    };
    let Some(rewritten) = replace_path_prefix(&text, old_root, new_root) else {
        return Ok(false);
    };
    write_atomic(path, rewritten.as_bytes(), Some(&metadata))?;
    Ok(true)
}

/// Replace `old_root` with `new_root` wherever it is a whole path prefix (not
/// merely the start of a longer name). `None` when nothing changed.
fn replace_path_prefix(text: &str, old_root: &str, new_root: &str) -> Option<String> {
    if old_root.is_empty() {
        return None;
    }
    let mut result = String::with_capacity(text.len());
    let mut rest = text;
    let mut changed = false;
    while let Some(at) = rest.find(old_root) {
        let end = at + old_root.len();
        let boundary = rest[end..].chars().next().map_or(true, |next| {
            !(next.is_alphanumeric() || matches!(next, '.' | '-' | '_'))
        });
        result.push_str(&rest[..at]);
        if boundary {
            result.push_str(new_root);
            changed = true;
        } else {
            result.push_str(old_root);
        }
        rest = &rest[end..];
    }
    result.push_str(rest);
    changed.then_some(result)
}

/// Import the old preferences domain into the new one through `defaults`
/// (which cooperates with cfprefsd), then drop the old domain.
#[cfg(target_os = "macos")]
fn migrate_preferences(legacy: &str, current: &str) -> Result<Option<String>, String> {
    use std::io::Write;
    use std::path::PathBuf;
    use std::process::{Command, Stdio};

    const DEFAULTS: &str = "/usr/bin/defaults";
    let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else {
        return Ok(None);
    };
    let directory = home.join("Library/Preferences");
    if !directory.join(format!("{legacy}.plist")).is_file() {
        return Ok(None);
    }
    if directory.join(format!("{current}.plist")).exists() {
        return Err(format!(
            "domain {current} already exists; left {legacy} in place"
        ));
    }

    let exported = Command::new(DEFAULTS)
        .args(["export", legacy, "-"])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .map_err(|error| format!("cannot run defaults export: {error}"))?;
    if !exported.status.success() {
        return Err(format!("defaults export {legacy} failed"));
    }
    let mut import = Command::new(DEFAULTS)
        .args(["import", current, "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| format!("cannot run defaults import: {error}"))?;
    import
        .stdin
        .take()
        .ok_or_else(|| "defaults import has no stdin".to_string())?
        .write_all(&exported.stdout)
        .map_err(|error| format!("cannot feed defaults import: {error}"))?;
    let imported = import
        .wait()
        .map_err(|error| format!("defaults import did not finish: {error}"))?;
    if !imported.success() {
        return Err(format!(
            "defaults import {current} failed; {legacy} left in place"
        ));
    }
    let deleted = Command::new(DEFAULTS)
        .args(["delete", legacy])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false);
    Ok(Some(if deleted {
        format!("moved preferences domain {legacy} to {current}")
    } else {
        format!("copied preferences domain {legacy} to {current}; old domain could not be deleted")
    }))
}

/// Explanation for the permission panel. Always `None`: the identifier never
/// changes again, so existing macOS permissions stay granted.
pub(crate) fn permission_notice(_data_dir: &Path) -> Option<String> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(path: &Path, text: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    #[test]
    fn legacy_identifier_swaps_only_the_brand_prefix() {
        assert_eq!(
            legacy_identifier("com.imploselabs.ultravox").as_deref(),
            Some("com.paretocybernetics.ultravox")
        );
        assert_eq!(
            legacy_identifier("com.imploselabs.ultravox.dev").as_deref(),
            Some("com.paretocybernetics.ultravox.dev")
        );
        // The Pareto bridge must not move data back before the final original-ID build.
        assert_eq!(legacy_identifier("com.paretocybernetics.ultravox"), None);
        assert_eq!(legacy_identifier("com.libertydesignstudio.ultraterm"), None);
        assert!(permission_notice(Path::new("unused")).is_none());
    }

    #[test]
    fn path_prefix_replacement_respects_name_boundaries() {
        let old = "/Users/a/Library/Application Support/com.old.app";
        let new = "/Users/a/Library/Application Support/com.new.app";
        let text = format!(
            "models_directory = \"{old}/models\"\nother = \"{old}.dev/models\"\nbare = \"{old}\"\n"
        );
        let rewritten = replace_path_prefix(&text, old, new).unwrap();
        assert!(rewritten.contains(&format!("\"{new}/models\"")));
        assert!(rewritten.contains(&format!("\"{new}\"")));
        assert!(rewritten.contains(&format!("\"{old}.dev/models\"")));
        assert!(replace_path_prefix("nothing here", old, new).is_none());
    }

    #[test]
    fn moves_the_directory_and_rewrites_stored_paths() {
        let root = tempfile::tempdir().unwrap();
        let legacy = root.path().join("com.old.app");
        let current = root.path().join("com.new.app");
        let old_models = legacy.join("models");
        write(
            &legacy.join("settings.toml"),
            &format!("models_directory = \"{}\"\n", old_models.display()),
        );
        write(
            &legacy.join("voices.json"),
            &format!("[\"{}/voices/a.bin\"]", legacy.display()),
        );
        write(&legacy.join("models/ggml.bin"), "weights");
        write(&legacy.join("recordings/a.wav"), "audio");

        let mut actions = Vec::new();
        migrate_data_dir(&current, "com.old.app", &mut actions);

        assert!(!legacy.exists());
        assert_eq!(
            fs::read_to_string(current.join("models/ggml.bin")).unwrap(),
            "weights"
        );
        let settings = fs::read_to_string(current.join("settings.toml")).unwrap();
        assert!(settings.contains(&current.join("models").display().to_string()));
        assert!(!settings.contains("com.old.app"));
        let voices = fs::read_to_string(current.join("voices.json")).unwrap();
        assert!(voices.contains(&current.display().to_string()));
        assert_eq!(actions.len(), 2, "{actions:?}");
        let notice = read_notice(&current.join(NOTICE_FILE)).unwrap();
        assert!(notice.rebase_from.is_none());

        // A second launch finds nothing to do.
        let mut again = Vec::new();
        migrate_data_dir(&current, "com.old.app", &mut again);
        assert!(again.is_empty());
    }

    #[test]
    fn merges_without_overwriting_and_never_drops_conflicting_data() {
        let root = tempfile::tempdir().unwrap();
        let legacy = root.path().join("com.old.app");
        let current = root.path().join("com.new.app");
        write(&legacy.join("only-old.txt"), "old");
        write(&legacy.join("shared.txt"), "old shared");
        write(&current.join("shared.txt"), "new shared");

        let error = move_data_dir(&legacy, &current).unwrap_err();
        assert!(error.contains("conflict"), "{error}");
        assert_eq!(
            fs::read_to_string(current.join("shared.txt")).unwrap(),
            "new shared"
        );
        assert_eq!(
            fs::read_to_string(current.join("only-old.txt")).unwrap(),
            "old"
        );
        assert_eq!(
            fs::read_to_string(legacy.join("shared.txt")).unwrap(),
            "old shared"
        );

        fs::remove_file(legacy.join("shared.txt")).unwrap();
        let action = move_data_dir(&legacy, &current).unwrap().unwrap();
        assert!(action.starts_with("merged"), "{action}");
        assert!(!legacy.exists());
    }

    #[test]
    fn a_missing_legacy_directory_is_not_an_error() {
        let root = tempfile::tempdir().unwrap();
        assert!(
            move_data_dir(&root.path().join("absent"), &root.path().join("new"))
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn permission_notice_never_asks_to_grant_again() {
        let root = tempfile::tempdir().unwrap();
        record_actions(root.path(), vec!["moved the data directory".to_string()]);
        assert!(permission_notice(root.path()).is_none());
    }
}
