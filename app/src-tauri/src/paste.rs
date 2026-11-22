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
        send_paste_keys()?;
        if let Some(previous) = previous {
            thread::sleep(Duration::from_millis(400));
            self.copy(&previous)?;
        }
        Ok(())
    }
}

fn send_paste_keys() -> Result<()> {
    let mut enigo =
        Enigo::new(&enigo::Settings::default()).context("starting keyboard simulation")?;
    let modifier = if cfg!(target_os = "macos") {
        Key::Meta
    } else {
        Key::Control
    };
    enigo.key(modifier, Direction::Press)?;
    let result = enigo.key(Key::Unicode('v'), Direction::Click);
    enigo.key(modifier, Direction::Release)?;
    result?;
    Ok(())
}
