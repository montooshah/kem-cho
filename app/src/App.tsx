import { useCallback, useEffect, useMemo, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import {
  api,
  acceleratorFromEvent,
  formatBytes,
  prettyShortcut,
  type AgentStatus,
  type AppInfo,
  type Entry,
  type Language,
  type ModelInfo,
  type Progress,
  type PullProgress,
  type Settings,
  type Status,
  type TokenEvent,
  type TurnDone,
  type TurnEvent,
  type Voices,
} from "./api";

type Pane = "general" | "agent" | "output" | "history" | "about";

const PANES: { id: Pane; label: string }[] = [
  { id: "general", label: "General" },
  { id: "agent", label: "Agent" },
  { id: "output", label: "Output" },
  { id: "history", label: "History" },
  { id: "about", label: "About" },
];

export default function App() {
  const [pane, setPane] = useState<Pane>("general");
  const [settings, setSettings] = useState<Settings | null>(null);
  const [models, setModels] = useState<ModelInfo[]>([]);
  const [progress, setProgress] = useState<Record<string, number>>({});
  const [status, setStatus] = useState<Status>({
    phase: "idle",
    message: null,
  });
  const [last, setLast] = useState<Entry | null>(null);
  const [info, setInfo] = useState<AppInfo | null>(null);
  const [error, setError] = useState<string | null>(null);

  const refreshModels = useCallback(() => api.listModels().then(setModels), []);

  useEffect(() => {
    api.getSettings().then(setSettings);
    api.getStatus().then(setStatus);
    api.appInfo().then(setInfo);
    refreshModels();
    const offs = [
      listen<Status>("status", (e) => setStatus(e.payload)),
      listen<Entry>("dictation", (e) => setLast(e.payload)),
      listen<Progress>("model-progress", (e) =>
        setProgress((p) => ({
          ...p,
          [e.payload.id]: e.payload.done / Math.max(1, e.payload.total),
        })),
      ),
      listen<{ id: string; error: string | null }>("model-done", (e) => {
        setProgress((p) => {
          const next = { ...p };
          delete next[e.payload.id];
          return next;
        });
        if (e.payload.error) setError(e.payload.error);
        refreshModels();
      }),
    ];
    return () => {
      offs.forEach((p) => p.then((off) => off()));
    };
  }, [refreshModels]);

  const save = useCallback(async (next: Settings) => {
    setSettings(next);
    try {
      setSettings(await api.saveSettings(next));
      setError(null);
    } catch (e) {
      setError(String(e));
      setSettings(await api.getSettings());
    }
  }, []);

  if (!settings || !info) return <div className="loading">Kem Cho…</div>;

  const current = models.find((m) => m.id === settings.model);
  const needsModel = current && !current.installed;

  return (
    <div className="shell">
      <nav className="sidebar">
        <div className="brand">
          <img src="/favicon.svg" alt="" />
          <span>Kem Cho</span>
        </div>
        {PANES.map((p) => (
          <button
            key={p.id}
            className={`nav ${pane === p.id ? "active" : ""}`}
            onClick={() => setPane(p.id)}
          >
            {p.label}
          </button>
        ))}
        <div className="sidebar-foot">v{info.version} · on-device</div>
      </nav>
      <main className="content">
        {error && (
          <div className="banner error" onClick={() => setError(null)}>
            {error}
          </div>
        )}
        {needsModel && pane === "general" && (
          <div className="banner">
            Kem Cho needs a speech model ({formatBytes(current.size_bytes)})
            before it can listen. It downloads once; after that nothing leaves
            your computer.
            <button
              className="primary"
              disabled={current.id in progress}
              onClick={() =>
                api.downloadModel(current.id).catch((e) => setError(String(e)))
              }
            >
              {current.id in progress
                ? `Downloading ${Math.round(progress[current.id] * 100)}%`
                : "Download"}
            </button>
          </div>
        )}
        {pane === "general" && (
          <General
            settings={settings}
            save={save}
            models={models}
            progress={progress}
            status={status}
            last={last}
            os={info.os}
            onError={setError}
            refreshModels={refreshModels}
          />
        )}
        {/* Kept mounted so replies to "Kem cho, …" dictations aren't lost. */}
        <div hidden={pane !== "agent"}>
          <Agent
            settings={settings}
            save={save}
            status={status}
            os={info.os}
            onError={setError}
          />
        </div>
        {pane === "output" && <Output settings={settings} save={save} />}
        {pane === "history" && (
          <History settings={settings} save={save} last={last} />
        )}
        {pane === "about" && <About info={info} />}
      </main>
    </div>
  );
}

const PHASE_LABELS: Record<Status["phase"], string> = {
  idle: "Ready",
  recording: "Listening…",
  transcribing: "Transcribing…",
  thinking: "Thinking…",
  speaking: "Speaking…",
};

interface Exchange {
  id: number;
  question: string;
  reply: string;
  error: string | null;
  done: boolean;
}

function Agent({
  settings,
  save,
  status,
  os,
  onError,
}: {
  settings: Settings;
  save: (s: Settings) => void;
  status: Status;
  os: string;
  onError: (e: string) => void;
}) {
  const [agent, setAgent] = useState<AgentStatus | null>(null);
  const [voices, setVoices] = useState<Voices | null>(null);
  const [pull, setPull] = useState<PullProgress | null>(null);
  const [voicePct, setVoicePct] = useState<Record<string, number>>({});
  const [chat, setChat] = useState<Exchange[]>([]);
  const [typed, setTyped] = useState("");
  const [warning, setWarning] = useState<string | null>(null);

  const refresh = useCallback(() => {
    api.agentStatus().then(setAgent);
    api.listVoices().then(setVoices);
  }, []);

  useEffect(() => {
    refresh();
    const offs = [
      listen<TurnEvent>("agent-turn", (e) =>
        setChat((c) => [
          ...c,
          { ...e.payload, reply: "", error: null, done: false },
        ]),
      ),
      listen<TokenEvent>("agent-token", (e) =>
        setChat((c) =>
          c.map((x) =>
            x.id === e.payload.id
              ? { ...x, reply: x.reply + e.payload.token }
              : x,
          ),
        ),
      ),
      listen<TurnDone>("agent-done", (e) =>
        setChat((c) =>
          c.map((x) =>
            x.id === e.payload.id
              ? {
                  ...x,
                  reply: e.payload.reply,
                  error: e.payload.error,
                  done: true,
                }
              : x,
          ),
        ),
      ),
      listen<string>("agent-warning", (e) => setWarning(e.payload)),
      listen<PullProgress>("agent-pull", (e) => setPull(e.payload)),
      listen<{ model: string; error: string | null }>(
        "agent-pull-done",
        (e) => {
          setPull(null);
          if (e.payload.error) onError(e.payload.error);
          refresh();
        },
      ),
      listen<Progress>("voice-progress", (e) =>
        setVoicePct((p) => ({
          ...p,
          [e.payload.id]: e.payload.done / Math.max(1, e.payload.total),
        })),
      ),
      listen<{ id: string; error: string | null }>("voice-done", (e) => {
        setVoicePct((p) => {
          const next = { ...p };
          delete next[e.payload.id];
          return next;
        });
        if (e.payload.error) onError(e.payload.error);
        refresh();
      }),
    ];
    return () => {
      offs.forEach((p) => p.then((off) => off()));
    };
  }, [refresh, onError]);

  const installed = new Set(agent?.installed.map((m) => m.name) ?? []);
  const options = [
    ...(agent?.suggested ?? []),
    ...(agent?.installed ?? [])
      .filter((m) => !agent?.suggested.some((s) => s.id === m.name))
      .map((m) => ({
        id: m.name,
        label: `${m.name} · ${formatBytes(m.size)}`,
      })),
  ];
  const modelReady = installed.has(settings.agent_model);
  const voice = voices?.voices.find((v) => v.id === settings.agent_voice);
  const voiceReady = !voices?.neural || voice?.installed;
  const busy = status.phase === "thinking" || status.phase === "speaking";
  const ask = () => {
    if (!typed.trim()) return;
    setWarning(null);
    api.agentAsk(typed).catch((e) => onError(String(e)));
    setTyped("");
  };

  return (
    <section>
      <h1>Agent</h1>
      <p className="muted">
        Ask a question out loud and Kem Cho answers out loud. Your voice, the
        language model and the reply voice all run on this computer.
      </p>

      {agent && !agent.running && (
        <div className="banner">
          Kem Cho uses Ollama to think. Install it from{" "}
          <a
            href="https://ollama.com/download"
            target="_blank"
            rel="noreferrer"
          >
            ollama.com
          </a>
          , open it, then check again.
          {agent.error && <div className="muted">{agent.error}</div>}
          <button className="primary" onClick={refresh}>
            Check again
          </button>
        </div>
      )}
      {agent?.running && !modelReady && (
        <div className="banner">
          {settings.agent_model} isn’t downloaded yet.
          <button
            className="primary"
            disabled={!!pull}
            onClick={() => api.agentPull(settings.agent_model)}
          >
            {pull
              ? `${pull.status} ${pull.total ? Math.round((pull.done / pull.total) * 100) + "%" : ""}`
              : "Download"}
          </button>
        </div>
      )}
      {voices?.neural && voice && !voice.installed && (
        <div className="banner">
          The {voice.label} voice ({formatBytes(voice.size_bytes)}) needs
          downloading before Kem Cho can speak.
          <button
            className="primary"
            disabled={voice.id in voicePct}
            onClick={() =>
              api.downloadVoice(voice.id).catch((e) => onError(String(e)))
            }
          >
            {voice.id in voicePct
              ? `Downloading ${Math.round(voicePct[voice.id] * 100)}%`
              : "Download"}
          </button>
        </div>
      )}

      <div className="hero">
        <div>
          <div className={`phase ${status.phase}`}>
            {PHASE_LABELS[status.phase]}
          </div>
          <p>
            {settings.shortcut_mode === "hold" ? "Hold" : "Press"}{" "}
            <kbd>{prettyShortcut(settings.agent_shortcut, os)}</kbd> and ask
            anything. Press it again while Kem Cho talks to stop it.
          </p>
          {status.message && <p className="muted">{status.message}</p>}
        </div>
        <button
          className="primary big"
          disabled={status.phase === "transcribing" || !modelReady}
          onClick={() => (busy ? api.agentStop() : api.toggleAgent())}
        >
          {status.phase === "recording" ? "Done" : busy ? "Stop" : "Ask"}
        </button>
      </div>

      <div className="chat">
        {chat.map((x) => (
          <div key={x.id} className="exchange">
            <div className="bubble you">{x.question}</div>
            <div className={`bubble kem ${x.error ? "error" : ""}`}>
              {x.error ??
                (x.reply ||
                  (x.done ? <em className="muted">(stopped)</em> : "…"))}
            </div>
          </div>
        ))}
        {warning && <p className="muted">{warning}</p>}
        <div className="replacement">
          <input
            value={typed}
            placeholder="Or type a question"
            disabled={!modelReady}
            onChange={(e) => setTyped(e.target.value)}
            onKeyDown={(e) => e.key === "Enter" && ask()}
          />
          <button
            onClick={ask}
            disabled={!modelReady || status.phase !== "idle"}
          >
            Ask
          </button>
          {chat.length > 0 && (
            <button
              className="ghost"
              onClick={() => {
                api.agentReset();
                setChat([]);
              }}
            >
              New conversation
            </button>
          )}
        </div>
      </div>

      <h2>Settings</h2>
      <Row
        title="Agent shortcut"
        hint="Works in any app. Uses the same hold or tap mode as dictation."
      >
        <ShortcutField
          value={settings.agent_shortcut}
          os={os}
          onChange={(agent_shortcut) => save({ ...settings, agent_shortcut })}
        />
      </Row>
      <Row
        title="Language model"
        hint={
          agent?.version
            ? `Ollama ${agent.version} on this computer.`
            : "Runs in Ollama."
        }
      >
        <select
          value={settings.agent_model}
          onChange={(e) => save({ ...settings, agent_model: e.target.value })}
        >
          {options.map((m) => (
            <option key={m.id} value={m.id}>
              {installed.has(m.id) ? "✓ " : ""}
              {m.label}
            </option>
          ))}
        </select>
      </Row>
      <Row
        title="“Kem cho” commands"
        hint="Start a dictation with “Kem cho” to ask a question, “Kem cho, write …” to have text written for you, or select text and say “Kem cho, make this formal”."
      >
        <Toggle
          checked={settings.agent_commands}
          onChange={(agent_commands) => save({ ...settings, agent_commands })}
        />
      </Row>
      <Row title="Speak replies" hint="Off: replies are only shown here.">
        <Toggle
          checked={settings.speak_replies}
          onChange={(speak_replies) => save({ ...settings, speak_replies })}
        />
      </Row>
      {voices?.neural ? (
        <Row title="Voice" hint="Piper neural voices, run on this computer.">
          <select
            value={settings.agent_voice}
            onChange={(e) => save({ ...settings, agent_voice: e.target.value })}
          >
            {voices.voices.map((v) => (
              <option key={v.id} value={v.id}>
                {v.installed ? "✓ " : ""}
                {v.label}
              </option>
            ))}
          </select>
          <button
            className="ghost"
            disabled={!voiceReady}
            onClick={() =>
              api
                .previewVoice(settings.agent_voice)
                .catch((e) => onError(String(e)))
            }
          >
            Preview
          </button>
        </Row>
      ) : (
        <Row title="Voice" hint="Uses your system voice.">
          <button
            className="ghost"
            onClick={() =>
              api
                .previewVoice(settings.agent_voice)
                .catch((e) => onError(String(e)))
            }
          >
            Preview
          </button>
        </Row>
      )}
      <Row
        title="Personality"
        hint="How Kem Cho should answer. Leave empty for short, friendly spoken answers."
      >
        <span />
      </Row>
      <textarea
        className="personality"
        rows={3}
        defaultValue={settings.agent_personality}
        placeholder="You are Kem Cho, a warm, helpful voice assistant…"
        onBlur={(e) =>
          e.target.value !== settings.agent_personality &&
          save({ ...settings, agent_personality: e.target.value })
        }
      />
      <Row
        title="Ollama address"
        hint="Change only if Ollama runs on another port."
      >
        <input
          defaultValue={settings.ollama_url}
          onBlur={(e) => {
            if (e.target.value !== settings.ollama_url) {
              save({ ...settings, ollama_url: e.target.value });
              setTimeout(refresh, 300);
            }
          }}
        />
      </Row>
    </section>
  );
}

function Row({
  title,
  hint,
  children,
}: {
  title: string;
  hint?: string;
  children: React.ReactNode;
}) {
  return (
    <div className="row">
      <div className="row-text">
        <div className="row-title">{title}</div>
        {hint && <div className="row-hint">{hint}</div>}
      </div>
      <div className="row-control">{children}</div>
    </div>
  );
}

function Toggle({
  checked,
  onChange,
}: {
  checked: boolean;
  onChange: (v: boolean) => void;
}) {
  return (
    <button
      role="switch"
      aria-checked={checked}
      className={`switch ${checked ? "on" : ""}`}
      onClick={() => onChange(!checked)}
    >
      <span />
    </button>
  );
}

function ShortcutField({
  value,
  os,
  onChange,
}: {
  value: string;
  os: string;
  onChange: (accelerator: string) => void;
}) {
  const [capturing, setCapturing] = useState(false);

  useEffect(() => {
    if (!capturing) return;
    api.suspendShortcut(true);
    const onKey = (e: KeyboardEvent) => {
      e.preventDefault();
      if (e.code === "Escape") {
        setCapturing(false);
        return;
      }
      const acc = acceleratorFromEvent(e);
      if (acc) {
        setCapturing(false);
        onChange(acc);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("keydown", onKey);
      api.suspendShortcut(false);
    };
  }, [capturing, onChange]);

  return (
    <button
      className={`shortcut ${capturing ? "capturing" : ""}`}
      onClick={() => setCapturing((c) => !c)}
    >
      {capturing ? (
        "Press a shortcut… (Esc to cancel)"
      ) : (
        <kbd>{prettyShortcut(value, os)}</kbd>
      )}
    </button>
  );
}

function General({
  settings,
  save,
  models,
  progress,
  status,
  last,
  os,
  onError,
  refreshModels,
}: {
  settings: Settings;
  save: (s: Settings) => void;
  models: ModelInfo[];
  progress: Record<string, number>;
  status: Status;
  last: Entry | null;
  os: string;
  onError: (e: string) => void;
  refreshModels: () => void;
}) {
  const [mics, setMics] = useState<string[]>([]);
  const [languages, setLanguages] = useState<Language[]>([]);
  useEffect(() => {
    api.listMicrophones().then(setMics);
    api.listLanguages().then(setLanguages);
  }, []);
  const language = languages.find((l) => l.id === settings.language);
  const usable = models.filter(
    (m) =>
      !language ||
      m.languages.length === 0 ||
      m.languages.includes(language.whisper),
  );

  const verb = settings.shortcut_mode === "hold" ? "Hold" : "Press";
  const phaseLabel = PHASE_LABELS[status.phase];

  return (
    <section>
      <h1>General</h1>
      <div className="hero">
        <div>
          <div className={`phase ${status.phase}`}>{phaseLabel}</div>
          <p>
            {verb} <kbd>{prettyShortcut(settings.shortcut, os)}</kbd>, speak,
            and{" "}
            {settings.shortcut_mode === "hold" ? "let go" : "press it again"}.
            Kem Cho{" "}
            {settings.paste
              ? "types what you said where your cursor is"
              : "copies what you said"}
            .
          </p>
          {status.message && <p className="muted">{status.message}</p>}
          {last && <blockquote>{last.text}</blockquote>}
        </div>
        <button
          className="primary big"
          onClick={() => api.toggleDictation()}
          disabled={status.phase !== "idle" && status.phase !== "recording"}
        >
          {status.phase === "recording" ? "Stop" : "Try it"}
        </button>
      </div>

      <h2>Shortcut</h2>
      <Row title="Dictation shortcut" hint="Works in any app.">
        <ShortcutField
          value={settings.shortcut}
          os={os}
          onChange={(shortcut) => save({ ...settings, shortcut })}
        />
      </Row>
      <Row
        title="Shortcut mode"
        hint="Hold to talk, or tap to start and tap to stop."
      >
        <select
          value={settings.shortcut_mode}
          onChange={(e) =>
            save({
              ...settings,
              shortcut_mode: e.target.value as Settings["shortcut_mode"],
            })
          }
        >
          <option value="hold">Hold</option>
          <option value="toggle">Tap</option>
        </select>
      </Row>

      <h2>Input and output</h2>
      <Row title="Microphone">
        <select
          value={settings.microphone ?? ""}
          onChange={(e) =>
            save({ ...settings, microphone: e.target.value || null })
          }
        >
          <option value="">System default</option>
          {mics.map((m) => (
            <option key={m} value={m}>
              {m}
            </option>
          ))}
        </select>
      </Row>
      <Row
        title="Paste at the cursor"
        hint="Off: the text is only copied to the clipboard."
      >
        <Toggle
          checked={settings.paste}
          onChange={(paste) => save({ ...settings, paste })}
        />
      </Row>
      <Row
        title="Keep my clipboard"
        hint="Put back what was on the clipboard after pasting."
      >
        <Toggle
          checked={settings.restore_clipboard}
          onChange={(restore_clipboard) =>
            save({ ...settings, restore_clipboard })
          }
        />
      </Row>

      <h2>Language</h2>
      <Row title="I speak" hint={language?.hint || undefined}>
        <select
          value={settings.language}
          onChange={(e) => {
            const next = languages.find((l) => l.id === e.target.value);
            if (!next) return;
            const current = models.find((m) => m.id === settings.model);
            // A model the person picked themselves is kept when it fits; the
            // old language's default gives way to the new language's default.
            const fits =
              current &&
              current.id !== language?.default_model &&
              (current.languages.length === 0 ||
                current.languages.includes(next.whisper));
            save({
              ...settings,
              language: next.id,
              model: fits ? settings.model : next.default_model,
            });
          }}
        >
          {languages.map((l) => (
            <option key={l.id} value={l.id}>
              {l.label}
            </option>
          ))}
        </select>
      </Row>

      <h2>Speech model</h2>
      <div className="models">
        {usable.map((m) => {
          const pct = progress[m.id];
          const selected = m.id === settings.model;
          return (
            <div key={m.id} className={`model ${selected ? "selected" : ""}`}>
              <label>
                <input
                  type="radio"
                  name="model"
                  checked={selected}
                  onChange={() => save({ ...settings, model: m.id })}
                />
                <div>
                  <div className="row-title">
                    {m.label}{" "}
                    <span className="muted">{formatBytes(m.size_bytes)}</span>
                  </div>
                  <div className="row-hint">{m.description}</div>
                </div>
              </label>
              {pct !== undefined ? (
                <div className="bar">
                  <span style={{ width: `${pct * 100}%` }} />
                </div>
              ) : m.installed ? (
                <button
                  className="ghost"
                  onClick={() =>
                    api
                      .deleteModel(m.id)
                      .then(refreshModels)
                      .catch((e) => onError(String(e)))
                  }
                >
                  Remove
                </button>
              ) : (
                <button
                  className="ghost"
                  onClick={() =>
                    api.downloadModel(m.id).catch((e) => onError(String(e)))
                  }
                >
                  Download
                </button>
              )}
            </div>
          );
        })}
      </div>
    </section>
  );
}

function ListEditor({
  items,
  placeholder,
  onChange,
}: {
  items: string[];
  placeholder: string;
  onChange: (items: string[]) => void;
}) {
  const [draft, setDraft] = useState("");
  const add = () => {
    const v = draft.trim();
    if (v && !items.includes(v)) onChange([...items, v]);
    setDraft("");
  };
  return (
    <div className="list-editor">
      <div className="chips">
        {items.map((w) => (
          <span key={w} className="chip">
            {w}
            <button
              aria-label={`Remove ${w}`}
              onClick={() => onChange(items.filter((x) => x !== w))}
            >
              ×
            </button>
          </span>
        ))}
      </div>
      <div className="inline">
        <input
          value={draft}
          placeholder={placeholder}
          onChange={(e) => setDraft(e.target.value)}
          onKeyDown={(e) => e.key === "Enter" && add()}
        />
        <button onClick={add}>Add</button>
      </div>
    </div>
  );
}

function Output({
  settings,
  save,
}: {
  settings: Settings;
  save: (s: Settings) => void;
}) {
  const [from, setFrom] = useState("");
  const [to, setTo] = useState("");
  const addReplacement = () => {
    if (!from.trim()) return;
    save({
      ...settings,
      replacements: [...settings.replacements, { from: from.trim(), to }],
    });
    setFrom("");
    setTo("");
  };
  return (
    <section>
      <h1>Output</h1>
      <p className="muted">
        Every dictation runs through these steps, in order, before it is pasted.
      </p>
      <Row title="Remove filler words" hint="Drops um, uh, erm and hmm.">
        <Toggle
          checked={settings.remove_fillers}
          onChange={(remove_fillers) => save({ ...settings, remove_fillers })}
        />
      </Row>
      <Row
        title="Word replacements"
        hint="Whole words, any case. “kem cho” → “Kem Cho”."
      >
        <span />
      </Row>
      <div className="replacements">
        {settings.replacements.map((r, i) => (
          <div key={i} className="replacement">
            <span>{r.from}</span>
            <span className="muted">→</span>
            <span>{r.to || <em className="muted">(delete)</em>}</span>
            <button
              className="ghost"
              onClick={() =>
                save({
                  ...settings,
                  replacements: settings.replacements.filter((_, j) => j !== i),
                })
              }
            >
              Remove
            </button>
          </div>
        ))}
        <div className="replacement">
          <input
            value={from}
            placeholder="When I say"
            onChange={(e) => setFrom(e.target.value)}
          />
          <span className="muted">→</span>
          <input
            value={to}
            placeholder="Write"
            onChange={(e) => setTo(e.target.value)}
            onKeyDown={(e) => e.key === "Enter" && addReplacement()}
          />
          <button onClick={addReplacement}>Add</button>
        </div>
      </div>
      <Row title="Voice commands" hint="Say “new line” or “new paragraph”.">
        <Toggle
          checked={settings.voice_commands}
          onChange={(voice_commands) => save({ ...settings, voice_commands })}
        />
      </Row>
      <h2>Vocabulary</h2>
      <p className="muted">Names and terms the speech model should expect.</p>
      <ListEditor
        items={settings.vocabulary}
        placeholder="Add a name or term"
        onChange={(vocabulary) => save({ ...settings, vocabulary })}
      />
    </section>
  );
}

function History({
  settings,
  save,
  last,
}: {
  settings: Settings;
  save: (s: Settings) => void;
  last: Entry | null;
}) {
  const [query, setQuery] = useState("");
  const [entries, setEntries] = useState<Entry[]>([]);
  const [copied, setCopied] = useState<number | null>(null);
  const load = useCallback(
    () => api.historyList(query).then(setEntries),
    [query],
  );
  useEffect(() => {
    load();
  }, [load, last]);

  const grouped = useMemo(() => {
    const groups = new Map<string, Entry[]>();
    for (const e of entries) {
      const day = new Date(e.id).toLocaleDateString(undefined, {
        weekday: "long",
        month: "short",
        day: "numeric",
      });
      groups.set(day, [...(groups.get(day) ?? []), e]);
    }
    return [...groups.entries()];
  }, [entries]);

  return (
    <section>
      <h1>History</h1>
      <div className="inline">
        <input
          className="search"
          placeholder="Search dictations"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
        />
        <select
          value={settings.history_days}
          onChange={(e) =>
            save({ ...settings, history_days: Number(e.target.value) })
          }
        >
          <option value={0}>Don't keep</option>
          <option value={1}>Keep 1 day</option>
          <option value={7}>Keep 7 days</option>
          <option value={30}>Keep 30 days</option>
          <option value={365}>Keep 1 year</option>
        </select>
        <button
          className="ghost"
          onClick={() => api.historyClear().then(load)}
          disabled={!entries.length}
        >
          Clear all
        </button>
      </div>
      {!entries.length && <p className="muted">Your dictations appear here.</p>}
      {grouped.map(([day, items]) => (
        <div key={day}>
          <h2>{day}</h2>
          {items.map((e) => (
            <div key={e.id} className="entry">
              <div className="entry-text">{e.text}</div>
              <div className="entry-meta">
                {new Date(e.id).toLocaleTimeString(undefined, {
                  hour: "numeric",
                  minute: "2-digit",
                })}{" "}
                · {(e.audio_ms / 1000).toFixed(1)}s audio · {e.transcribe_ms} ms
                · {e.model}
              </div>
              <div className="entry-actions">
                <button
                  className="ghost"
                  onClick={() =>
                    api.copyText(e.text).then(() => setCopied(e.id))
                  }
                >
                  {copied === e.id ? "Copied" : "Copy"}
                </button>
                <button
                  className="ghost"
                  onClick={() => api.historyDelete(e.id).then(load)}
                >
                  Delete
                </button>
              </div>
            </div>
          ))}
        </div>
      ))}
    </section>
  );
}

function About({ info }: { info: AppInfo }) {
  return (
    <section>
      <h1>About Kem Cho</h1>
      <p>
        Kem Cho (“how are you?” in Gujarati) is private voice dictation. Speech
        is transcribed on this computer with OpenAI's Whisper running in
        whisper.cpp; audio never leaves your machine.
      </p>
      <Row title="Version">{info.version}</Row>
      <Row title="Data folder" hint="Settings, history and models.">
        <code>{info.dataDir}</code>
      </Row>
      <h2>Coming next</h2>
      <ul>
        <li>Voice agent: ask a local LLM and hear the answer spoken back.</li>
        <li>Gujarati and Hindi dictation.</li>
      </ul>
      <p className="muted">
        Inspired by <a href="https://github.com/anomalyco/hex">Hex</a> (MIT).
        Open source at{" "}
        <a href="https://github.com/montooshah/kem-cho">
          github.com/montooshah/kem-cho
        </a>
        .
      </p>
    </section>
  );
}
