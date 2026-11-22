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
    let Ok(re) = RegexBuilder::new(&regex::escape(from))
        .case_insensitive(true)
        .build()
    else {
        return s.to_string();
    };
    // Whole words only: no letter or digit may touch either end, which `\b`
    // gets wrong for terms that start or end in punctuation, like "C++".
    let is_word = |c: char| c.is_alphanumeric() || c == '_';
    let mut out = String::with_capacity(s.len());
    let mut last = 0;
    for m in re.find_iter(s) {
        let before = s[..m.start()].chars().next_back();
        let after = s[m.end()..].chars().next();
        if before.is_some_and(is_word) || after.is_some_and(is_word) {
            continue;
        }
        out.push_str(&s[last..m.start()]);
        out.push_str(&r.to);
        last = m.end();
    }
    out.push_str(&s[last..]);
    out
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

    #[test]
    fn replaces_terms_ending_in_punctuation() {
        let reps = vec![Replacement {
            from: "C++".into(),
            to: "cpp".into(),
        }];
        let steps = Steps {
            replacements: &reps,
            ..Default::default()
        };
        assert_eq!(process("I use C++ daily", &steps), "I use cpp daily");
        assert_eq!(process("I use C++17", &steps), "I use C++17");
    }
}
