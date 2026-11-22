//! The voice agent's brain: a local LLM served by Ollama on this computer.
//! Kem Cho talks to it over loopback HTTP and streams the reply so speech can
//! start on the first sentence.

use std::io::{BufRead, BufReader};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use serde_json::json;

pub const DEFAULT_URL: &str = "http://127.0.0.1:11434";
pub const DEFAULT_MODEL: &str = "llama3.2:3b";

/// Models suggested in the app, smallest first. Any other Ollama model works.
pub const SUGGESTED: &[(&str, &str)] = &[
    (
        "llama3.2:1b",
        "Llama 3.2 1B · 1.3 GB · fastest, simple answers",
    ),
    ("llama3.2:3b", "Llama 3.2 3B · 2.0 GB · good default"),
    (
        "gemma3:4b",
        "Gemma 3 4B · 3.3 GB · stronger, needs 8 GB RAM",
    ),
    (
        "qwen3:8b",
        "Qwen 3 8B · 5.2 GB · strongest, needs 16 GB RAM",
    ),
];

pub const DEFAULT_PERSONALITY: &str = "You are Kem Cho, a warm, helpful voice assistant that runs entirely on the user's own computer. \
Your replies are spoken aloud, so answer in one to three short sentences of natural speech. \
Never use markdown, lists, code blocks, URLs or emoji. If a question needs a long answer, give the key point and offer to go on.";

/// How many past exchanges are sent with each question.
const MAX_TURNS: usize = 12;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Message {
    pub role: String,
    pub content: String,
}

impl Message {
    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: "user".into(),
            content: content.into(),
        }
    }
    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            role: "assistant".into(),
            content: content.into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalModel {
    pub name: String,
    pub size: u64,
}

pub struct Ollama {
    base: String,
    agent: ureq::Agent,
}

impl Ollama {
    pub fn new(base: &str) -> Self {
        let base = if base.trim().is_empty() {
            DEFAULT_URL
        } else {
            base.trim()
        };
        let agent = ureq::Agent::config_builder()
            .timeout_connect(Some(Duration::from_secs(2)))
            .http_status_as_error(false)
            .build()
            .new_agent();
        Self {
            base: base.trim_end_matches('/').to_string(),
            agent,
        }
    }

    fn unreachable(&self, err: ureq::Error) -> anyhow::Error {
        anyhow!(
            "Ollama isn't running at {} ({err}). Install it from ollama.com and open it.",
            self.base
        )
    }

    fn check(response: &mut ureq::http::Response<ureq::Body>) -> Result<()> {
        if response.status().is_success() {
            return Ok(());
        }
        let status = response.status();
        let body = response.body_mut().read_to_string().unwrap_or_default();
        let message = serde_json::from_str::<serde_json::Value>(&body)
            .ok()
            .and_then(|v| v["error"].as_str().map(str::to_string))
            .unwrap_or(body);
        bail!("Ollama: {message} ({status})")
    }

    pub fn version(&self) -> Result<String> {
        let mut response = self
            .agent
            .get(format!("{}/api/version", self.base))
            .call()
            .map_err(|e| self.unreachable(e))?;
        Self::check(&mut response)?;
        let v: serde_json::Value = response.body_mut().read_json()?;
        Ok(v["version"].as_str().unwrap_or("unknown").to_string())
    }

    /// Models already downloaded into Ollama.
    pub fn models(&self) -> Result<Vec<LocalModel>> {
        #[derive(Deserialize)]
        struct Tags {
            models: Vec<LocalModel>,
        }
        let mut response = self
            .agent
            .get(format!("{}/api/tags", self.base))
            .call()
            .map_err(|e| self.unreachable(e))?;
        Self::check(&mut response)?;
        let tags: Tags = response.body_mut().read_json()?;
        Ok(tags.models)
    }

    /// Downloads a model into Ollama, reporting `(status, done, total)`.
    pub fn pull(&self, model: &str, mut progress: impl FnMut(&str, u64, u64)) -> Result<()> {
        let mut response = self
            .agent
            .post(format!("{}/api/pull", self.base))
            .send_json(json!({ "model": model, "stream": true }))
            .map_err(|e| self.unreachable(e))?;
        Self::check(&mut response)?;
        let reader = BufReader::new(response.into_body().into_reader());
        for line in reader.lines() {
            let v: serde_json::Value = serde_json::from_str(&line?)?;
            if let Some(err) = v["error"].as_str() {
                bail!("Ollama couldn't download {model}: {err}");
            }
            progress(
                v["status"].as_str().unwrap_or(""),
                v["completed"].as_u64().unwrap_or(0),
                v["total"].as_u64().unwrap_or(0),
            );
        }
        Ok(())
    }

    /// Loads a model into memory so the next reply starts quickly.
    pub fn warm(&self, model: &str) -> Result<()> {
        let mut response = self
            .agent
            .post(format!("{}/api/chat", self.base))
            .send_json(json!({ "model": model, "messages": [], "keep_alive": "30m" }))
            .map_err(|e| self.unreachable(e))?;
        Self::check(&mut response)
    }

    /// Streams a chat reply, calling `on_token` with each new piece of text.
    /// Returns the whole reply, or what arrived before `cancel` was set.
    pub fn chat(
        &self,
        model: &str,
        messages: &[Message],
        cancel: &AtomicBool,
        mut on_token: impl FnMut(&str),
    ) -> Result<String> {
        let mut response = self
            .agent
            .post(format!("{}/api/chat", self.base))
            .send_json(json!({
                "model": model,
                "messages": messages,
                "stream": true,
                "think": false,
                "keep_alive": "30m",
            }))
            .map_err(|e| self.unreachable(e))?;
        if response.status() == 404 {
            bail!("The model `{model}` isn't downloaded in Ollama yet.");
        }
        Self::check(&mut response)?;
        let reader = BufReader::new(response.into_body().into_reader());
        let mut reply = String::new();
        for line in reader.lines() {
            if cancel.load(Ordering::Relaxed) {
                break;
            }
            let line = line.context("reading Ollama's reply")?;
            if line.trim().is_empty() {
                continue;
            }
            let v: serde_json::Value = serde_json::from_str(&line)?;
            if let Some(err) = v["error"].as_str() {
                bail!("Ollama: {err}");
            }
            if let Some(token) = v["message"]["content"].as_str()
                && !token.is_empty()
            {
                reply.push_str(token);
                on_token(token);
            }
            if v["done"].as_bool() == Some(true) {
                break;
            }
        }
        Ok(reply)
    }
}

/// The running conversation, kept in memory only.
#[derive(Debug, Default, Clone)]
pub struct Conversation {
    pub turns: Vec<Message>,
}

impl Conversation {
    /// The messages to send for a new question.
    pub fn prompt(&self, personality: &str, question: &str) -> Vec<Message> {
        let personality = if personality.trim().is_empty() {
            DEFAULT_PERSONALITY
        } else {
            personality
        };
        let start = self.turns.len().saturating_sub(MAX_TURNS * 2);
        let mut messages = vec![Message {
            role: "system".into(),
            content: personality.into(),
        }];
        messages.extend(self.turns[start..].iter().cloned());
        messages.push(Message::user(question));
        messages
    }

    pub fn record(&mut self, question: &str, reply: &str) {
        self.turns.push(Message::user(question));
        self.turns.push(Message::assistant(reply));
    }
}

/// Cuts streamed text into sentences so each can be spoken as soon as it
/// is complete.
#[derive(Default)]
pub struct Sentences {
    buf: String,
}

impl Sentences {
    pub fn push(&mut self, text: &str) -> Vec<String> {
        self.buf.push_str(text);
        let mut out = Vec::new();
        loop {
            let cut = self.buf.char_indices().find_map(|(i, c)| {
                let end = i + c.len_utf8();
                let ends_sentence = matches!(c, '.' | '!' | '?' | '\n' | '…' | '।');
                let next = self.buf[end..].chars().next();
                (ends_sentence && next.is_some_and(char::is_whitespace)).then_some(end)
            });
            let Some(end) = cut else { break };
            let sentence: String = self.buf.drain(..end).collect();
            if let Some(s) = speakable(&sentence) {
                out.push(s);
            }
        }
        out
    }

    pub fn finish(&mut self) -> Option<String> {
        speakable(&std::mem::take(&mut self.buf))
    }
}

/// Strips what reads badly aloud: markdown marks, code fences and links.
pub fn speakable(text: &str) -> Option<String> {
    let mut s = String::with_capacity(text.len());
    for word in text.split_whitespace() {
        let word = if word.starts_with("http://") || word.starts_with("https://") {
            "a link"
        } else {
            word.trim_matches(|c| matches!(c, '*' | '_' | '`' | '#' | '>'))
        };
        if word.is_empty() || word == "-" {
            continue;
        }
        if !s.is_empty() {
            s.push(' ');
        }
        s.push_str(word);
    }
    s.chars().any(char::is_alphanumeric).then_some(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sentences_split_as_they_stream() {
        let mut s = Sentences::default();
        let mut all = Vec::new();
        for piece in ["Kem", " cho! I'm", " fine. Version 3.5", " is out", "."] {
            all.extend(s.push(piece));
        }
        all.extend(s.finish());
        assert_eq!(all, ["Kem cho!", "I'm fine.", "Version 3.5 is out."]);
    }

    #[test]
    fn speakable_strips_markdown() {
        assert_eq!(
            speakable("**Sure** - see https://example.com `now`").as_deref(),
            Some("Sure see a link now")
        );
        assert_eq!(speakable(" ** "), None);
    }

    #[test]
    fn prompt_keeps_recent_turns() {
        let mut c = Conversation::default();
        for i in 0..20 {
            c.record(&format!("q{i}"), &format!("a{i}"));
        }
        let p = c.prompt("", "next");
        assert_eq!(p[0].role, "system");
        assert_eq!(p.len(), 1 + MAX_TURNS * 2 + 1);
        assert_eq!(p.last().unwrap().content, "next");
        assert_eq!(p[1].content, "q8");
    }
}
