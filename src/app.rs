//! Launcher state and input handling.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

pub struct App {
    pub should_quit: bool,
    /// Static placeholder menu until items come from config (step 2).
    pub items: Vec<&'static str>,
    pub selected: usize,
}

impl App {
    pub fn new() -> Self {
        Self {
            should_quit: false,
            items: vec![
                "Chromium", "Files", "Games", "Terminal", "Settings", "Power",
            ],
            selected: 0,
        }
    }

    pub fn on_key(&mut self, key: KeyEvent) {
        match key.code {
            // Dev escape hatch. On the device the launcher never quits.
            KeyCode::Char('q') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.should_quit = true;
            }
            KeyCode::Up => self.selected = self.selected.saturating_sub(1),
            KeyCode::Down if self.selected + 1 < self.items.len() => self.selected += 1,
            _ => {}
        }
    }
}
