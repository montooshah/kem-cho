//! The dictation languages Kem Cho offers, and how each maps onto Whisper.
use serde::Serialize;

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
pub struct Language {
    /// Stored in settings, e.g. `hi-en` for Hindi mixed with English.
    pub id: &'static str,
    pub label: &'static str,
    /// The language Whisper is told to expect.
    pub whisper: &'static str,
    /// The model picked when someone switches to this language.
    pub default_model: &'static str,
    pub hint: &'static str,
}

pub const LANGUAGES: &[Language] = &[
    Language {
        id: "en",
        label: "English",
        whisper: "en",
        default_model: "base.en",
        hint: "",
    },
    Language {
        id: "hi",
        label: "Hindi (हिन्दी)",
        whisper: "hi",
        default_model: "hi-small",
        hint: "Written in Devanagari.",
    },
    Language {
        id: "hi-en",
        label: "Hindi + English (Hinglish)",
        whisper: "hi",
        default_model: "hinglish",
        hint: "The Hinglish model writes everything in Roman letters; the other models write Devanagari.",
    },
    Language {
        id: "gu",
        label: "Gujarati (ગુજરાતી)",
        whisper: "gu",
        default_model: "gu-small",
        hint: "Written in Gujarati script.",
    },
    Language {
        id: "gu-en",
        label: "Gujarati + English",
        whisper: "gu",
        default_model: "gu-small",
        hint: "Early support: English words come out in Gujarati script or may be missed.",
    },
];

pub const DEFAULT_LANGUAGE: &str = "en";

pub fn find(id: &str) -> Option<&'static Language> {
    LANGUAGES.iter().find(|l| l.id == id)
}
