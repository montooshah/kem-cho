//! “Kem cho, write …” and “Kem cho, make this …”: the agent's model writes
//! or rewrites text, which is pasted at the cursor like a dictation.

use std::sync::atomic::Ordering;

use anyhow::bail;
use kemcho_core::agent::{self, Ollama};
use tauri::{AppHandle, Manager};

use crate::{AppState, Phase, emit_status, set_phase};

/// The text to paste for `instruction`, replacing `selection` when given.
/// `None` when the person stopped it.
pub fn compose(
    app: &AppHandle,
    instruction: &str,
    selection: Option<&str>,
) -> anyhow::Result<Option<String>> {
    let state = app.state::<AppState>();
    let settings = state.settings.lock().unwrap().clone();
    state.cancel.store(false, Ordering::Relaxed);
    set_phase(app, Phase::Thinking);
    emit_status(app, Phase::Thinking, None);
    let messages = agent::writing_prompt(instruction, selection);
    let reply = Ollama::new(&settings.ollama_url).chat(
        &settings.agent_model,
        &messages,
        &state.cancel,
        |_| {},
    )?;
    if state.cancel.load(Ordering::Relaxed) {
        return Ok(None);
    }
    let text = agent::tidy_written(&reply);
    if text.is_empty() {
        bail!("The model wrote nothing.");
    }
    Ok(Some(text))
}
