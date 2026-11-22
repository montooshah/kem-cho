import { useCallback, useEffect, useMemo, useState } from 'react'
import { listen } from '@tauri-apps/api/event'
import {
  api,
  acceleratorFromEvent,
  formatBytes,
  prettyShortcut,
  type AppInfo,
  type Entry,
  type ModelInfo,
  type Progress,
  type Settings,
  type Status,
} from './api'

type Pane = 'general' | 'output' | 'history' | 'about'

const PANES: { id: Pane; label: string }[] = [
  { id: 'general', label: 'General' },
  { id: 'output', label: 'Output' },
  { id: 'history', label: 'History' },
  { id: 'about', label: 'About' },
]

export default function App() {
  const [pane, setPane] = useState<Pane>('general')
  const [settings, setSettings] = useState<Settings | null>(null)
  const [models, setModels] = useState<ModelInfo[]>([])
  const [progress, setProgress] = useState<Record<string, number>>({})
  const [status, setStatus] = useState<Status>({ phase: 'idle', message: null })
  const [last, setLast] = useState<Entry | null>(null)
  const [info, setInfo] = useState<AppInfo | null>(null)
  const [error, setError] = useState<string | null>(null)

  const refreshModels = useCallback(() => api.listModels().then(setModels), [])

  useEffect(() => {
    api.getSettings().then(setSettings)
    api.getStatus().then(setStatus)
    api.appInfo().then(setInfo)
    refreshModels()
    const offs = [
      listen<Status>('status', (e) => setStatus(e.payload)),
      listen<Entry>('dictation', (e) => setLast(e.payload)),
      listen<Progress>('model-progress', (e) =>
        setProgress((p) => ({ ...p, [e.payload.id]: e.payload.done / Math.max(1, e.payload.total) })),
      ),
      listen<{ id: string; error: string | null }>('model-done', (e) => {
        setProgress((p) => {
          const next = { ...p }
          delete next[e.payload.id]
          return next
        })
        if (e.payload.error) setError(e.payload.error)
        refreshModels()
      }),
    ]
    return () => {
      offs.forEach((p) => p.then((off) => off()))
    }
  }, [refreshModels])

  const save = useCallback(async (next: Settings) => {
    setSettings(next)
    try {
      setSettings(await api.saveSettings(next))
      setError(null)
    } catch (e) {
      setError(String(e))
      setSettings(await api.getSettings())
    }
  }, [])

  if (!settings || !info) return <div className="loading">Kem Cho…</div>

  const current = models.find((m) => m.id === settings.model)
  const needsModel = current && !current.installed

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
            className={`nav ${pane === p.id ? 'active' : ''}`}
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
        {needsModel && pane === 'general' && (
          <div className="banner">
            Kem Cho needs a speech model ({formatBytes(current.size_bytes)}) before it can listen. It downloads once; after
            that nothing leaves your computer.
            <button
              className="primary"
              disabled={current.id in progress}
              onClick={() => api.downloadModel(current.id).catch((e) => setError(String(e)))}
            >
              {current.id in progress ? `Downloading ${Math.round(progress[current.id] * 100)}%` : 'Download'}
            </button>
          </div>
        )}
        {pane === 'general' && (
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
        {pane === 'output' && <Output settings={settings} save={save} />}
        {pane === 'history' && <History settings={settings} save={save} last={last} />}
        {pane === 'about' && <About info={info} />}
      </main>
    </div>
  )
}

function Row({ title, hint, children }: { title: string; hint?: string; children: React.ReactNode }) {
  return (
    <div className="row">
      <div className="row-text">
        <div className="row-title">{title}</div>
        {hint && <div className="row-hint">{hint}</div>}
      </div>
      <div className="row-control">{children}</div>
    </div>
  )
}

function Toggle({ checked, onChange }: { checked: boolean; onChange: (v: boolean) => void }) {
  return (
    <button
      role="switch"
      aria-checked={checked}
      className={`switch ${checked ? 'on' : ''}`}
      onClick={() => onChange(!checked)}
    >
      <span />
    </button>
  )
}

function ShortcutField({
  value,
  os,
  onChange,
}: {
  value: string
  os: string
  onChange: (accelerator: string) => void
}) {
  const [capturing, setCapturing] = useState(false)

  useEffect(() => {
    if (!capturing) return
    api.suspendShortcut(true)
    const onKey = (e: KeyboardEvent) => {
      e.preventDefault()
      if (e.code === 'Escape') {
        setCapturing(false)
        return
      }
      const acc = acceleratorFromEvent(e)
      if (acc) {
        setCapturing(false)
        onChange(acc)
      }
    }
    window.addEventListener('keydown', onKey)
    return () => {
      window.removeEventListener('keydown', onKey)
      api.suspendShortcut(false)
    }
  }, [capturing, onChange])

  return (
    <button className={`shortcut ${capturing ? 'capturing' : ''}`} onClick={() => setCapturing((c) => !c)}>
      {capturing ? 'Press a shortcut… (Esc to cancel)' : <kbd>{prettyShortcut(value, os)}</kbd>}
    </button>
  )
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
  settings: Settings
  save: (s: Settings) => void
  models: ModelInfo[]
  progress: Record<string, number>
  status: Status
  last: Entry | null
  os: string
  onError: (e: string) => void
  refreshModels: () => void
}) {
  const [mics, setMics] = useState<string[]>([])
  useEffect(() => {
    api.listMicrophones().then(setMics)
  }, [])

  const verb = settings.shortcut_mode === 'hold' ? 'Hold' : 'Press'
  const phaseLabel = { idle: 'Ready', recording: 'Listening…', transcribing: 'Transcribing…' }[status.phase]

  return (
    <section>
      <h1>General</h1>
      <div className="hero">
        <div>
          <div className={`phase ${status.phase}`}>{phaseLabel}</div>
          <p>
            {verb} <kbd>{prettyShortcut(settings.shortcut, os)}</kbd>, speak, and{' '}
            {settings.shortcut_mode === 'hold' ? 'let go' : 'press it again'}. Kem Cho{' '}
            {settings.paste ? 'types what you said where your cursor is' : 'copies what you said'}.
          </p>
          {status.message && <p className="muted">{status.message}</p>}
          {last && <blockquote>{last.text}</blockquote>}
        </div>
        <button className="primary big" onClick={() => api.toggleDictation()} disabled={status.phase === 'transcribing'}>
          {status.phase === 'recording' ? 'Stop' : 'Try it'}
        </button>
      </div>

      <h2>Shortcut</h2>
      <Row title="Dictation shortcut" hint="Works in any app.">
        <ShortcutField value={settings.shortcut} os={os} onChange={(shortcut) => save({ ...settings, shortcut })} />
      </Row>
      <Row title="Shortcut mode" hint="Hold to talk, or tap to start and tap to stop.">
        <select
          value={settings.shortcut_mode}
          onChange={(e) => save({ ...settings, shortcut_mode: e.target.value as Settings['shortcut_mode'] })}
        >
          <option value="hold">Hold</option>
          <option value="toggle">Tap</option>
        </select>
      </Row>

      <h2>Input and output</h2>
      <Row title="Microphone">
        <select
          value={settings.microphone ?? ''}
          onChange={(e) => save({ ...settings, microphone: e.target.value || null })}
        >
          <option value="">System default</option>
          {mics.map((m) => (
            <option key={m} value={m}>
              {m}
            </option>
          ))}
        </select>
      </Row>
      <Row title="Paste at the cursor" hint="Off: the text is only copied to the clipboard.">
        <Toggle checked={settings.paste} onChange={(paste) => save({ ...settings, paste })} />
      </Row>
      <Row title="Keep my clipboard" hint="Put back what was on the clipboard after pasting.">
        <Toggle
          checked={settings.restore_clipboard}
          onChange={(restore_clipboard) => save({ ...settings, restore_clipboard })}
        />
      </Row>

      <h2>Speech model</h2>
      <div className="models">
        {models.map((m) => {
          const pct = progress[m.id]
          const selected = m.id === settings.model
          return (
            <div key={m.id} className={`model ${selected ? 'selected' : ''}`}>
              <label>
                <input
                  type="radio"
                  name="model"
                  checked={selected}
                  onChange={() =>
                    save({ ...settings, model: m.id, language: m.english_only ? 'en' : settings.language })
                  }
                />
                <div>
                  <div className="row-title">
                    {m.label} <span className="muted">{formatBytes(m.size_bytes)}</span>
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
                <button className="ghost" onClick={() => api.downloadModel(m.id).catch((e) => onError(String(e)))}>
                  Download
                </button>
              )}
            </div>
          )
        })}
      </div>
      {!models.find((m) => m.id === settings.model)?.english_only && (
        <Row title="Language" hint="Multilingual models only. Gujarati support is coming next.">
          <select value={settings.language} onChange={(e) => save({ ...settings, language: e.target.value })}>
            <option value="auto">Detect automatically</option>
            <option value="en">English</option>
            <option value="gu">Gujarati (ગુજરાતી)</option>
            <option value="hi">Hindi (हिन्दी)</option>
          </select>
        </Row>
      )}
    </section>
  )
}

function ListEditor({
  items,
  placeholder,
  onChange,
}: {
  items: string[]
  placeholder: string
  onChange: (items: string[]) => void
}) {
  const [draft, setDraft] = useState('')
  const add = () => {
    const v = draft.trim()
    if (v && !items.includes(v)) onChange([...items, v])
    setDraft('')
  }
  return (
    <div className="list-editor">
      <div className="chips">
        {items.map((w) => (
          <span key={w} className="chip">
            {w}
            <button aria-label={`Remove ${w}`} onClick={() => onChange(items.filter((x) => x !== w))}>
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
          onKeyDown={(e) => e.key === 'Enter' && add()}
        />
        <button onClick={add}>Add</button>
      </div>
    </div>
  )
}

function Output({ settings, save }: { settings: Settings; save: (s: Settings) => void }) {
  const [from, setFrom] = useState('')
  const [to, setTo] = useState('')
  const addReplacement = () => {
    if (!from.trim()) return
    save({ ...settings, replacements: [...settings.replacements, { from: from.trim(), to }] })
    setFrom('')
    setTo('')
  }
  return (
    <section>
      <h1>Output</h1>
      <p className="muted">Every dictation runs through these steps, in order, before it is pasted.</p>
      <Row title="Remove filler words" hint="Drops um, uh, erm and hmm.">
        <Toggle checked={settings.remove_fillers} onChange={(remove_fillers) => save({ ...settings, remove_fillers })} />
      </Row>
      <Row title="Word replacements" hint="Whole words, any case. “kem cho” → “Kem Cho”.">
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
              onClick={() => save({ ...settings, replacements: settings.replacements.filter((_, j) => j !== i) })}
            >
              Remove
            </button>
          </div>
        ))}
        <div className="replacement">
          <input value={from} placeholder="When I say" onChange={(e) => setFrom(e.target.value)} />
          <span className="muted">→</span>
          <input
            value={to}
            placeholder="Write"
            onChange={(e) => setTo(e.target.value)}
            onKeyDown={(e) => e.key === 'Enter' && addReplacement()}
          />
          <button onClick={addReplacement}>Add</button>
        </div>
      </div>
      <Row title="Voice commands" hint="Say “new line” or “new paragraph”.">
        <Toggle checked={settings.voice_commands} onChange={(voice_commands) => save({ ...settings, voice_commands })} />
      </Row>
      <h2>Vocabulary</h2>
      <p className="muted">Names and terms the speech model should expect.</p>
      <ListEditor
        items={settings.vocabulary}
        placeholder="Add a name or term"
        onChange={(vocabulary) => save({ ...settings, vocabulary })}
      />
    </section>
  )
}

function History({ settings, save, last }: { settings: Settings; save: (s: Settings) => void; last: Entry | null }) {
  const [query, setQuery] = useState('')
  const [entries, setEntries] = useState<Entry[]>([])
  const [copied, setCopied] = useState<number | null>(null)
  const load = useCallback(() => api.historyList(query).then(setEntries), [query])
  useEffect(() => {
    load()
  }, [load, last])

  const grouped = useMemo(() => {
    const groups = new Map<string, Entry[]>()
    for (const e of entries) {
      const day = new Date(e.id).toLocaleDateString(undefined, { weekday: 'long', month: 'short', day: 'numeric' })
      groups.set(day, [...(groups.get(day) ?? []), e])
    }
    return [...groups.entries()]
  }, [entries])

  return (
    <section>
      <h1>History</h1>
      <div className="inline">
        <input className="search" placeholder="Search dictations" value={query} onChange={(e) => setQuery(e.target.value)} />
        <select
          value={settings.history_days}
          onChange={(e) => save({ ...settings, history_days: Number(e.target.value) })}
        >
          <option value={0}>Don't keep</option>
          <option value={1}>Keep 1 day</option>
          <option value={7}>Keep 7 days</option>
          <option value={30}>Keep 30 days</option>
          <option value={365}>Keep 1 year</option>
        </select>
        <button className="ghost" onClick={() => api.historyClear().then(load)} disabled={!entries.length}>
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
                {new Date(e.id).toLocaleTimeString(undefined, { hour: 'numeric', minute: '2-digit' })} ·{' '}
                {(e.audio_ms / 1000).toFixed(1)}s audio · {e.transcribe_ms} ms · {e.model}
              </div>
              <div className="entry-actions">
                <button
                  className="ghost"
                  onClick={() => api.copyText(e.text).then(() => setCopied(e.id))}
                >
                  {copied === e.id ? 'Copied' : 'Copy'}
                </button>
                <button className="ghost" onClick={() => api.historyDelete(e.id).then(load)}>
                  Delete
                </button>
              </div>
            </div>
          ))}
        </div>
      ))}
    </section>
  )
}

function About({ info }: { info: AppInfo }) {
  return (
    <section>
      <h1>About Kem Cho</h1>
      <p>
        Kem Cho (“how are you?” in Gujarati) is private voice dictation. Speech is transcribed on this computer with
        OpenAI's Whisper running in whisper.cpp; audio never leaves your machine.
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
        Inspired by <a href="https://github.com/anomalyco/hex">Hex</a> (MIT). Open source at{' '}
        <a href="https://github.com/montooshah/kem-cho">github.com/montooshah/kem-cho</a>.
      </p>
    </section>
  )
}
