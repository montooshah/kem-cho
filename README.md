# Kem Cho

Private voice dictation and a voice assistant. Hold <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>Space</kbd>
(<kbd>⌘</kbd>+<kbd>Shift</kbd>+<kbd>Space</kbd> on a Mac), speak, and let go:
Kem Cho transcribes what you said on your computer with Whisper and pastes it
at the cursor. Hold <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>K</kbd> instead to ask
a question: a local language model answers and Kem Cho reads the reply aloud.
Audio never leaves your machine.

*Kem cho* means “how are you?” in Gujarati.

Website: [kem-cho.onrender.com](https://kem-cho.onrender.com) ·
Download: [latest release](https://github.com/montooshah/kem-cho/releases/latest)

## Features

- Hold-to-talk or tap-to-toggle global shortcut, from the tray.
- Local transcription with [whisper.cpp](https://github.com/ggerganov/whisper.cpp)
  (via `whisper-rs`); models download once from Hugging Face and are checked
  against pinned SHA-256 hashes.
- Silence is dropped before transcription, and Whisper's non-speech markers
  (`[BLANK_AUDIO]`, `(music)`) are never pasted.
- Clean-up: filler-word removal, word replacements, vocabulary hints, and the
  voice commands “new line” and “new paragraph”.
- Paste at the cursor, keeping whatever was on your clipboard.
- Searchable history, kept for 7 days by default.
- Voice agent: questions go to a model running in [Ollama](https://ollama.com)
  on your computer, and the reply is spoken sentence by sentence as it streams,
  with a [Piper](https://github.com/rhasspy/piper) neural voice (Linux and
  Windows; the system voice on macOS). Press the shortcut again to stop it.
- A browser version ([try it](https://kem-cho.onrender.com/try.html), source in
  [`site/try.html`](site/try.html)) that runs Whisper in the tab with
  transformers.js.

## Layout

| Path | What |
| --- | --- |
| `crates/kemcho-core` | Audio capture, resampling, VAD, Whisper, models, clean-up, settings, history, Ollama agent (`agent`), speech output (`tts`) |
| `crates/kemcho-cli` | `kemcho` command-line tool |
| `app/` | Tauri 2 desktop app (React front end, Rust in `app/src-tauri`) |
| `site/` | Landing page and browser demo, deployed to [Render](https://kem-cho.onrender.com) (`render.yaml`) |

## Develop

Linux needs `libwebkit2gtk-4.1-dev libayatana-appindicator3-dev librsvg2-dev
libasound2-dev libxdo-dev clang cmake`. macOS needs Xcode command-line tools;
Windows needs the MSVC build tools, LLVM and CMake.

```sh
cd app && npm install
npm run tauri dev          # desktop app
cargo test --workspace     # core tests
cargo run -p kemcho-cli -- transcribe speech.wav
cargo run -p kemcho-cli -- record --seconds 5
cargo run -p kemcho-cli -- ask "What is the capital of Gujarat?"
cargo run -p kemcho-cli -- chat --seconds 5   # speak a question, hear the answer
```

The agent needs Ollama running with a model pulled, e.g.
`ollama pull llama3.2:3b` (the app's Agent page can download it too).

Set `KEMCHO_DATA_DIR` to keep settings, history and models somewhere other
than the platform data folder.

## Release

Push a `v*` tag. The Release workflow builds macOS (Apple silicon and Intel),
Windows and Linux installers and attaches them to a draft GitHub Release.

## Roadmap

1. **Dictation** in English, Hindi, Hindi + English, Gujarati and Gujarati + English.
2. **Voice agent**: talk to a local LLM and hear the reply spoken.
3. **Dictation and agent together**.
4. Better Gujarati and code-mixed accuracy, and more languages.

## Credits

Inspired by [Hex](https://github.com/anomalyco/hex) (MIT). Speech recognition
by OpenAI Whisper and whisper.cpp; voices by Piper; language models via
Ollama. MIT licensed.
