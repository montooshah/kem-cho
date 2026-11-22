import { useEffect, useState } from 'react'
import { listen } from '@tauri-apps/api/event'
import type { Status } from './api'

const BARS = 9

export default function Hud() {
  const [phase, setPhase] = useState<Status['phase']>('recording')
  const [levels, setLevels] = useState<number[]>(Array(BARS).fill(0))

  useEffect(() => {
    const offs = [
      listen<Status>('status', (e) => setPhase(e.payload.phase)),
      listen<number>('level', (e) => {
        const v = Math.min(1, Math.sqrt(e.payload) * 1.6)
        setLevels((prev) => [...prev.slice(1), v])
      }),
    ]
    return () => {
      offs.forEach((p) => p.then((off) => off()))
    }
  }, [])

  return (
    <div className="hud-pill">
      <span className={`hud-dot ${phase}`} />
      {phase === 'transcribing' ? (
        <span className="hud-label">Transcribing…</span>
      ) : (
        <div className="hud-bars">
          {levels.map((v, i) => (
            <span key={i} style={{ height: `${6 + v * 22}px` }} />
          ))}
        </div>
      )}
    </div>
  )
}
