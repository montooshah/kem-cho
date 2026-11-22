use regex::{Regex, RegexBuilder};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Replacement {
    pub from: String,
    pub to: String,
}

#[derive(Debug, Clone, Default)]
pub struct Steps<'a> {
    pub remove_fillers: bool,
    pub voice_commands: bool,
    pub replacements: &'a [Replacement],
}

/// Turns a raw transcript into the text that gets pasted.
pub fn process(raw: &str, steps: &Steps) -> String {
    let mut text = collapse_spaces(raw.trim());
    if steps.remove_fillers {
        text = remove_fillers(&text);
    }
    for r in steps.replacements {
        text = replace_word(&text, r);
    }
    if steps.voice_commands {
        text = voice_commands(&text);
    }
    text = tidy_punctuation(&text);
    capitalize_first(text.trim_matches(' '))
}

fn collapse_spaces(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn remove_fillers(s: &str) -> String {
    let re = Regex::new(r"(?i)\b(um+|uh+|erm+|hmm+|ah+)\b[,.]?\s*").unwrap();
    collapse_spaces(&re.replace_all(s, ""))
}

fn replace_word(s: &str, r: &Replacement) -> String {
    let from = r.from.trim();
    if from.is_empty() {
        return s.to_string();
    }
    let pattern = format!(r"(?:^|\b){}(?:\b|$)", regex::escape(from));
    match RegexBuilder::new(&pattern).case_insensitive(true).build() {
        Ok(re) => re.replace_all(s, regex::NoExpand(&r.to)).into_owned(),
        Err(_) => s.to_string(),
    }
}

fn voice_commands(s: &str) -> String {
    let para = Regex::new(r"(?i)\s*[,.]?\s*\bnew paragraph\b[,.]?\s*").unwrap();
    let line = Regex::new(r"(?i)\s*[,.]?\s*\bnew line\b[,.]?\s*").unwrap();
    let s = para.replace_all(s, "\n\n");
    line.replace_all(&s, "\n").into_owned()
}

fn tidy_punctuation(s: &str) -> String {
    let re = Regex::new(r" +([,.!?;:])").unwrap();
    re.replace_all(s, "$1").into_owned()
}

fn capitalize_first(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) => c.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cleans_and_replaces() {
        let reps = vec![Replacement {
            from: "kem cho".into(),
            to: "Kem Cho".into(),
        }];
        let steps = Steps {
            remove_fillers: true,
            voice_commands: true,
            replacements: &reps,
        };
        assert_eq!(
            process("  um, welcome to   kem cho , new line uh bye ", &steps),
            "Welcome to Kem Cho\nbye"
        );
    }

    #[test]
    fn leaves_words_containing_fillers() {
        let steps = Steps {
            remove_fillers: true,
            ..Default::default()
        };
        assert_eq!(
            process("the umbrella is huge", &steps),
            "The umbrella is huge"
        );
    }

    #[test]
    fn replacement_is_literal() {
        let reps = vec![Replacement {
            from: "dollar".into(),
            to: "$1".into(),
        }];
        let steps = Steps {
            replacements: &reps,
            ..Default::default()
        };
        assert_eq!(process("one dollar", &steps), "One $1");
    }
}
