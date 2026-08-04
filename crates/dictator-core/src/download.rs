use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use thiserror::Error;
use tokio::io::AsyncWriteExt;
use tokio::sync::oneshot;
use uuid::Uuid;

/// Errors from the model download subsystem.
#[derive(Debug, Error)]
pub enum DownloadError {
    #[error("download already in progress")]
    AlreadyInProgress,
    #[error("download not found: {0}")]
    NotFound(Uuid),
    #[error("download failed: {0}")]
    Failed(String),
    #[error("download cancelled")]
    Cancelled,
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("request error: {0}")]
    Request(#[from] reqwest::Error),
}

/// Progress state for an active or completed download.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DownloadState {
    Queued,
    Downloading,
    Completed,
    Cancelled,
    Failed,
}

/// Metadata and progress for a model download.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DownloadProgress {
    pub id: Uuid,
    pub model_id: String,
    pub state: DownloadState,
    pub bytes_total: Option<u64>,
    pub bytes_received: u64,
}

impl DownloadProgress {
    /// Fraction of the download completed, if total size is known.
    pub fn fraction(&self) -> Option<f64> {
        self.bytes_total.map(|total| {
            if total == 0 {
                0.0
            } else {
                (self.bytes_received as f64 / total as f64).clamp(0.0, 1.0)
            }
        })
    }
}

/// A model download request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelDownload {
    pub id: Uuid,
    pub model_id: String,
    pub url: String,
    pub destination: PathBuf,
}

/// Abstraction over an active download handle.
#[async_trait]
pub trait DownloadHandle: Send + Sync {
    /// Cancel the download.
    async fn cancel(&self) -> Result<(), DownloadError>;
    /// Current progress snapshot.
    async fn progress(&self) -> DownloadProgress;
}

/// Manages model downloads and on-disk cache under the configured models directory.
#[derive(Debug, Clone)]
pub struct DownloadManager {
    models_dir: PathBuf,
    client: reqwest::Client,
    downloads: Arc<Mutex<HashMap<Uuid, DownloadProgress>>>,
    cancel_tokens: Arc<Mutex<HashMap<Uuid, oneshot::Sender<()>>>>,
}

impl Default for DownloadManager {
    fn default() -> Self {
        Self::new()
    }
}

impl DownloadManager {
    /// Create a manager with a default temporary models directory.
    pub fn new() -> Self {
        Self::with_models_dir(std::env::temp_dir().join("dictator-models"))
    }

    /// Create a manager backed by the given models directory.
    pub fn with_models_dir(models_dir: PathBuf) -> Self {
        std::fs::create_dir_all(&models_dir).ok();
        Self {
            models_dir,
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(300))
                .build()
                .unwrap_or_default(),
            downloads: Arc::new(Mutex::new(HashMap::new())),
            cancel_tokens: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Path to the root models directory.
    pub fn models_dir(&self) -> &Path {
        &self.models_dir
    }

    /// Resolve a destination path against the models directory.
    ///
    /// Absolute paths are returned as-is. Relative paths are joined under the
    /// models directory. A leading `models/` component is stripped so that UI
    /// paths like `models/model.bin` map to the correct cache location.
    pub fn resolve_destination(&self, destination: impl AsRef<Path>) -> PathBuf {
        let destination = destination.as_ref();
        if destination.is_absolute() {
            return destination.to_path_buf();
        }
        let stripped = destination.strip_prefix("models").unwrap_or(destination);
        self.models_dir.join(stripped)
    }

    /// Check whether a model file is already present and non-empty in the cache.
    pub fn is_downloaded(&self, filename: impl AsRef<str>) -> bool {
        self.downloaded_path(&filename).is_some()
    }

    /// Return the local path for a cached model if it exists.
    pub fn downloaded_path(&self, filename: impl AsRef<str>) -> Option<PathBuf> {
        let path = self.models_dir.join(filename.as_ref());
        match std::fs::metadata(&path) {
            Ok(m) if m.is_file() && m.len() > 0 => Some(path),
            _ => None,
        }
    }

    /// Start a download and return an initial progress snapshot.
    ///
    /// This is the low-level registration API; it does not perform any network
    /// I/O. For real downloads, use [`Self::start_download`].
    pub fn start(&self, request: ModelDownload) -> Result<DownloadProgress, DownloadError> {
        let mut map = self.downloads.lock().unwrap();
        if map.values().any(|d| {
            d.model_id == request.model_id
                && matches!(d.state, DownloadState::Queued | DownloadState::Downloading)
        }) {
            return Err(DownloadError::AlreadyInProgress);
        }
        let progress = DownloadProgress {
            id: request.id,
            model_id: request.model_id,
            state: DownloadState::Queued,
            bytes_total: None,
            bytes_received: 0,
        };
        map.insert(request.id, progress.clone());
        Ok(progress)
    }

    /// Start a real streaming download and spawn it in the background.
    ///
    /// If the destination file already exists and is non-empty, the download is
    /// skipped and a completed progress snapshot is returned immediately.
    pub async fn start_download(
        &self,
        request: ModelDownload,
    ) -> Result<DownloadProgress, DownloadError> {
        let absolute_path = self.resolve_destination(&request.destination);

        // Skip if already cached.
        if let Some(metadata) = std::fs::metadata(&absolute_path).ok() {
            if metadata.is_file() && metadata.len() > 0 {
                let progress = DownloadProgress {
                    id: request.id,
                    model_id: request.model_id.clone(),
                    state: DownloadState::Completed,
                    bytes_total: Some(metadata.len()),
                    bytes_received: metadata.len(),
                };
                self.downloads
                    .lock()
                    .unwrap()
                    .insert(request.id, progress.clone());
                return Ok(progress);
            }
        }

        let progress = self.start(request.clone())?;
        let (tx, rx) = oneshot::channel::<()>();
        self.cancel_tokens.lock().unwrap().insert(request.id, tx);

        let downloads = self.downloads.clone();
        let cancel_tokens = self.cancel_tokens.clone();
        let client = self.client.clone();
        let url = request.url.clone();
        let model_id = request.model_id.clone();
        let id = request.id;

        tokio::spawn(async move {
            let result = Self::download_stream(
                client,
                url,
                absolute_path,
                id,
                model_id,
                downloads.clone(),
                rx,
            )
            .await;

            cancel_tokens.lock().unwrap().remove(&id);

            if let Err(ref e) = result {
                let mut map = downloads.lock().unwrap();
                if let Some(entry) = map.get_mut(&id) {
                    match e {
                        DownloadError::Cancelled => entry.state = DownloadState::Cancelled,
                        _ => entry.state = DownloadState::Failed,
                    }
                }
            }

            result
        });

        Ok(progress)
    }

    async fn download_stream(
        client: reqwest::Client,
        url: String,
        final_path: PathBuf,
        id: Uuid,
        model_id: String,
        downloads: Arc<Mutex<HashMap<Uuid, DownloadProgress>>>,
        mut cancel_rx: oneshot::Receiver<()>,
    ) -> Result<(), DownloadError> {
        let response = client
            .get(&url)
            .send()
            .await
            .map_err(DownloadError::Request)?;
        let total = response.content_length();

        {
            let mut map = downloads.lock().unwrap();
            let entry = map.get_mut(&id).ok_or(DownloadError::NotFound(id))?;
            entry.bytes_total = total;
            entry.model_id = model_id;
            entry.state = DownloadState::Downloading;
        }

        let part_path = final_path.with_extension("part");
        if let Some(parent) = part_path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }

        let mut file = tokio::fs::File::create(&part_path).await?;
        let mut received: u64 = 0;

        let mut response = response;
        while let Some(chunk) = response.chunk().await.map_err(DownloadError::Request)? {
            if cancel_rx.try_recv().is_ok() {
                {
                    let mut map = downloads.lock().unwrap();
                    if let Some(entry) = map.get_mut(&id) {
                        entry.state = DownloadState::Cancelled;
                    }
                }
                let _ = tokio::fs::remove_file(&part_path).await;
                return Err(DownloadError::Cancelled);
            }

            file.write_all(&chunk).await?;
            received += chunk.len() as u64;

            let mut map = downloads.lock().unwrap();
            if let Some(entry) = map.get_mut(&id) {
                entry.bytes_received = received;
            }
        }

        file.flush().await?;
        drop(file);

        tokio::fs::rename(&part_path, &final_path).await?;

        {
            let mut map = downloads.lock().unwrap();
            if let Some(entry) = map.get_mut(&id) {
                entry.state = DownloadState::Completed;
                entry.bytes_received = total.unwrap_or(received);
            }
        }

        Ok(())
    }

    /// Update progress for a download.
    pub fn update_progress(
        &self,
        id: Uuid,
        bytes_total: Option<u64>,
        bytes_received: u64,
    ) -> Result<(), DownloadError> {
        let mut map = self.downloads.lock().unwrap();
        let entry = map.get_mut(&id).ok_or(DownloadError::NotFound(id))?;
        entry.bytes_total = bytes_total;
        entry.bytes_received = bytes_received;
        if entry.state == DownloadState::Queued {
            entry.state = DownloadState::Downloading;
        }
        Ok(())
    }

    /// Mark a download as completed.
    pub fn complete(&self, id: Uuid) -> Result<(), DownloadError> {
        let mut map = self.downloads.lock().unwrap();
        let entry = map.get_mut(&id).ok_or(DownloadError::NotFound(id))?;
        entry.state = DownloadState::Completed;
        if entry.bytes_total.is_some() {
            entry.bytes_received = entry.bytes_total.unwrap();
        }
        Ok(())
    }

    /// Mark a download as failed.
    pub fn fail(&self, id: Uuid, reason: String) -> Result<(), DownloadError> {
        let mut map = self.downloads.lock().unwrap();
        let entry = map.get_mut(&id).ok_or(DownloadError::NotFound(id))?;
        entry.state = DownloadState::Failed;
        Err(DownloadError::Failed(reason))
    }

    /// Cancel a download.
    pub fn cancel(&self, id: Uuid) -> Result<(), DownloadError> {
        let mut map = self.downloads.lock().unwrap();
        let entry = map.get_mut(&id).ok_or(DownloadError::NotFound(id))?;
        if matches!(
            entry.state,
            DownloadState::Completed | DownloadState::Cancelled | DownloadState::Failed
        ) {
            return Ok(());
        }
        entry.state = DownloadState::Cancelled;

        if let Some(tx) = self.cancel_tokens.lock().unwrap().remove(&id) {
            let _ = tx.send(());
        }
        Ok(())
    }

    /// Get a snapshot of progress for a download.
    pub fn get(&self, id: Uuid) -> Option<DownloadProgress> {
        self.downloads.lock().unwrap().get(&id).cloned()
    }

    /// List all known downloads.
    pub fn list(&self) -> Vec<DownloadProgress> {
        self.downloads.lock().unwrap().values().cloned().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_request() -> ModelDownload {
        ModelDownload {
            id: Uuid::new_v4(),
            model_id: "fluidaudio-en-v2".to_string(),
            url: "https://example.com/model.bin".to_string(),
            destination: PathBuf::from("fluidaudio-en-v2.bin"),
        }
    }

    #[test]
    fn start_and_progress() {
        let manager = DownloadManager::new();
        let req = sample_request();
        let progress = manager.start(req).unwrap();
        assert_eq!(progress.state, DownloadState::Queued);
        manager.update_progress(progress.id, Some(100), 50).unwrap();
        let updated = manager.get(progress.id).unwrap();
        assert_eq!(updated.state, DownloadState::Downloading);
        assert_eq!(updated.fraction(), Some(0.5));
    }

    #[test]
    fn duplicate_download_rejected() {
        let manager = DownloadManager::new();
        let req = sample_request();
        manager.start(req.clone()).unwrap();
        let result = manager.start(req);
        assert!(matches!(result, Err(DownloadError::AlreadyInProgress)));
    }

    #[test]
    fn resolve_destination_strips_models_prefix() {
        let manager = DownloadManager::with_models_dir(PathBuf::from("/app/models"));
        assert_eq!(
            manager.resolve_destination("models/model.bin"),
            PathBuf::from("/app/models/model.bin")
        );
        assert_eq!(
            manager.resolve_destination("model.bin"),
            PathBuf::from("/app/models/model.bin")
        );
    }

    #[tokio::test]
    async fn skips_existing_download() {
        let models_dir = std::env::temp_dir().join(format!("dictator-test-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&models_dir).unwrap();
        let manager = DownloadManager::with_models_dir(models_dir.clone());
        let file_path = models_dir.join("existing.bin");
        std::fs::write(&file_path, b"cached").unwrap();

        let req = ModelDownload {
            id: Uuid::new_v4(),
            model_id: "existing".to_string(),
            url: "https://example.com/existing.bin".to_string(),
            destination: PathBuf::from("existing.bin"),
        };

        let progress = manager.start_download(req).await.unwrap();
        assert_eq!(progress.state, DownloadState::Completed);
        assert_eq!(progress.bytes_received, 6);
    }
}
