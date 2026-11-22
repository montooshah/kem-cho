# Kem Cho

Private voice dictation. Hold <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>Space</kbd>
(<kbd>⌘</kbd>+<kbd>Shift</kbd>+<kbd>Space</kbd> on a Mac), speak, and let go:
Kem Cho transcribes what you said on your computer with Whisper and pastes it
at the cursor. Audio never leaves your machine.

*Kem cho* means “how are you?” in Gujarati.

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
- A browser version ([`site/try.html`](site/try.html)) that runs Whisper in
  the tab with transformers.js.

## Layout

| Path | What |
| --- | --- |
| `crates/kemcho-core` | Audio capture, resampling, VAD, Whisper, models, clean-up, settings, history |
| `crates/kemcho-cli` | `kemcho` command-line tool |
| `app/` | Tauri 2 desktop app (React front end, Rust in `app/src-tauri`) |
| `site/` | Landing page and browser demo, deployed to Render (`render.yaml`) |

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
```

Set `KEMCHO_DATA_DIR` to keep settings, history and models somewhere other
than the platform data folder.

## Release

Push a `v*` tag. The Release workflow builds macOS (Apple silicon and Intel),
Windows and Linux installers and attaches them to a draft GitHub Release.

## Roadmap

1. **Dictation** in English (this release).
2. **Voice agent**: talk to a local LLM and hear the reply spoken.
3. **Dictation and agent together**.
4. **Gujarati, Hindi** and more languages.

## Credits

Inspired by [Hex](https://github.com/anomalyco/hex) (MIT). Speech recognition
by OpenAI Whisper and whisper.cpp. MIT licensed.
