import { invoke } from '@tauri-apps/api/core'

export type ShortcutMode = 'hold' | 'toggle'
export type Phase = 'idle' | 'recording' | 'transcribing' | 'thinking' | 'speaking'

export interface Replacement {
  from: string
  to: string
}

export interface Settings {
  model: string
  language: string
  shortcut: string
  shortcut_mode: ShortcutMode
  microphone: string | null
  paste: boolean
  restore_clipboard: boolean
  history_days: number
  remove_fillers: boolean
  voice_commands: boolean
  replacements: Replacement[]
  vocabulary: string[]
  onboarded: boolean
  agent_shortcut: string
  agent_model: string
  ollama_url: string
  agent_voice: string
  speak_replies: boolean
  agent_personality: string
  agent_commands: boolean
}

export interface AgentStatus {
  running: boolean
  version: string | null
  error: string | null
  installed: { name: string; size: number }[]
  suggested: { id: string; label: string }[]
}

export interface VoiceInfo {
  id: string
  label: string
  size_bytes: number
  installed: boolean
}

export interface Voices {
  neural: boolean
  voices: VoiceInfo[]
}

export interface PullProgress {
  model: string
  status: string
  done: number
  total: number
}

export interface TurnEvent {
  id: number
  question: string
}

export interface TokenEvent {
  id: number
  token: string
}

export interface TurnDone {
  id: number
  reply: string
  error: string | null
}

export interface Language {
  id: string
  label: string
  whisper: string
  default_model: string
  hint: string
}

export interface ModelInfo {
  id: string
  label: string
  file: string
  size_bytes: number
  sha256: string
  languages: string[]
  source: string
  no_timestamps: boolean
  description: string
  installed: boolean
}

export interface Entry {
  id: number
  text: string
  raw: string
  model: string
  audio_ms: number
  transcribe_ms: number
}

export interface Status {
  phase: Phase
  message: string | null
}

export interface Progress {
  id: string
  done: number
  total: number
}

export interface AppInfo {
  version: string
  dataDir: string | null
  os: string
}

export const api = {
  getSettings: () => invoke<Settings>('get_settings'),
  saveSettings: (settings: Settings) => invoke<Settings>('save_settings', { settings }),
  suspendShortcut: (suspended: boolean) => invoke<void>('suspend_shortcut', { suspended }),
  listLanguages: () => invoke<Language[]>('list_languages'),
  listModels: () => invoke<ModelInfo[]>('list_models'),
  downloadModel: (id: string) => invoke<void>('download_model', { id }),
  deleteModel: (id: string) => invoke<void>('delete_model', { id }),
  listMicrophones: () => invoke<string[]>('list_microphones'),
  historyList: (query?: string) => invoke<Entry[]>('history_list', { query }),
  historyDelete: (id: number) => invoke<void>('history_delete', { id }),
  historyClear: () => invoke<void>('history_clear'),
  copyText: (text: string) => invoke<void>('copy_text', { text }),
  toggleDictation: () => invoke<void>('toggle_dictation'),
  toggleAgent: () => invoke<void>('toggle_agent'),
  agentStatus: () => invoke<AgentStatus>('agent_status'),
  agentPull: (model: string) => invoke<void>('agent_pull', { model }),
  listVoices: () => invoke<Voices>('list_voices'),
  downloadVoice: (id: string) => invoke<void>('download_voice', { id }),
  previewVoice: (id: string) => invoke<void>('preview_voice', { id }),
  agentAsk: (question: string) => invoke<void>('agent_ask', { question }),
  agentStop: () => invoke<void>('agent_stop'),
  agentReset: () => invoke<void>('agent_reset'),
  getStatus: () => invoke<Status>('get_status'),
  appInfo: () => invoke<AppInfo>('app_info'),
}

/** Formats a Tauri accelerator for display, e.g. `CommandOrControl+Shift+Space`. */
export function prettyShortcut(accelerator: string, os: string): string {
  const mac = os === 'macos'
  return accelerator
    .split('+')
    .map((part) => {
      switch (part) {
        case 'CommandOrControl':
          return mac ? '⌘' : 'Ctrl'
        case 'Alt':
          return mac ? '⌥' : 'Alt'
        case 'Shift':
          return mac ? '⇧' : 'Shift'
        case 'Super':
          return mac ? '⌃' : 'Super'
        default:
          return part
      }
    })
    .join(mac ? '' : ' + ')
}

/** Builds a Tauri accelerator from a keydown, or null for a lone modifier. */
export function acceleratorFromEvent(e: KeyboardEvent): string | null {
  const code = e.code
  if (/^(Shift|Control|Alt|Meta|OS)(Left|Right)?$/.test(code)) return null
  let key: string
  if (code.startsWith('Key')) key = code.slice(3)
  else if (code.startsWith('Digit')) key = code.slice(5)
  else if (/^F\d{1,2}$/.test(code)) key = code
  else key = code
  const parts: string[] = []
  if (e.ctrlKey || e.metaKey) parts.push('CommandOrControl')
  if (e.altKey) parts.push('Alt')
  if (e.shiftKey) parts.push('Shift')
  if (parts.length === 0 && !/^F\d{1,2}$/.test(key)) return null
  parts.push(key)
  return parts.join('+')
}

export function formatBytes(n: number): string {
  return n >= 1e9 ? `${(n / 1e9).toFixed(1)} GB` : `${Math.round(n / 1e6)} MB`
}
