//! Kem Cho's desktop app: a tray app with a global hold-to-talk shortcut
//! that records, transcribes locally, and pastes at the cursor, plus an
//! agent shortcut that answers spoken questions out loud.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use kemcho_core::agent::Conversation;
use kemcho_core::history::{self, Entry, History};
use kemcho_core::models::{self, Model};
use kemcho_core::settings::{Settings, ShortcutMode};
use kemcho_core::transcribe::Options;
use kemcho_core::{Transcriber, audio, postprocess, vad};
use serde::Serialize;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, State, WindowEvent};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};

mod agent;
mod paste;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Phase {
    Idle,
    Recording,
    Transcribing,
    Thinking,
    Speaking,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    Dictate,
    Agent,
}

#[derive(Clone, Debug, Serialize)]
pub struct Status {
    phase: Phase,
    message: Option<String>,
}

struct Session {
    phase: Phase,
    mode: Mode,
    started: Option<Instant>,
}

pub struct AppState {
    settings: Mutex<Settings>,
    session: Mutex<Session>,
    recorder: audio::Recorder,
    transcriber: Mutex<Option<Transcriber>>,
    history: Option<History>,
    paster: Mutex<paste::Paster>,
    conversation: Mutex<Conversation>,
    /// Set to stop the agent's current reply.
    cancel: Arc<AtomicBool>,
}

#[derive(Serialize)]
struct ModelInfo {
    #[serde(flatten)]
    model: Model,
    installed: bool,
}

#[derive(Clone, Serialize)]
struct Progress {
    id: String,
    done: u64,
    total: u64,
}

#[derive(Clone, Serialize)]
struct DownloadDone {
    id: String,
    error: Option<String>,
}

fn set_phase(app: &AppHandle, phase: Phase) {
    app.state::<AppState>().session.lock().unwrap().phase = phase;
}

fn emit_status(app: &AppHandle, phase: Phase, message: Option<String>) {
    let _ = app.emit("status", Status { phase, message });
    if let Some(tray) = app.tray_by_id("main") {
        let tip = match phase {
            Phase::Idle => "Kem Cho",
            Phase::Recording => "Kem Cho: listening",
            Phase::Transcribing => "Kem Cho: transcribing",
            Phase::Thinking => "Kem Cho: thinking",
            Phase::Speaking => "Kem Cho: speaking",
        };
        let _ = tray.set_tooltip(Some(tip));
    }
    if let Some(hud) = app.get_webview_window("hud") {
        if phase == Phase::Idle {
            let _ = hud.hide();
        } else {
            place_hud(&hud);
            let _ = hud.show();
        }
    }
}

fn place_hud(hud: &tauri::WebviewWindow) {
    let Ok(Some(monitor)) = hud.primary_monitor() else {
        return;
    };
    let (Ok(size), area) = (hud.outer_size(), monitor.work_area()) else {
        return;
    };
    let x = area.position.x + (area.size.width as i32 - size.width as i32) / 2;
    let y = area.position.y + area.size.height as i32 - size.height as i32 - 48;
    let _ = hud.set_position(PhysicalPosition::new(x, y));
}

fn show_main(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

fn start_recording(app: &AppHandle, mode: Mode) {
    let state = app.state::<AppState>();
    let (model_id, microphone) = {
        let s = state.settings.lock().unwrap();
        (s.model.clone(), s.microphone.clone())
    };
    let mut session = state.session.lock().unwrap();
    if session.phase != Phase::Idle {
        return;
    }
    match models::find(&model_id) {
        Ok(m) if m.is_installed() => {}
        _ => {
            drop(session);
            emit_status(
                app,
                Phase::Idle,
                Some("Download a speech model first.".into()),
            );
            show_main(app);
            return;
        }
    }
    if let Err(err) = state.recorder.start(microphone) {
        drop(session);
        emit_status(app, Phase::Idle, Some(format!("Microphone error: {err:#}")));
        return;
    }
    session.phase = Phase::Recording;
    session.mode = mode;
    session.started = Some(Instant::now());
    drop(session);
    emit_status(app, Phase::Recording, None);
    if mode == Mode::Agent {
        state.cancel.store(false, Ordering::Relaxed);
        agent::warm(app);
    }

    let app = app.clone();
    thread::spawn(move || {
        let state = app.state::<AppState>();
        while state.session.lock().unwrap().phase == Phase::Recording {
            let _ = app.emit_to("hud", "level", state.recorder.level());
            thread::sleep(Duration::from_millis(50));
        }
    });
}

fn stop_recording(app: &AppHandle) {
    let state = app.state::<AppState>();
    let mode = {
        let mut session = state.session.lock().unwrap();
        if session.phase != Phase::Recording {
            return;
        }
        session.phase = Phase::Transcribing;
        session.mode
    };
    emit_status(app, Phase::Transcribing, None);
    let app = app.clone();
    if mode == Mode::Agent {
        thread::spawn(move || match hear(&app) {
            Ok(Some(heard)) => agent::answer(&app, heard.text),
            result => {
                let message = match result {
                    Err(err) => format!("{err:#}"),
                    _ => "No speech heard.".into(),
                };
                set_phase(&app, Phase::Idle);
                emit_status(&app, Phase::Idle, Some(message));
            }
        });
        return;
    }
    thread::spawn(move || {
        let message = match finish(&app) {
            Ok(Some(entry)) => {
                let _ = app.emit("dictation", entry);
                None
            }
            Ok(None) => Some("No speech heard.".to_string()),
            Err(err) => {
                tracing::error!("dictation failed: {err:#}");
                Some(format!("{err:#}"))
            }
        };
        let state = app.state::<AppState>();
        {
            let mut session = state.session.lock().unwrap();
            session.phase = Phase::Idle;
            session.started = None;
        }
        emit_status(&app, Phase::Idle, message);
    });
}

fn ensure_transcriber<'a>(
    slot: &'a mut Option<Transcriber>,
    model: &'static Model,
) -> anyhow::Result<&'a Transcriber> {
    if slot.as_ref().is_none_or(|t| t.model.id != model.id) {
        *slot = None;
        *slot = Some(Transcriber::load(model)?);
    }
    Ok(slot.as_ref().unwrap())
}

struct Heard {
    text: String,
    raw: String,
    model: &'static Model,
    audio_ms: u64,
    transcribe_ms: u64,
}

/// Stops recording and transcribes what was said.
fn hear(app: &AppHandle) -> anyhow::Result<Option<Heard>> {
    let state = app.state::<AppState>();
    let samples = state.recorder.stop()?;
    let audio_ms = samples.len() as u64 * 1000 / kemcho_core::SAMPLE_RATE as u64;
    let Some(speech) = vad::trim(&samples, vad::DEFAULT_MIN_RMS) else {
        return Ok(None);
    };
    let settings = state.settings.lock().unwrap().clone();
    let model = models::find(&settings.model)?;
    let transcript = {
        let mut slot = state.transcriber.lock().unwrap();
        let transcriber = ensure_transcriber(&mut slot, model)?;
        transcriber.transcribe(
            speech,
            &Options {
                language: &settings.language,
                vocabulary: &settings.vocabulary,
                threads: None,
            },
        )?
    };
    let text = postprocess::process(&transcript.text, &settings.steps());
    if text.is_empty() {
        return Ok(None);
    }
    Ok(Some(Heard {
        text,
        raw: transcript.text,
        model,
        audio_ms,
        transcribe_ms: transcript.elapsed_ms,
    }))
}

fn finish(app: &AppHandle) -> anyhow::Result<Option<Entry>> {
    let state = app.state::<AppState>();
    let Some(Heard {
        text,
        raw,
        model,
        audio_ms,
        transcribe_ms,
    }) = hear(app)?
    else {
        return Ok(None);
    };
    let settings = state.settings.lock().unwrap().clone();
    let entry = Entry {
        id: history::now_ms(),
        text,
        raw,
        model: model.id.to_string(),
        audio_ms,
        transcribe_ms,
    };
    // Saved before pasting so a clipboard failure doesn't lose the text.
    if settings.history_days > 0
        && let Some(h) = &state.history
        && let Err(err) = h.append(&entry)
    {
        tracing::warn!("saving history: {err:#}");
    }
    {
        let mut paster = state.paster.lock().unwrap();
        if settings.paste {
            paster.paste(&entry.text, settings.restore_clipboard)?;
        } else {
            paster.copy(&entry.text)?;
        }
    }
    Ok(Some(entry))
}

fn on_shortcut(app: &AppHandle, shortcut: &Shortcut, pressed: bool) {
    let state = app.state::<AppState>();
    let (hold, agent_shortcut) = {
        let s = state.settings.lock().unwrap();
        (
            s.shortcut_mode == ShortcutMode::Hold,
            s.agent_shortcut.parse::<Shortcut>().ok(),
        )
    };
    let wanted = if agent_shortcut.as_ref() == Some(shortcut) {
        Mode::Agent
    } else {
        Mode::Dictate
    };
    let (phase, mode) = {
        let s = state.session.lock().unwrap();
        (s.phase, s.mode)
    };
    match (phase, pressed) {
        (Phase::Idle, true) => start_recording(app, wanted),
        (Phase::Recording, false) if hold && mode == wanted => stop_recording(app),
        (Phase::Recording, true) if !hold && mode == wanted => stop_recording(app),
        (Phase::Thinking | Phase::Speaking, true) => agent::interrupt(app),
        _ => {}
    }
}

fn parse_shortcut(accelerator: &str) -> Result<Shortcut, String> {
    accelerator
        .parse()
        .map_err(|e| format!("`{accelerator}` is not a valid shortcut: {e}"))
}

fn register_shortcuts(app: &AppHandle, settings: &Settings) -> Result<(), String> {
    let dictate = parse_shortcut(&settings.shortcut)?;
    let agent = parse_shortcut(&settings.agent_shortcut)?;
    if dictate == agent {
        return Err("Dictation and the agent need different shortcuts.".into());
    }
    let gs = app.global_shortcut();
    let _ = gs.unregister_all();
    gs.register(dictate)
        .map_err(|e| format!("couldn't register `{}`: {e}", settings.shortcut))?;
    gs.register(agent)
        .map_err(|e| format!("couldn't register `{}`: {e}", settings.agent_shortcut))
}

/// Loads the chosen model in the background so the first dictation is fast.
fn preload(app: &AppHandle) {
    let app = app.clone();
    thread::spawn(move || {
        let state = app.state::<AppState>();
        let id = state.settings.lock().unwrap().model.clone();
        if let Ok(model) = models::find(&id)
            && model.is_installed()
        {
            let mut slot = state.transcriber.lock().unwrap();
            if let Err(err) = ensure_transcriber(&mut slot, model) {
                tracing::warn!("preloading {id}: {err:#}");
            }
        }
    });
}

#[tauri::command]
fn get_settings(state: State<AppState>) -> Settings {
    state.settings.lock().unwrap().clone()
}

#[tauri::command]
fn save_settings(
    app: AppHandle,
    state: State<AppState>,
    settings: Settings,
) -> Result<Settings, String> {
    let previous = state.settings.lock().unwrap().clone();
    if (previous.shortcut != settings.shortcut
        || previous.agent_shortcut != settings.agent_shortcut)
        && let Err(err) = register_shortcuts(&app, &settings)
    {
        let _ = register_shortcuts(&app, &previous);
        return Err(err);
    }
    models::find(&settings.model).map_err(|e| e.to_string())?;
    settings
        .save()
        .map_err(|e| format!("saving settings: {e:#}"))?;
    *state.settings.lock().unwrap() = settings.clone();
    if previous.history_days != settings.history_days
        && let Some(h) = &state.history
    {
        let _ = h.prune(settings.history_days);
    }
    if previous.model != settings.model {
        preload(&app);
    }
    Ok(settings)
}

#[tauri::command]
fn suspend_shortcut(app: AppHandle, state: State<AppState>, suspended: bool) -> Result<(), String> {
    if suspended {
        app.global_shortcut()
            .unregister_all()
            .map_err(|e| e.to_string())
    } else {
        let settings = state.settings.lock().unwrap().clone();
        register_shortcuts(&app, &settings)
    }
}

#[tauri::command]
fn list_models() -> Vec<ModelInfo> {
    models::CATALOG
        .iter()
        .map(|m| ModelInfo {
            model: *m,
            installed: m.is_installed(),
        })
        .collect()
}

#[tauri::command]
fn download_model(app: AppHandle, id: String) -> Result<(), String> {
    let model = models::find(&id).map_err(|e| e.to_string())?;
    thread::spawn(move || {
        let mut last = u64::MAX;
        let result = model.download(|done, total| {
            let pct = done * 100 / total.max(1);
            if pct != last {
                last = pct;
                let _ = app.emit(
                    "model-progress",
                    Progress {
                        id: model.id.into(),
                        done,
                        total,
                    },
                );
            }
        });
        let _ = app.emit(
            "model-done",
            DownloadDone {
                id: model.id.into(),
                error: result.as_ref().err().map(|e| format!("{e:#}")),
            },
        );
        if result.is_ok() {
            preload(&app);
        }
    });
    Ok(())
}

#[tauri::command]
fn delete_model(state: State<AppState>, id: String) -> Result<(), String> {
    let model = models::find(&id).map_err(|e| e.to_string())?;
    let mut slot = state.transcriber.lock().unwrap();
    if slot.as_ref().is_some_and(|t| t.model.id == model.id) {
        *slot = None;
    }
    model.remove().map_err(|e| e.to_string())
}

#[tauri::command]
fn list_microphones() -> Vec<String> {
    audio::input_devices()
}

#[tauri::command]
fn history_list(state: State<AppState>, query: Option<String>) -> Result<Vec<Entry>, String> {
    let Some(h) = &state.history else {
        return Ok(Vec::new());
    };
    let days = state.settings.lock().unwrap().history_days;
    if let Err(err) = h.prune(days) {
        tracing::warn!("pruning history: {err:#}");
    }
    match query.filter(|q| !q.trim().is_empty()) {
        Some(q) => h.search(&q),
        None => h.list(),
    }
    .map_err(|e| e.to_string())
}

#[tauri::command]
fn history_delete(state: State<AppState>, id: u64) -> Result<(), String> {
    match &state.history {
        Some(h) => h.delete(id).map_err(|e| e.to_string()),
        None => Ok(()),
    }
}

#[tauri::command]
fn history_clear(state: State<AppState>) -> Result<(), String> {
    match &state.history {
        Some(h) => h.clear().map_err(|e| e.to_string()),
        None => Ok(()),
    }
}

#[tauri::command]
fn copy_text(state: State<AppState>, text: String) -> Result<(), String> {
    state
        .paster
        .lock()
        .unwrap()
        .copy(&text)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn toggle_dictation(app: AppHandle, state: State<AppState>) {
    let phase = state.session.lock().unwrap().phase;
    match phase {
        Phase::Idle => start_recording(&app, Mode::Dictate),
        Phase::Recording => stop_recording(&app),
        _ => {}
    }
}

#[tauri::command]
fn toggle_agent(app: AppHandle, state: State<AppState>) {
    let phase = state.session.lock().unwrap().phase;
    match phase {
        Phase::Idle => start_recording(&app, Mode::Agent),
        Phase::Recording => stop_recording(&app),
        Phase::Thinking | Phase::Speaking => agent::interrupt(&app),
        Phase::Transcribing => {}
    }
}

#[tauri::command]
fn get_status(state: State<AppState>) -> Status {
    Status {
        phase: state.session.lock().unwrap().phase,
        message: None,
    }
}

#[tauri::command]
fn app_info() -> serde_json::Value {
    serde_json::json!({
        "version": env!("CARGO_PKG_VERSION"),
        "dataDir": kemcho_core::paths::data_dir().ok(),
        "os": std::env::consts::OS,
    })
}

pub fn run() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "kem_cho_lib=info,kemcho_core=info,whisper_rs=warn".into()),
        )
        .try_init();
    whisper_rs::install_logging_hooks();

    let settings = Settings::load();
    let history = History::open()
        .inspect_err(|e| tracing::warn!("history unavailable: {e:#}"))
        .ok();
    if let Some(h) = &history {
        let _ = h.prune(settings.history_days);
    }
    let state = AppState {
        settings: Mutex::new(settings),
        session: Mutex::new(Session {
            phase: Phase::Idle,
            mode: Mode::Dictate,
            started: None,
        }),
        recorder: audio::Recorder::new(),
        transcriber: Mutex::new(None),
        history,
        paster: Mutex::new(paste::Paster::default()),
        conversation: Mutex::new(Conversation::default()),
        cancel: Arc::new(AtomicBool::new(false)),
    };

    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _, _| {
            show_main(app)
        }))
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, shortcut, event| {
                    on_shortcut(app, shortcut, event.state == ShortcutState::Pressed);
                })
                .build(),
        )
        .manage(state)
        .setup(|app| {
            let handle = app.handle().clone();
            let open = MenuItem::with_id(app, "open", "Open Kem Cho", true, None::<&str>)?;
            let dictate =
                MenuItem::with_id(app, "dictate", "Start / stop dictation", true, None::<&str>)?;
            let ask = MenuItem::with_id(app, "ask", "Ask Kem Cho", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "Quit Kem Cho", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&open, &dictate, &ask, &quit])?;
            let mut tray = TrayIconBuilder::with_id("main")
                .tooltip("Kem Cho")
                .menu(&menu)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "open" => show_main(app),
                    "dictate" => {
                        let state = app.state::<AppState>();
                        toggle_dictation(app.clone(), state);
                    }
                    "ask" => {
                        let state = app.state::<AppState>();
                        toggle_agent(app.clone(), state);
                    }
                    "quit" => app.exit(0),
                    _ => {}
                });
            if let Some(icon) = app.default_window_icon() {
                tray = tray.icon(icon.clone());
            }
            if let Err(err) = tray.build(app) {
                tracing::warn!("no tray icon: {err}");
            }

            let settings = handle.state::<AppState>().settings.lock().unwrap().clone();
            if let Err(err) = register_shortcuts(&handle, &settings) {
                tracing::warn!("{err}");
            }
            preload(&handle);
            Ok(())
        })
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event
                && window.label() == "main"
            {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .invoke_handler(tauri::generate_handler![
            get_settings,
            save_settings,
            suspend_shortcut,
            list_models,
            download_model,
            delete_model,
            list_microphones,
            history_list,
            history_delete,
            history_clear,
            copy_text,
            toggle_dictation,
            toggle_agent,
            agent::agent_status,
            agent::agent_pull,
            agent::list_voices,
            agent::download_voice,
            agent::preview_voice,
            agent::agent_ask,
            agent::agent_stop,
            agent::agent_reset,
            get_status,
            app_info,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Kem Cho");
}
