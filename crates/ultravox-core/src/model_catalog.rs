use serde::{Deserialize, Serialize};

/// FluidAudio / ASR model family.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ModelFamily {
    Whisper,
    FluidAudio,
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

impl Default for ModelCatalog {
    fn default() -> Self {
        Self {
            models: vec![
                ModelEntry {
                    id: "fluidaudio-en-v2".to_string(),
                    family: ModelFamily::FluidAudio,
                    version: ModelVersion::V2,
                    name: "English (Parakeet v2)".to_string(),
                    description: "Optimized English transcription model (default).".to_string(),
                    url: "https://huggingface.co/fluidaudio/asr-en-v2/resolve/main/model.bin"
                        .to_string(),
                    filename: "fluidaudio-en-v2.bin".to_string(),
                    size_bytes: Some(464_470_016),
                    is_default: true,
                },
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
            ],
        }
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
    fn default_catalog_has_english_v2_default() {
        let catalog = ModelCatalog::default();
        let default = catalog.default_model().expect("default model should exist");
        assert_eq!(default.id, "fluidaudio-en-v2");
        assert!(matches!(default.version, ModelVersion::V2));
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
