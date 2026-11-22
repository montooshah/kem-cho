import { StrictMode } from 'react'
import { createRoot } from 'react-dom/client'
import { getCurrentWindow } from '@tauri-apps/api/window'
import './index.css'
import App from './App.tsx'
import Hud from './Hud.tsx'

const isHud = getCurrentWindow().label === 'hud'
document.documentElement.classList.toggle('hud', isHud)

createRoot(document.getElementById('root')!).render(
  <StrictMode>{isHud ? <Hud /> : <App />}</StrictMode>,
)
