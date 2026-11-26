use std::path::Path;

use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::models::DEFAULT_MODEL;
use crate::postprocess::Replacement;
use crate::{agent, languages, models, paths, postprocess, tts};

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
    /// Hold (or tap) to ask the voice agent instead of dictating.
    pub agent_shortcut: String,
    pub agent_model: String,
    pub ollama_url: String,
    pub agent_voice: String,
    pub speak_replies: bool,
    /// Instructions for the agent; empty uses the built-in personality.
    pub agent_personality: String,
    /// Dictation that starts with “Kem cho” goes to the agent.
    pub agent_commands: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            model: DEFAULT_MODEL.into(),
            language: languages::DEFAULT_LANGUAGE.into(),
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
            agent_shortcut: "CommandOrControl+Shift+K".into(),
            agent_model: agent::DEFAULT_MODEL.into(),
            ollama_url: agent::DEFAULT_URL.into(),
            agent_voice: tts::DEFAULT_VOICE.into(),
            speak_replies: true,
            agent_personality: String::new(),
            agent_commands: true,
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
        serde_json::from_value::<Self>(Value::Object(merged))
            .unwrap_or_default()
            .normalized()
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

    /// Replaces an unknown language with English and a model that can't
    /// transcribe the language with that language's default model.
    pub fn normalized(mut self) -> Self {
        let language = languages::find(&self.language)
            .or_else(|| languages::find(languages::DEFAULT_LANGUAGE))
            .unwrap();
        self.language = language.id.into();
        if !models::find(&self.model).is_ok_and(|m| m.supports(language.whisper)) {
            self.model = language.default_model.into();
        }
        self
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

    #[test]
    fn language_and_model_stay_compatible() {
        let fix = |language: &str, model: &str| {
            let s = Settings {
                language: language.into(),
                model: model.into(),
                ..Settings::default()
            }
            .normalized();
            (s.language, s.model)
        };
        assert_eq!(fix("auto", "small"), ("en".into(), "small".into()));
        assert_eq!(fix("gu-en", "base.en"), ("gu-en".into(), "gu-small".into()));
        assert_eq!(fix("gu", "small"), ("gu".into(), "small".into()));
        assert_eq!(fix("en", "hi-small"), ("en".into(), DEFAULT_MODEL.into()));
        assert_eq!(fix("xx", "nope"), ("en".into(), DEFAULT_MODEL.into()));
    }
}
