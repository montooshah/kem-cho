//! Agent mode: the agent shortcut records a question, Whisper transcribes
//! it, a local Ollama model answers, and the reply is spoken as it streams.

use std::sync::atomic::Ordering;
use std::thread;

use kemcho_core::agent::{self, LocalModel, Ollama, Sentences};
use kemcho_core::history;
use kemcho_core::tts::{self, Voice};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};

use crate::{AppState, Phase, emit_status, set_phase};

#[derive(Clone, Serialize)]
struct Turn {
    id: u64,
    question: String,
}

#[derive(Clone, Serialize)]
struct Token {
    id: u64,
    token: String,
}

#[derive(Clone, Serialize)]
struct TurnDone {
    id: u64,
    reply: String,
    error: Option<String>,
}

/// Answers one question, speaking the reply. Runs on a worker thread and
/// leaves the phase at Idle.
pub fn answer(app: &AppHandle, question: String) {
    let state = app.state::<AppState>();
    let id = history::now_ms();
    let _ = app.emit(
        "agent-turn",
        Turn {
            id,
            question: question.clone(),
        },
    );
    let result = reply(app, id, &question);
    let (reply, error) = match result {
        Ok(reply) => (reply, None),
        Err(err) => {
            tracing::error!("agent turn failed: {err:#}");
            (String::new(), Some(format!("{err:#}")))
        }
    };
    if !reply.trim().is_empty() {
        state
            .conversation
            .lock()
            .unwrap()
            .record(&question, reply.trim());
    }
    let _ = app.emit(
        "agent-done",
        TurnDone {
            id,
            reply,
            error: error.clone(),
        },
    );
    set_phase(app, Phase::Idle);
    emit_status(app, Phase::Idle, error);
}

fn reply(app: &AppHandle, id: u64, question: &str) -> anyhow::Result<String> {
    let state = app.state::<AppState>();
    let settings = state.settings.lock().unwrap().clone();
    let cancel = state.cancel.clone();
    let messages = state
        .conversation
        .lock()
        .unwrap()
        .prompt(&settings.agent_personality, question);
    set_phase(app, Phase::Thinking);
    emit_status(app, Phase::Thinking, None);

    let speech = if settings.speak_replies {
        match tts::Speech::start(&settings.agent_voice, cancel.clone()) {
            Ok(speech) => Some(speech),
            Err(err) => {
                let _ = app.emit("agent-warning", format!("{err:#}"));
                None
            }
        }
    } else {
        None
    };
    let mut sentences = Sentences::default();
    let mut speaking = false;
    let mut start_speaking = |app: &AppHandle| {
        if !speaking {
            speaking = true;
            set_phase(app, Phase::Speaking);
            emit_status(app, Phase::Speaking, None);
        }
    };
    let ollama = Ollama::new(&settings.ollama_url);
    let reply = ollama.chat(&settings.agent_model, &messages, &cancel, |token| {
        let _ = app.emit(
            "agent-token",
            Token {
                id,
                token: token.to_string(),
            },
        );
        for sentence in sentences.push(token) {
            if let Some(speech) = &speech {
                start_speaking(app);
                speech.say(&sentence);
            }
        }
    });
    if let Some(speech) = speech {
        if reply.is_ok()
            && let Some(rest) = sentences.finish()
        {
            start_speaking(app);
            speech.say(&rest);
        }
        if let Err(err) = speech.finish() {
            let _ = app.emit("agent-warning", format!("{err:#}"));
        }
    }
    reply
}

/// Starts loading the agent's model while the person is still talking.
pub fn warm(app: &AppHandle) {
    let state = app.state::<AppState>();
    let (url, model) = {
        let s = state.settings.lock().unwrap();
        (s.ollama_url.clone(), s.agent_model.clone())
    };
    thread::spawn(move || {
        if let Err(err) = Ollama::new(&url).warm(&model) {
            tracing::debug!("warming {model}: {err:#}");
        }
    });
}

/// Stops the current reply, voice included.
pub fn interrupt(app: &AppHandle) {
    app.state::<AppState>()
        .cancel
        .store(true, Ordering::Relaxed);
}

#[derive(Serialize)]
pub struct AgentStatus {
    running: bool,
    version: Option<String>,
    error: Option<String>,
    installed: Vec<LocalModel>,
    suggested: Vec<Suggested>,
}

#[derive(Serialize)]
pub struct Suggested {
    id: &'static str,
    label: &'static str,
}

#[tauri::command]
pub fn agent_status(state: State<AppState>) -> AgentStatus {
    let url = state.settings.lock().unwrap().ollama_url.clone();
    let ollama = Ollama::new(&url);
    let suggested = agent::SUGGESTED
        .iter()
        .map(|&(id, label)| Suggested { id, label })
        .collect();
    match ollama.version().and_then(|v| Ok((v, ollama.models()?))) {
        Ok((version, installed)) => AgentStatus {
            running: true,
            version: Some(version),
            error: None,
            installed,
            suggested,
        },
        Err(err) => AgentStatus {
            running: false,
            version: None,
            error: Some(format!("{err:#}")),
            installed: Vec::new(),
            suggested,
        },
    }
}

#[derive(Clone, Serialize)]
struct PullProgress {
    model: String,
    status: String,
    done: u64,
    total: u64,
}

#[derive(Clone, Serialize)]
struct PullDone {
    model: String,
    error: Option<String>,
}

#[tauri::command]
pub fn agent_pull(app: AppHandle, state: State<AppState>, model: String) {
    let url = state.settings.lock().unwrap().ollama_url.clone();
    thread::spawn(move || {
        let mut last = (String::new(), u64::MAX);
        let result = Ollama::new(&url).pull(&model, |status, done, total| {
            let pct = done * 100 / total.max(1);
            if last.0 != status || last.1 != pct {
                last = (status.to_string(), pct);
                let _ = app.emit(
                    "agent-pull",
                    PullProgress {
                        model: model.clone(),
                        status: status.to_string(),
                        done,
                        total,
                    },
                );
            }
        });
        let _ = app.emit(
            "agent-pull-done",
            PullDone {
                model,
                error: result.err().map(|e| format!("{e:#}")),
            },
        );
    });
}

#[derive(Serialize)]
pub struct VoiceInfo {
    #[serde(flatten)]
    voice: Voice,
    installed: bool,
}

#[derive(Serialize)]
pub struct Voices {
    /// False where the OS voice speaks instead (macOS).
    neural: bool,
    voices: Vec<VoiceInfo>,
}

#[tauri::command]
pub fn list_voices() -> Voices {
    Voices {
        neural: tts::uses_piper(),
        voices: tts::VOICES
            .iter()
            .map(|v| VoiceInfo {
                voice: *v,
                installed: v.is_installed(),
            })
            .collect(),
    }
}

#[derive(Clone, Serialize)]
struct Progress {
    id: String,
    done: u64,
    total: u64,
}

#[derive(Clone, Serialize)]
struct Done {
    id: String,
    error: Option<String>,
}

#[tauri::command]
pub fn download_voice(app: AppHandle, id: String) -> Result<(), String> {
    let voice = tts::find_voice(&id).map_err(|e| e.to_string())?;
    thread::spawn(move || {
        let mut last = u64::MAX;
        let result = voice.download(|done, total| {
            let pct = done * 100 / total.max(1);
            if pct != last {
                last = pct;
                let _ = app.emit(
                    "voice-progress",
                    Progress {
                        id: voice.id.into(),
                        done,
                        total,
                    },
                );
            }
        });
        let _ = app.emit(
            "voice-done",
            Done {
                id: voice.id.into(),
                error: result.err().map(|e| format!("{e:#}")),
            },
        );
    });
    Ok(())
}

/// Speaks a sample sentence in the chosen voice.
#[tauri::command]
pub fn preview_voice(state: State<AppState>, id: String) -> Result<(), String> {
    let cancel = state.cancel.clone();
    cancel.store(false, Ordering::Relaxed);
    let speech = tts::Speech::start(&id, cancel).map_err(|e| format!("{e:#}"))?;
    thread::spawn(move || {
        speech.say("Kem cho! This is how I'll sound when I answer you.");
        let _ = speech.finish();
    });
    Ok(())
}

/// Asks a typed question, as if it had been spoken.
#[tauri::command]
pub fn agent_ask(app: AppHandle, state: State<AppState>, question: String) -> Result<(), String> {
    let question = question.trim().to_string();
    if question.is_empty() {
        return Ok(());
    }
    {
        let mut session = state.session.lock().unwrap();
        if session.phase != Phase::Idle {
            return Err("Kem Cho is busy.".into());
        }
        session.phase = Phase::Thinking;
    }
    state.cancel.store(false, Ordering::Relaxed);
    thread::spawn(move || answer(&app, question));
    Ok(())
}

#[tauri::command]
pub fn agent_stop(app: AppHandle) {
    interrupt(&app);
}

/// Forgets the conversation so far.
#[tauri::command]
pub fn agent_reset(state: State<AppState>) {
    state.conversation.lock().unwrap().turns.clear();
}
