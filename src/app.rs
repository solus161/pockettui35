//! Launcher state and input handling.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::config;
use crate::items::{HOME, Tree};

pub struct App {
    pub should_quit: bool,
    pub tree: Tree,
    /// Id of the folder being shown.
    pub folder: String,
    pub selected: usize,
}

impl App {
    pub fn new() -> Self {
        Self::with_tree(config::load())
    }

    pub fn with_tree(tree: Tree) -> Self {
        Self {
            should_quit: false,
            tree,
            folder: HOME.to_string(),
            selected: 0,
        }
    }

    pub fn on_key(&mut self, key: KeyEvent) {
        let len = self.tree.children(&self.folder).len();
        match key.code {
            // Dev escape hatch. On the device the launcher never quits.
            KeyCode::Char('q') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.should_quit = true;
            }
            KeyCode::Up => self.selected = self.selected.saturating_sub(1),
            KeyCode::Down if self.selected + 1 < len => self.selected += 1,
            _ => {}
        }
    }
}
