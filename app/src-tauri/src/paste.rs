use std::thread;
use std::time::Duration;

use anyhow::{Context, Result};
use enigo::{Direction, Enigo, Key, Keyboard};

/// Owns the clipboard for the life of the app: on X11 and Wayland the
/// copied text disappears when its owner does.
#[derive(Default)]
pub struct Paster {
    clipboard: Option<arboard::Clipboard>,
}

impl Paster {
    fn clipboard(&mut self) -> Result<&mut arboard::Clipboard> {
        if self.clipboard.is_none() {
            self.clipboard = Some(arboard::Clipboard::new().context("opening the clipboard")?);
        }
        Ok(self.clipboard.as_mut().unwrap())
    }

    pub fn copy(&mut self, text: &str) -> Result<()> {
        self.clipboard()?.set_text(text.to_string())?;
        Ok(())
    }

    /// Pastes at the cursor of the focused app via the clipboard, then puts
    /// back whatever text was there before.
    pub fn paste(&mut self, text: &str, restore: bool) -> Result<()> {
        let previous = if restore {
            self.clipboard()?.get_text().ok()
        } else {
            None
        };
        self.copy(text)?;
        thread::sleep(Duration::from_millis(60));
        send_shortcut('v')?;
        if let Some(previous) = previous {
            thread::sleep(Duration::from_millis(400));
            self.copy(&previous)?;
        }
        Ok(())
    }

    /// The text selected in the focused app, read by sending the copy
    /// shortcut; the clipboard's text or image is put back afterwards.
    pub fn selection(&mut self) -> Result<Option<String>> {
        let saved = {
            let clipboard = self.clipboard()?;
            match clipboard.get_text() {
                Ok(text) => Saved::Text(text),
                Err(_) => clipboard.get_image().map_or(Saved::Empty, Saved::Image),
            }
        };
        self.clipboard()?.clear()?;
        let selected = send_shortcut('c').and_then(|()| self.copied_text());
        let restored = match saved {
            Saved::Text(text) => self.copy(&text),
            Saved::Image(image) => Ok(self.clipboard()?.set_image(image)?),
            Saved::Empty => Ok(self.clipboard()?.clear()?),
        };
        let selected = selected?;
        restored?;
        Ok(selected)
    }

    /// Waits briefly for the focused app to answer the copy shortcut.
    fn copied_text(&mut self) -> Result<Option<String>> {
        for _ in 0..10 {
            thread::sleep(Duration::from_millis(30));
            if let Ok(text) = self.clipboard()?.get_text()
                && !text.trim().is_empty()
            {
                return Ok(Some(text));
            }
        }
        Ok(None)
    }
}

enum Saved {
    Text(String),
    Image(arboard::ImageData<'static>),
    Empty,
}

fn send_shortcut(key: char) -> Result<()> {
    let mut enigo =
        Enigo::new(&enigo::Settings::default()).context("starting keyboard simulation")?;
    let modifier = if cfg!(target_os = "macos") {
        Key::Meta
    } else {
        Key::Control
    };
    enigo.key(modifier, Direction::Press)?;
    let result = enigo.key(Key::Unicode(key), Direction::Click);
    enigo.key(modifier, Direction::Release)?;
    result?;
    Ok(())
}
