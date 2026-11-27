//! Bounded lifetimes for everything UltraVox writes that is not your own data.
//!
//! Every temporary, partial or rollback file has an owner that removes it on
//! the normal path (drop guards, the install helper, import cleanup). This
//! sweep is the backstop for a crash, a force quit or a power cut: shortly
//! after launch, leftovers older than their lifetime are deleted. PRIVACY.md
//! lists each write and its lifetime; keep the two in step.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

/// Temporary work that never outlives the operation that made it: update
/// staging, Retex output and atomic-write temporaries.
pub const TEMP_MAX_AGE: Duration = Duration::from_secs(60 * 60);
/// Interrupted model downloads and development sandboxes.
pub const PARTIAL_MAX_AGE: Duration = Duration::from_secs(24 * 60 * 60);
/// The sweep and rollback cleanup run once this version has been up this long.
/// It is the rollback grace, so one timer covers both.
pub const SETTLE_DELAY: Duration = crate::update::ROLLBACK_COPY_GRACE;

#[derive(Clone, Copy, Debug)]
pub struct Rule {
    pub prefix: &'static str,
    pub suffix: &'static str,
    pub max_age: Duration,
    /// The whole name must equal `prefix`; nothing may follow it.
    pub exact: bool,
}

impl Rule {
    const fn new(prefix: &'static str, suffix: &'static str, max_age: Duration) -> Self {
        Self {
            prefix,
            suffix,
            max_age,
            exact: false,
        }
    }

    /// One fixed UltraVox-owned name. Use this wherever the directory can be
    /// one the user owns (`ULTRAVOX_DATA_DIR`, a chosen models folder): a
    /// wildcard there would delete files UltraVox never wrote.
    const fn exact(name: &'static str, max_age: Duration) -> Self {
        Self {
            prefix: name,
            suffix: "",
            max_age,
            exact: true,
        }
    }

    fn matches(&self, name: &str) -> bool {
        if self.exact {
            return name == self.prefix;
        }
        name.len() >= self.prefix.len() + self.suffix.len()
            && name.starts_with(self.prefix)
            && name.ends_with(self.suffix)
    }
}

/// Leftovers in the system temporary directory.
pub const TEMP_RULES: &[Rule] = &[
    Rule::new("ultravox-update-", "", TEMP_MAX_AGE),
    Rule::new("ultravox-retex-", ".stdout", TEMP_MAX_AGE),
    Rule::new("ultravox-retex-", ".stderr", TEMP_MAX_AGE),
    Rule::new("ultravox-import-smoke-", ".wav", TEMP_MAX_AGE),
    Rule::new("ultravox-control-models-", "", PARTIAL_MAX_AGE),
    Rule::new("ultravox-control-shortcut-", "", PARTIAL_MAX_AGE),
    Rule::new("ultravox-mirror-", "", PARTIAL_MAX_AGE),
];

/// Atomic-write temporaries in the app data directory. That directory can be
/// one the user chose with `ULTRAVOX_DATA_DIR`, so the migration temporaries
/// are named exactly, never by a `*.migrate.tmp` pattern: only the files the
/// migration writer itself creates for UltraVox's own metadata get swept.
pub const DATA_RULES: &[Rule] = &[
    Rule::new("settings.toml.tmp-", "", TEMP_MAX_AGE),
    Rule::new("update-preferences.json", ".tmp", TEMP_MAX_AGE),
    Rule::new("update-check.json", ".tmp", TEMP_MAX_AGE),
    Rule::exact("legacy-migration-v1.json.migrate.tmp", TEMP_MAX_AGE),
    Rule::exact("settings.toml.migrate.tmp", TEMP_MAX_AGE),
    Rule::exact("update-preferences.json.migrate.tmp", TEMP_MAX_AGE),
    Rule::exact("update-check.json.migrate.tmp", TEMP_MAX_AGE),
];

/// Atomic-write temporaries in the platform cache directory.
pub const CACHE_RULES: &[Rule] = &[Rule::new("update-check.json", ".tmp", TEMP_MAX_AGE)];

/// Exact UltraVox-created leftovers in the models directory. The user can
/// point that directory at a shared folder, so a wildcard `*.part`/
/// `*.partial`/`*.tmp` sweep would delete other people's work.
/// Every entry is an exact name: `ggml-base.bin.part.keep` or
/// `phonon-2-mine.partial` in a shared folder is never touched. The last two
/// are the names builds before 0.13.7 used for the same interrupted downloads
/// (the extension was replaced instead of appended).
pub const MODEL_RULES: &[Rule] = &[
    Rule::exact("phonon-2.bps.tar.zst.part", PARTIAL_MAX_AGE),
    Rule::exact("phonon-2.partial", PARTIAL_MAX_AGE),
    Rule::exact("fluidaudio-multilingual-v3.bin.part", PARTIAL_MAX_AGE),
    Rule::exact("ggml-base.bin.part", PARTIAL_MAX_AGE),
    Rule::exact("fluidaudio-multilingual-v3.part", PARTIAL_MAX_AGE),
    Rule::exact("ggml-base.part", PARTIAL_MAX_AGE),
];

/// Voice Studio catalog temporaries (official Pro builds).
pub const VOICE_STUDIO_RULES: &[Rule] = &[Rule::new("voices.", ".tmp", TEMP_MAX_AGE)];

/// Removes direct children of `dir` that match a rule and were last modified
/// longer ago than its lifetime. Symlinks are removed as links, never
/// followed. An entry from the future (clock moved back) is left alone.
pub fn sweep(dir: &Path, rules: &[Rule], now: SystemTime) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut removed = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        let Some(rule) = rules.iter().find(|rule| rule.matches(name)) else {
            continue;
        };
        let Ok(metadata) = fs::symlink_metadata(entry.path()) else {
            continue;
        };
        let expired = metadata
            .modified()
            .ok()
            .and_then(|modified| now.duration_since(modified).ok())
            .is_some_and(|age| age >= rule.max_age);
        if !expired {
            continue;
        }
        let path = entry.path();
        let result = if metadata.is_dir() {
            fs::remove_dir_all(&path)
        } else {
            fs::remove_file(&path)
        };
        if result.is_ok() {
            removed.push(path);
        }
    }
    removed
}

/// Where the sweep looks. Missing directories are skipped.
#[derive(Debug, Default)]
pub struct Locations {
    pub temp: Option<PathBuf>,
    pub data: Option<PathBuf>,
    pub cache: Option<PathBuf>,
    pub models: Option<PathBuf>,
    pub voice_studio: Option<PathBuf>,
}

pub fn sweep_all(locations: &Locations, now: SystemTime) -> Vec<PathBuf> {
    let mut removed = Vec::new();
    for (dir, rules) in [
        (&locations.temp, TEMP_RULES),
        (&locations.data, DATA_RULES),
        (&locations.cache, CACHE_RULES),
        (&locations.models, MODEL_RULES),
        (&locations.voice_studio, VOICE_STUDIO_RULES),
    ] {
        if let Some(dir) = dir {
            removed.extend(sweep(dir, rules, now));
        }
    }
    removed
}

pub fn locations(app: &tauri::AppHandle) -> Locations {
    use tauri::Manager;
    let data = crate::state::data_dir(app).ok();
    let models = app
        .try_state::<crate::state::AppState>()
        .and_then(|state| {
            state
                .config
                .lock()
                .ok()
                .and_then(|config| config.get().models_directory.clone())
        })
        .or_else(|| data.as_ref().map(|dir| dir.join("models")));
    Locations {
        temp: Some(std::env::temp_dir()),
        cache: app.path().app_cache_dir().ok(),
        voice_studio: data.as_ref().map(|dir| dir.join("voice-studio")),
        models,
        data,
    }
}

/// Runs once, [`SETTLE_DELAY`] after launch: removes rollback copies of the
/// previous version and expired leftovers. Best effort and silent apart from
/// a one-line count on stderr.
pub fn spawn_settled_cleanup(app: tauri::AppHandle) {
    std::thread::spawn(move || {
        std::thread::sleep(SETTLE_DELAY);
        crate::update::remove_rollback_copies();
        let removed = sweep_all(&locations(&app), SystemTime::now());
        if !removed.is_empty() {
            eprintln!("Removed {} expired temporary file(s).", removed.len());
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn age(path: &Path, seconds: u64) {
        let when = SystemTime::now() - Duration::from_secs(seconds);
        let file = fs::OpenOptions::new()
            .write(true)
            .open(path)
            .or_else(|_| fs::File::open(path))
            .unwrap();
        file.set_modified(when).unwrap();
    }

    fn dir_with(root: &Path, name: &str, seconds: u64) -> PathBuf {
        let path = root.join(name);
        fs::create_dir_all(path.join("unpacked")).unwrap();
        fs::write(path.join("unpacked/payload"), b"x").unwrap();
        age(&path, seconds);
        path
    }

    fn file_with(root: &Path, name: &str, seconds: u64) -> PathBuf {
        let path = root.join(name);
        fs::write(&path, b"x").unwrap();
        age(&path, seconds);
        path
    }

    #[test]
    fn expired_temp_leftovers_are_removed_and_fresh_or_unrelated_ones_kept() {
        let root = tempfile::tempdir().unwrap();
        let hour = TEMP_MAX_AGE.as_secs();
        let old_staging = dir_with(root.path(), "ultravox-update-1111", hour + 5);
        let live_staging = dir_with(root.path(), "ultravox-update-2222", 30);
        let old_stdout = file_with(root.path(), "ultravox-retex-abc.stdout", hour + 5);
        let old_stderr = file_with(root.path(), "ultravox-retex-abc.stderr", hour + 5);
        let fresh_mirror = dir_with(root.path(), "ultravox-mirror-aaaa", hour + 5);
        let old_mirror = dir_with(
            root.path(),
            "ultravox-mirror-bbbb",
            PARTIAL_MAX_AGE.as_secs() + 5,
        );
        let unrelated = dir_with(root.path(), "someone-else-update-1111", hour * 48);
        let bare_prefix = file_with(root.path(), "ultravox-retex-", hour * 48);

        let removed = sweep(root.path(), TEMP_RULES, SystemTime::now());

        assert_eq!(removed.len(), 4, "{removed:?}");
        for gone in [&old_staging, &old_stdout, &old_stderr, &old_mirror] {
            assert!(!gone.exists(), "{gone:?} should be removed");
        }
        for kept in [&live_staging, &fresh_mirror, &unrelated, &bare_prefix] {
            assert!(kept.exists(), "{kept:?} should be kept");
        }
    }

    #[test]
    fn interrupted_downloads_and_atomic_temporaries_have_bounded_lifetimes() {
        let root = tempfile::tempdir().unwrap();
        let data = root.path().join("data");
        let cache = root.path().join("cache");
        let models = data.join("models");
        let voices = data.join("voice-studio");
        for dir in [&data, &cache, &models, &voices] {
            fs::create_dir_all(dir).unwrap();
        }
        let day = PARTIAL_MAX_AGE.as_secs();
        let hour = TEMP_MAX_AGE.as_secs();
        let stale_part = file_with(&models, "ggml-base.bin.part", day + 5);
        let active_part = file_with(&models, "ggml-medium.part", 10);
        let stale_staging = dir_with(&models, "phonon-2.partial", day + 5);
        let model = file_with(&models, "ggml-large-v3.bin", day * 30);
        let settings_tmp = file_with(&data, "settings.toml.tmp-1234", hour + 5);
        let settings = file_with(&data, "settings.toml", day * 30);
        let prefs_tmp = file_with(&data, "update-preferences.json.tmp", hour + 5);
        let migrate_tmp = file_with(&data, "legacy-migration-v1.json.migrate.tmp", hour + 5);
        let history = file_with(&data, "recordings.sqlite", day * 30);
        let check_tmp = file_with(&cache, "update-check.json.tmp", hour + 5);
        let check = file_with(&cache, "update-check.json", day * 30);
        let voices_tmp = file_with(&voices, "voices.9f.tmp", hour + 5);
        let voices_json = file_with(&voices, "voices.json", day * 30);

        let removed = sweep_all(
            &Locations {
                temp: None,
                data: Some(data.clone()),
                cache: Some(cache.clone()),
                models: Some(models.clone()),
                voice_studio: Some(voices.clone()),
            },
            SystemTime::now(),
        );

        for gone in [
            &stale_part,
            &stale_staging,
            &settings_tmp,
            &prefs_tmp,
            &migrate_tmp,
            &check_tmp,
            &voices_tmp,
        ] {
            assert!(!gone.exists(), "{gone:?} should be removed");
        }
        // User data and the single bounded cache file are never swept.
        for kept in [
            &active_part,
            &model,
            &settings,
            &history,
            &check,
            &voices_json,
            &models,
            &voices,
        ] {
            assert!(kept.exists(), "{kept:?} should be kept");
        }
        assert_eq!(removed.len(), 7, "{removed:?}");
    }

    #[test]
    fn user_chosen_data_directory_keeps_unrelated_migrate_temporaries() {
        // The data directory may be a folder the user owns (`ULTRAVOX_DATA_DIR`),
        // so a `*.migrate.tmp` wildcard would delete their files.
        let root = tempfile::tempdir().unwrap();
        let data = root.path().join("my-data");
        fs::create_dir_all(&data).unwrap();
        let hour = TEMP_MAX_AGE.as_secs();

        let app_notice = file_with(&data, "legacy-migration-v1.json.migrate.tmp", hour + 5);
        let app_settings = file_with(&data, "settings.toml.migrate.tmp", hour + 5);
        let user_plain = file_with(&data, "notes.migrate.tmp", hour + 5);
        let user_json = file_with(&data, "my-notes.json.migrate.tmp", hour + 5);
        let user_toml = file_with(&data, "project.toml.migrate.tmp", hour + 5);
        let user_similar = file_with(&data, "settings.toml.migrate.tmp.keep", hour + 5);

        let removed = sweep_all(
            &Locations {
                data: Some(data.clone()),
                ..Locations::default()
            },
            SystemTime::now(),
        );

        let mut removed = removed;
        removed.sort();
        let mut expected = vec![app_notice.clone(), app_settings.clone()];
        expected.sort();
        assert_eq!(removed, expected);
        for gone in [&app_notice, &app_settings] {
            assert!(!gone.exists(), "{gone:?} should be removed");
        }
        for kept in [&user_plain, &user_json, &user_toml, &user_similar] {
            assert!(kept.exists(), "{kept:?} was not written by UltraVox");
        }
    }

    #[test]
    fn shared_models_directory_keeps_unrelated_partials() {
        let root = tempfile::tempdir().unwrap();
        let models = root.path().join("shared-models");
        fs::create_dir_all(&models).unwrap();
        let day = PARTIAL_MAX_AGE.as_secs();

        let app_archive = file_with(&models, "phonon-2.bps.tar.zst.part", day + 5);
        let app_staging = dir_with(&models, "phonon-2.partial", day + 5);
        let app_multilingual = file_with(&models, "fluidaudio-multilingual-v3.bin.part", day + 5);
        let app_whisper = file_with(&models, "ggml-base.bin.part", day + 5);
        let shared_part = file_with(&models, "another-app-model.part", day + 5);
        let shared_partial = dir_with(&models, "another-app-model.partial", day + 5);
        let shared_tmp = file_with(&models, "another-app-model.tmp", day + 5);
        let legacy_whisper = file_with(&models, "ggml-base.part", day + 5);
        let legacy_multilingual = file_with(&models, "fluidaudio-multilingual-v3.part", day + 5);
        let lookalike_part = file_with(&models, "ggml-base.bin.x.part", day + 5);
        let lookalike_suffix = file_with(&models, "ggml-base.bin.part.keep", day + 5);
        let lookalike_staging = dir_with(&models, "phonon-2-mine.partial", day + 5);

        let removed = sweep_all(
            &Locations {
                models: Some(models.clone()),
                ..Locations::default()
            },
            SystemTime::now(),
        );

        let mut removed = removed;
        removed.sort();
        let mut expected = vec![
            app_archive.clone(),
            app_staging.clone(),
            app_multilingual.clone(),
            app_whisper.clone(),
            legacy_whisper.clone(),
            legacy_multilingual.clone(),
        ];
        expected.sort();
        assert_eq!(removed, expected);
        for gone in [&app_archive, &app_staging, &app_multilingual, &app_whisper] {
            assert!(!gone.exists(), "{gone:?} should be removed");
        }
        for kept in [
            &shared_part,
            &shared_partial,
            &shared_tmp,
            &lookalike_part,
            &lookalike_suffix,
            &lookalike_staging,
        ] {
            assert!(kept.exists(), "{kept:?} in a shared folder should be kept");
        }
    }

    #[test]
    fn missing_directories_and_future_timestamps_are_harmless() {
        let root = tempfile::tempdir().unwrap();
        assert!(sweep(&root.path().join("missing"), TEMP_RULES, SystemTime::now()).is_empty());
        let staging = dir_with(root.path(), "ultravox-update-3333", 0);
        let past = SystemTime::now() - Duration::from_secs(PARTIAL_MAX_AGE.as_secs() * 2);
        assert!(sweep(root.path(), TEMP_RULES, past).is_empty());
        assert!(staging.exists());
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_leftovers_are_unlinked_without_following_them() {
        let root = tempfile::tempdir().unwrap();
        let outside = root.path().join("outside");
        fs::create_dir(&outside).unwrap();
        fs::write(outside.join("keep"), b"x").unwrap();
        let temp = root.path().join("temp");
        fs::create_dir(&temp).unwrap();
        let link = temp.join("ultravox-update-link");
        std::os::unix::fs::symlink(&outside, &link).unwrap();
        let later = SystemTime::now() + TEMP_MAX_AGE + Duration::from_secs(5);
        let removed = sweep(&temp, TEMP_RULES, later);
        assert_eq!(removed, vec![link.clone()]);
        assert!(fs::symlink_metadata(&link).is_err());
        assert!(outside.join("keep").exists());
    }
}
