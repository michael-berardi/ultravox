use serde::{Deserialize, Serialize};

/// Modifier-only hotkey selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ModifierKey {
    None,
    Command,
    Option,
    Control,
    Shift,
}

impl Default for ModifierKey {
    fn default() -> Self {
        ModifierKey::None
    }
}

impl AsRef<str> for ModifierKey {
    fn as_ref(&self) -> &str {
        match self {
            ModifierKey::None => "none",
            ModifierKey::Command => "command",
            ModifierKey::Option => "option",
            ModifierKey::Control => "control",
            ModifierKey::Shift => "shift",
        }
    }
}

impl TryFrom<&str> for ModifierKey {
    type Error = String;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "none" => Ok(ModifierKey::None),
            "command" => Ok(ModifierKey::Command),
            "option" => Ok(ModifierKey::Option),
            "control" => Ok(ModifierKey::Control),
            "shift" => Ok(ModifierKey::Shift),
            other => Err(format!("unknown modifier key: {other}")),
        }
    }
}

impl std::str::FromStr for ModifierKey {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::try_from(s)
    }
}

/// Global shortcut / recording trigger settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ShortcutSettings {
    pub modifier_only_hotkey: ModifierKey,
    pub key_combination: Option<String>,
    pub hold_to_record: bool,
    pub meeting_key_combination: String,
}

impl Default for ShortcutSettings {
    fn default() -> Self {
        Self {
            modifier_only_hotkey: ModifierKey::None,
            key_combination: Some("Option+Backtick".to_string()),
            hold_to_record: false,
            meeting_key_combination: "Control+M".to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modifier_roundtrip() {
        let modifier = ModifierKey::Command;
        let serialized = serde_json::to_string(&modifier).unwrap();
        assert_eq!(serialized, "\"command\"");
        let parsed: ModifierKey = serde_json::from_str(&serialized).unwrap();
        assert_eq!(parsed, ModifierKey::Command);
    }

    #[test]
    fn default_shortcut_settings_serialize() {
        let settings = ShortcutSettings::default();
        let json = serde_json::to_string(&settings).unwrap();
        assert!(json.contains("none"));
        assert!(json.contains("Option+Backtick"));
        assert!(json.contains("Control+M"));
    }
}
