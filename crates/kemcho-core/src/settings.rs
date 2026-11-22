use std::path::Path;

use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::models::DEFAULT_MODEL;
use crate::postprocess::Replacement;
use crate::{paths, postprocess};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ShortcutMode {
    /// Record while the shortcut is held.
    Hold,
    /// Press once to start, again to stop.
    Toggle,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub model: String,
    pub language: String,
    pub shortcut: String,
    pub shortcut_mode: ShortcutMode,
    pub microphone: Option<String>,
    /// Paste at the cursor; otherwise only copy to the clipboard.
    pub paste: bool,
    pub restore_clipboard: bool,
    pub history_days: u32,
    pub remove_fillers: bool,
    pub voice_commands: bool,
    pub replacements: Vec<Replacement>,
    pub vocabulary: Vec<String>,
    pub onboarded: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            model: DEFAULT_MODEL.into(),
            language: "en".into(),
            shortcut: "CommandOrControl+Shift+Space".into(),
            shortcut_mode: ShortcutMode::Hold,
            microphone: None,
            paste: true,
            restore_clipboard: true,
            history_days: 7,
            remove_fillers: true,
            voice_commands: true,
            replacements: Vec::new(),
            vocabulary: Vec::new(),
            onboarded: false,
        }
    }
}

impl Settings {
    /// Loads settings, never failing: a missing or unreadable file gives
    /// defaults, and each invalid field falls back to its default alone.
    pub fn load() -> Self {
        match paths::settings_file() {
            Ok(path) => Self::load_from(&path),
            Err(err) => {
                tracing::warn!("no settings location: {err:#}");
                Self::default()
            }
        }
    }

    pub fn load_from(path: &Path) -> Self {
        let Ok(text) = std::fs::read_to_string(path) else {
            return Self::default();
        };
        match serde_json::from_str::<Value>(&text) {
            Ok(Value::Object(saved)) => Self::merge(saved),
            Ok(_) => {
                tracing::warn!("{} is not a JSON object; using defaults", path.display());
                Self::default()
            }
            Err(err) => {
                tracing::warn!(
                    "{} is not valid JSON ({err}); using defaults",
                    path.display()
                );
                Self::default()
            }
        }
    }

    fn merge(saved: Map<String, Value>) -> Self {
        let Value::Object(mut merged) = serde_json::to_value(Self::default()).unwrap() else {
            unreachable!()
        };
        for (key, value) in saved {
            if !merged.contains_key(&key) {
                continue;
            }
            let previous = merged.insert(key.clone(), value);
            if serde_json::from_value::<Self>(Value::Object(merged.clone())).is_err() {
                tracing::warn!("settings: ignoring invalid `{key}`");
                if let Some(previous) = previous {
                    merged.insert(key, previous);
                }
            }
        }
        serde_json::from_value(Value::Object(merged)).unwrap_or_default()
    }

    pub fn save(&self) -> Result<()> {
        self.save_to(&paths::settings_file()?)
    }

    pub fn save_to(&self, path: &Path) -> Result<()> {
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(self)?)?;
        std::fs::rename(tmp, path)?;
        Ok(())
    }

    pub fn steps(&self) -> postprocess::Steps<'_> {
        postprocess::Steps {
            remove_fillers: self.remove_fillers,
            voice_commands: self.voice_commands,
            replacements: &self.replacements,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_fields_fall_back_alone() {
        let dir = std::env::temp_dir().join(format!("kemcho-settings-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("settings.json");
        std::fs::write(
            &path,
            r#"{"model":"small.en","history_days":"lots","paste":false,"unknown":1}"#,
        )
        .unwrap();
        let s = Settings::load_from(&path);
        assert_eq!(s.model, "small.en");
        assert!(!s.paste);
        assert_eq!(s.history_days, 7);

        std::fs::write(&path, "not json").unwrap();
        assert_eq!(Settings::load_from(&path), Settings::default());

        s.save_to(&path).unwrap();
        assert_eq!(Settings::load_from(&path), s);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
