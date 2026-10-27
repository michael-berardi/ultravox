//! Download, verify and install Phonon-2; remove the models it replaces.

use super::PhononError;
use super::native::sha256_hex;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

pub const MODEL_DIR_NAME: &str = "phonon-2";
const ARCHIVE_URL: &str = "https://huggingface.co/FermionResearch/Phonon-2/resolve/main/phonon-2.bps.tar.zst";
const ARCHIVE_NAME: &str = "phonon-2.bps.tar.zst";
const ARCHIVE_SHA256: &str = "98125795b6dda72f5c6eee9ba33d19815df65dcb18b50a357bf9f73c9935309e";
const CONTAINER_SHA256: &str = "4b6bfa3a12cc3c4e0a54f2ab3ec4ca7a842b09e5c7ecfc8e7ca0ac6cc8c11468";
const CONTAINER_BYTES: u64 = 177_438_361;

static PROGRESS: AtomicU64 = AtomicU64::new(0);

/// Download/unpack progress in `[0, 1]` for the install in flight (or the last one).
pub fn install_progress() -> f64 {
    f64::from_bits(PROGRESS.load(Ordering::Relaxed))
}

fn set_progress(value: f64) {
    PROGRESS.store(value.clamp(0.0, 1.0).to_bits(), Ordering::Relaxed);
}

/// The model directory is complete (both files, right size). Hashes are verified at install time.
pub fn is_installed(models_dir: &Path) -> bool {
    let dir = models_dir.join(MODEL_DIR_NAME);
    std::fs::metadata(dir.join("model.fermion")).is_ok_and(|m| m.len() == CONTAINER_BYTES)
        && dir.join("config.json").is_file()
}

/// Append `[have, end)` of the archive to `part`, failing if no bytes arrive for 30 s.
async fn fetch_from(
    client: &reqwest::Client,
    part: &Path,
    have: u64,
    total: u64,
) -> Result<(), String> {
    let mut request = client.get(ARCHIVE_URL);
    if have > 0 {
        request = request.header(reqwest::header::RANGE, format!("bytes={have}-"));
    }
    let mut response = request
        .send()
        .await
        .and_then(|r| r.error_for_status())
        .map_err(|e| format!("GET {ARCHIVE_URL}: {e}"))?;
    let resumed = response.status() == reqwest::StatusCode::PARTIAL_CONTENT;
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .append(resumed)
        .truncate(!resumed)
        .open(part)
        .map_err(|e| format!("open {}: {e}", part.display()))?;
    let mut received = if resumed { have } else { 0 };
    loop {
        let chunk = tokio::time::timeout(std::time::Duration::from_secs(30), response.chunk())
            .await
            .map_err(|_| "no data for 30 s".to_string())?
            .map_err(|e| format!("connection error: {e}"))?;
        let Some(chunk) = chunk else { break };
        file.write_all(&chunk)
            .map_err(|e| format!("write {}: {e}", part.display()))?;
        received += chunk.len() as u64;
        set_progress(received as f64 / total as f64 * 0.9);
    }
    if received < total {
        return Err(format!("connection closed at {received} of {total} bytes"));
    }
    Ok(())
}

fn dl(msg: impl Into<String>) -> PhononError {
    PhononError::Download(msg.into())
}

/// Download (verified), unpack (verified) and activate Phonon-2 under `models_dir`.
pub async fn install(models_dir: PathBuf) -> Result<(), PhononError> {
    if is_installed(&models_dir) {
        set_progress(1.0);
        return Ok(());
    }
    set_progress(0.0);
    std::fs::create_dir_all(&models_dir).map_err(|e| dl(format!("create {}: {e}", models_dir.display())))?;
    let part = models_dir.join(format!("{ARCHIVE_NAME}.part"));
    let client = reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| dl(e.to_string()))?;
    // A stalled or dropped connection resumes from the bytes already on disk (Range request),
    // so a flaky network costs a retry, not the whole 164 MB. The hash covers the final file.
    let total = 163_515_201u64;
    let mut attempts = 0u32;
    loop {
        let have = std::fs::metadata(&part).map(|m| m.len()).unwrap_or(0);
        if have >= total {
            break;
        }
        match fetch_from(&client, &part, have, total).await {
            Ok(()) => break,
            Err(error) => {
                attempts += 1;
                if attempts >= 8 {
                    return Err(dl(format!(
                        "download failed after {attempts} attempts ({} of {total} bytes kept in {}): {error}",
                        std::fs::metadata(&part).map(|m| m.len()).unwrap_or(0),
                        part.display()
                    )));
                }
                eprintln!("Phonon-2 download interrupted ({error}); resuming (attempt {attempts})");
                tokio::time::sleep(std::time::Duration::from_secs(2 * attempts as u64)).await;
            }
        }
    }
    let digest = {
        let bytes = std::fs::read(&part).map_err(|e| dl(format!("read {}: {e}", part.display())))?;
        sha256_hex(&bytes)
    };
    if digest != ARCHIVE_SHA256 {
        let _ = std::fs::remove_file(&part);
        return Err(dl(format!(
            "archive sha256 {digest} does not match the pinned {ARCHIVE_SHA256}; deleted the download"
        )));
    }

    let staging = models_dir.join(format!("{MODEL_DIR_NAME}.partial"));
    let final_dir = models_dir.join(MODEL_DIR_NAME);
    let unpack = {
        let (part, staging) = (part.clone(), staging.clone());
        tokio::task::spawn_blocking(move || unpack(&part, &staging))
    };
    unpack.await.map_err(|e| dl(format!("unpack task failed: {e}")))??;
    let _ = std::fs::remove_dir_all(&final_dir);
    std::fs::rename(&staging, &final_dir)
        .map_err(|e| dl(format!("activate {}: {e}", final_dir.display())))?;
    let _ = std::fs::remove_file(&part);
    set_progress(1.0);
    Ok(())
}

fn unpack(archive: &Path, staging: &Path) -> Result<(), PhononError> {
    let _ = std::fs::remove_dir_all(staging);
    std::fs::create_dir_all(staging).map_err(|e| dl(format!("create {}: {e}", staging.display())))?;
    let file = std::fs::File::open(archive).map_err(|e| dl(format!("open {}: {e}", archive.display())))?;
    let decoder = zstd::stream::read::Decoder::new(file).map_err(|e| dl(format!("zstd: {e}")))?;
    let mut tar = tar::Archive::new(decoder);
    for entry in tar.entries().map_err(|e| dl(format!("tar: {e}")))? {
        let mut entry = entry.map_err(|e| dl(format!("tar entry: {e}")))?;
        let name = entry.path().map_err(|e| dl(e.to_string()))?.into_owned();
        // Only the three flat files the archive is documented to hold; nothing may escape `staging`.
        let flat = name.components().count() == 1
            && matches!(
                name.to_str(),
                Some("model.fermion" | "config.json" | "packed_manifest.json" | "bps_manifest.json")
            );
        if !flat {
            return Err(dl(format!("unexpected archive member {}", name.display())));
        }
        entry
            .unpack(staging.join(&name))
            .map_err(|e| dl(format!("unpack {}: {e}", name.display())))?;
    }
    let container = std::fs::read(staging.join("model.fermion"))
        .map_err(|e| dl(format!("model.fermion missing from archive: {e}")))?;
    let digest = sha256_hex(&container);
    if digest != CONTAINER_SHA256 || container.len() as u64 != CONTAINER_BYTES {
        let _ = std::fs::remove_dir_all(staging);
        return Err(dl(format!(
            "model.fermion sha256 {digest} does not match the pinned {CONTAINER_SHA256}"
        )));
    }
    Ok(())
}

/// Remove the English models Phonon-2 replaces (Parakeet v2, Whisper `base.en`) and any older
/// `phonon-*` generation. The caller decides when that is safe: either Phonon-2 is installed, or
/// the user does not use the English model. Returns the paths it removed.
pub fn cleanup_superseded(models_dir: &Path, fluid_audio_v2_dirs: &[PathBuf]) -> Vec<PathBuf> {
    let mut removed = Vec::new();
    let mut try_remove = |path: PathBuf| {
        let result = if path.is_dir() {
            std::fs::remove_dir_all(&path)
        } else {
            std::fs::remove_file(&path)
        };
        match result {
            Ok(()) => removed.push(path),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => eprintln!("could not remove superseded model {}: {e}", path.display()),
        }
    };
    for legacy in [
        "ggml-base.en.bin",
        "ggml-base.en.bin.part",
        "ggml-base.en.bin.partial",
        "fluidaudio-en-v2.bin",
        "fluidaudio-en-v2.bin.part",
        "parakeet-tdt-0.6b-v2",
    ] {
        let path = models_dir.join(legacy);
        if path.exists() {
            try_remove(path);
        }
    }
    // Legacy Whisper cache dir: drop its English files, then the dir itself once empty.
    if let Some(parent) = models_dir.parent() {
        let legacy = parent.join("whisper-models");
        if let Ok(entries) = std::fs::read_dir(&legacy) {
            for entry in entries.flatten() {
                if entry.file_name().to_string_lossy().starts_with("ggml-base.en") {
                    try_remove(entry.path());
                }
            }
            let _ = std::fs::remove_dir(&legacy); // only succeeds when empty
        }
    }
    for dir in fluid_audio_v2_dirs {
        if dir.exists() {
            try_remove(dir.clone());
        }
    }
    if let Ok(entries) = std::fs::read_dir(models_dir) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with("phonon-") && name != MODEL_DIR_NAME {
                try_remove(entry.path());
            }
        }
    }
    removed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cleanup_removes_replaced_english_models_only() {
        let root = std::env::temp_dir().join(format!("ultravox-phonon-test-{}", std::process::id()));
        let models = root.join("models");
        std::fs::create_dir_all(models.join("phonon-1")).unwrap();
        std::fs::create_dir_all(models.join(MODEL_DIR_NAME)).unwrap();
        std::fs::create_dir_all(root.join("whisper-models")).unwrap();
        std::fs::write(root.join("whisper-models/ggml-base.en.bin"), b"x").unwrap();
        for keep in ["ggml-base.bin", "fluidaudio-multilingual-v3.bin"] {
            std::fs::write(models.join(keep), b"x").unwrap();
        }
        for stale in ["ggml-base.en.bin", "fluidaudio-en-v2.bin"] {
            std::fs::write(models.join(stale), b"x").unwrap();
        }
        let v2_cache = root.join("FluidAudio/parakeet-tdt-0.6b-v2");
        std::fs::create_dir_all(&v2_cache).unwrap();

        let removed = cleanup_superseded(&models, std::slice::from_ref(&v2_cache));
        assert_eq!(removed.len(), 5, "{removed:?}");
        assert!(models.join("ggml-base.bin").exists());
        assert!(models.join("fluidaudio-multilingual-v3.bin").exists());
        assert!(models.join(MODEL_DIR_NAME).exists());
        assert!(!models.join("phonon-1").exists());
        assert!(!models.join("ggml-base.en.bin").exists());
        assert!(!root.join("whisper-models").exists());
        assert!(!v2_cache.exists());
        let _ = std::fs::remove_dir_all(&root);
    }
}
