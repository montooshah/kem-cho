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
    /// shortcut; the clipboard is put back afterwards.
    pub fn selection(&mut self) -> Result<Option<String>> {
        let previous = self.clipboard()?.get_text().ok();
        self.clipboard()?.clear()?;
        send_shortcut('c')?;
        let mut selected = None;
        for _ in 0..10 {
            thread::sleep(Duration::from_millis(30));
            if let Ok(text) = self.clipboard()?.get_text()
                && !text.trim().is_empty()
            {
                selected = Some(text);
                break;
            }
        }
        match previous {
            Some(previous) => self.copy(&previous)?,
            None => self.clipboard()?.clear()?,
        }
        Ok(selected)
    }
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
