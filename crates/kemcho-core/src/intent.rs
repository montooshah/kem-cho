//! Spoken commands: dictation that starts with “Kem cho” goes to the agent
//! instead of being pasted.

use std::sync::LazyLock;

use regex::Regex;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Intent {
    /// Paste the text as dictated.
    Dictate,
    /// Write new text from this instruction and paste it.
    Write(String),
    /// Answer out loud, or rewrite the selected text when the request
    /// points at it (`about_selection`) and something is selected.
    Ask {
        request: String,
        about_selection: bool,
    },
}

// Whisper hears “Kem cho” as “Kim Cho”, “Ken Cho”, “Camcho” and so on.
static WAKE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?is)^[\s“”']*(?:(?:kem|kim|ken|kam|cam|came|kemm)[\s-]*(?:cho|chow|chu|choo|chho)\b|क[ेै]म\s*छो|કેમ\s*છો)[\s,.!?:;।-]*(.*)$",
    )
    .unwrap()
});

static WRITE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)^(?:please\s+)?(?:write|draft|compose|type|likho|lakho|लिखो|લખો)\b").unwrap()
});

// An edit of the selection: an English request that starts with an editing
// verb, or a Hindi or Gujarati one that does something to “this” (isko, આને).
// Questions such as “how does it rain?” are answered out loud instead.
static EDIT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)^(?:please\s+)?(?:make|rewrite|rephrase|reword|translate|fix|correct|proofread|shorten|summari[sz]e|simplify|expand|improve|polish|turn|convert|change|format|edit)\b|\b(?:ise|isko|isey|aane|aa ne|इसे|इसको|આને)\b",
    )
    .unwrap()
});

/// What a finished dictation asks for.
pub fn parse(text: &str) -> Intent {
    let Some(rest) = WAKE.captures(text).map(|c| c[1].trim().to_string()) else {
        return Intent::Dictate;
    };
    if rest.chars().filter(|c| c.is_alphanumeric()).count() < 2 {
        return Intent::Dictate;
    }
    let request = capitalize_first(&rest);
    if WRITE.is_match(&request) {
        Intent::Write(request)
    } else {
        Intent::Ask {
            about_selection: EDIT.is_match(&request),
            request,
        }
    }
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

    fn ask(request: &str, about_selection: bool) -> Intent {
        Intent::Ask {
            request: request.into(),
            about_selection,
        }
    }

    #[test]
    fn plain_dictation_is_pasted() {
        assert_eq!(parse("Move the release to Thursday."), Intent::Dictate);
        assert_eq!(parse("Kemchopra called."), Intent::Dictate);
        assert_eq!(parse("Kem cho?"), Intent::Dictate);
        assert_eq!(parse("I said kem cho, then left."), Intent::Dictate);
    }

    #[test]
    fn heard_variants_wake_the_agent() {
        // What base.en and small wrote for a spoken “Kem cho”.
        for text in [
            "Kem cho, what is the capital of Australia?",
            "Kim Cho, what is the capital of Australia?",
            "Ken Cho what is the capital of Australia?",
            "Camcho, what is the capital of Australia?",
            "Cam Cho, what is the capital of Australia?",
        ] {
            assert_eq!(
                parse(text),
                ask("What is the capital of Australia?", false),
                "{text}"
            );
        }
        assert_eq!(
            parse("કેમ છો, આજે હવામાન કેવું છે?"),
            ask("આજે હવામાન કેવું છે?", false)
        );
        assert_eq!(parse("केम छो। मौसम कैसा है?"), ask("मौसम कैसा है?", false));
    }

    #[test]
    fn write_and_rewrite() {
        assert_eq!(
            parse("Kim Cho, write a short thank you note to Priya."),
            Intent::Write("Write a short thank you note to Priya.".into())
        );
        assert_eq!(
            parse("Kem cho, please draft a reply"),
            Intent::Write("Please draft a reply".into())
        );
        assert_eq!(
            parse("Ken Cho make this more formal."),
            ask("Make this more formal.", true)
        );
        assert_eq!(
            parse("Kencho, translate this into Hindi."),
            ask("Translate this into Hindi.", true)
        );
        assert_eq!(
            parse("Kem cho, isko formal banao"),
            ask("Isko formal banao", true)
        );
        assert_eq!(parse("કેમ છો, આને ટૂંકું કરો"), ask("આને ટૂંકું કરો", true));
    }

    #[test]
    fn questions_about_it_are_answered() {
        assert_eq!(
            parse("Kem cho, how does it rain?"),
            ask("How does it rain?", false)
        );
        assert_eq!(
            parse("Kem cho, what does this mean?"),
            ask("What does this mean?", false)
        );
        assert_eq!(
            parse("Kem cho, is that right?"),
            ask("Is that right?", false)
        );
    }
}
