use serde::{Deserialize, Serialize};

/// ASR model family.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ModelFamily {
    Whisper,
    FluidAudio,
    /// Phonon-2 (English), run by the in-process native CPU engine.
    Phonon,
}

/// Logical model version identifier.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ModelVersion {
    V2,
    V3,
}

impl Default for ModelVersion {
    fn default() -> Self {
        ModelVersion::V2
    }
}

impl AsRef<str> for ModelVersion {
    fn as_ref(&self) -> &str {
        match self {
            ModelVersion::V2 => "v2",
            ModelVersion::V3 => "v3",
        }
    }
}

/// A single entry in the model catalog.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelEntry {
    pub id: String,
    pub family: ModelFamily,
    pub version: ModelVersion,
    pub name: String,
    pub description: String,
    pub url: String,
    pub filename: String,
    pub size_bytes: Option<u64>,
    pub is_default: bool,
}

/// In-memory catalog of downloadable / selectable models.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelCatalog {
    pub models: Vec<ModelEntry>,
}

/// The English model on every platform: Phonon-2 (164 MB, CC-BY-4.0).
fn phonon_entry() -> ModelEntry {
    ModelEntry {
        id: "phonon-2-en".to_string(),
        family: ModelFamily::Phonon,
        version: ModelVersion::V2,
        name: "English (Phonon-2)".to_string(),
        description: "Fast, accurate English transcription in a 164 MB download (default)."
            .to_string(),
        url: "https://huggingface.co/FermionResearch/Phonon-2/resolve/main/phonon-2.bps.tar.zst"
            .to_string(),
        filename: crate::phonon::MODEL_DIR_NAME.to_string(),
        size_bytes: Some(163_515_201),
        is_default: true,
    }
}

impl Default for ModelCatalog {
    fn default() -> Self {
        #[cfg(target_os = "macos")]
        let models = vec![
            phonon_entry(),
            ModelEntry {
                id: "fluidaudio-multilingual-v3".to_string(),
                family: ModelFamily::FluidAudio,
                version: ModelVersion::V3,
                name: "Multilingual (Parakeet v3)".to_string(),
                description: "Multilingual transcription model supporting 100+ languages."
                    .to_string(),
                url: "https://huggingface.co/fluidaudio/asr-multilingual-v3/resolve/main/model.bin"
                    .to_string(),
                filename: "fluidaudio-multilingual-v3.bin".to_string(),
                size_bytes: Some(483_311_616),
                is_default: false,
            },
        ];
        #[cfg(not(target_os = "macos"))]
        let models = vec![
            phonon_entry(),
            ModelEntry {
                id: "whisper-base-multilingual".to_string(),
                family: ModelFamily::Whisper,
                version: ModelVersion::V2,
                name: "Multilingual (Whisper base)".to_string(),
                description: "Multilingual transcription for Windows and Linux.".to_string(),
                url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base.bin"
                    .to_string(),
                filename: "ggml-base.bin".to_string(),
                size_bytes: Some(147_951_465),
                is_default: false,
            },
        ];
        Self { models }
    }
}

impl ModelCatalog {
    /// Find the default model entry.
    pub fn default_model(&self) -> Option<&ModelEntry> {
        self.models.iter().find(|m| m.is_default)
    }

    /// Find a model by its stable identifier.
    pub fn get(&self, id: &str) -> Option<&ModelEntry> {
        self.models.iter().find(|m| m.id == id)
    }

    /// List all models for a given family.
    pub fn by_family(&self, family: ModelFamily) -> Vec<&ModelEntry> {
        self.models.iter().filter(|m| m.family == family).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_catalog_has_phonon_english_default() {
        let catalog = ModelCatalog::default();
        let default = catalog.default_model().expect("default model should exist");
        assert_eq!(default.id, "phonon-2-en");
        assert_eq!(default.family, ModelFamily::Phonon);
        assert!(catalog.get("fluidaudio-en-v2").is_none());
        assert!(catalog.get("whisper-base-en").is_none());
    }

    #[test]
    fn catalog_includes_multilingual_v3() {
        let catalog = ModelCatalog::default();
        let m = catalog
            .get("fluidaudio-multilingual-v3")
            .expect("v3 model should exist");
        assert!(matches!(m.version, ModelVersion::V3));
    }
}
