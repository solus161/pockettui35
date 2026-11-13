//! Launcher state and input handling.
//!
//! Buttons arrive as keys through keyd: A is Enter, B is Escape.

use std::collections::HashMap;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::config;
use crate::history::History;
use crate::items::{Action, HOME, Item, Tree};

/// Most items shown in the Recent group.
pub const RECENT_MAX: usize = 5;

/// Identifies a selectable row. An item can be on Home twice, once in Recent
/// and once in the main list, so the id alone is not enough.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RowKey {
    pub id: String,
    pub recent: bool,
}

/// One line of the menu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Row {
    Header(&'static str),
    Separator,
    Item(RowKey),
}

impl Row {
    fn key(&self) -> Option<&RowKey> {
        match self {
            Row::Item(k) => Some(k),
            _ => None,
        }
    }
}

pub struct App {
    pub should_quit: bool,
    pub tree: Tree,
    pub history: History,
    /// Id of the folder being shown.
    pub folder: String,
    /// The selected row, by identity, so it survives rows moving.
    selected: Option<RowKey>,
    /// Cursor per folder, restored when the folder is shown again.
    cursors: HashMap<String, RowKey>,
    /// Item to run. Taken by the launch code (step 4).
    pub pending: Option<String>,
}

impl App {
    pub fn new() -> Self {
        Self::with(config::load(), History::load())
    }

    pub fn with(tree: Tree, history: History) -> Self {
        let mut app = Self {
            should_quit: false,
            tree,
            history,
            folder: HOME.to_string(),
            selected: None,
            cursors: HashMap::new(),
            pending: None,
        };
        app.fix_selection();
        app
    }

    /// The rows of the current folder. Home starts with the Recent group:
    /// apps and scripts from anywhere in the tree, newest first. The main
    /// list below it never reorders, so it can be learned by position.
    pub fn rows(&self) -> Vec<Row> {
        let mut rows = Vec::new();
        if self.folder == HOME {
            let recent: Vec<&str> = self
                .history
                .recent()
                .filter(|id| self.tree.get(id).is_some_and(is_launchable))
                .take(RECENT_MAX)
                .collect();
            if !recent.is_empty() {
                rows.push(Row::Header("Recent"));
                rows.extend(recent.into_iter().map(|id| {
                    Row::Item(RowKey {
                        id: id.to_string(),
                        recent: true,
                    })
                }));
                rows.push(Row::Separator);
            }
        }
        rows.extend(self.tree.children(&self.folder).iter().map(|id| {
            Row::Item(RowKey {
                id: id.clone(),
                recent: false,
            })
        }));
        rows
    }

    /// Index of the selected row in `rows()`.
    pub fn selected_index(&self, rows: &[Row]) -> Option<usize> {
        let sel = self.selected.as_ref()?;
        rows.iter().position(|r| r.key() == Some(sel))
    }

    pub fn selected_item(&self) -> Option<&Item> {
        self.tree.get(&self.selected.as_ref()?.id)
    }

    pub fn on_key(&mut self, key: KeyEvent) {
        match key.code {
            // Dev escape hatch. On the device the launcher never quits.
            KeyCode::Char('q') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.should_quit = true;
            }
            KeyCode::Up => self.move_by(-1),
            KeyCode::Down => self.move_by(1),
            KeyCode::Home => self.move_by(isize::MIN),
            KeyCode::End => self.move_by(isize::MAX),
            KeyCode::Enter => self.activate(),
            KeyCode::Esc => self.back(),
            _ => {}
        }
    }

    /// Moves the cursor by `delta` selectable rows, stopping at the ends.
    fn move_by(&mut self, delta: isize) {
        let rows = self.rows();
        let keys: Vec<&RowKey> = rows.iter().filter_map(Row::key).collect();
        if keys.is_empty() {
            return;
        }
        let cur = self
            .selected
            .as_ref()
            .and_then(|s| keys.iter().position(|k| *k == s))
            .unwrap_or(0);
        let last = keys.len() - 1;
        let next = cur.saturating_add_signed(delta).min(last);
        self.selected = Some(keys[next].clone());
    }

    /// Button A.
    fn activate(&mut self) {
        let Some(item) = self.selected_item() else {
            return;
        };
        let id = item.id.clone();
        match &item.action {
            Action::Folder => self.enter(&id),
            // Adjusted with Left/Right, not A (step 7).
            Action::Value(_) | Action::Choice(_) => {}
            Action::App { .. } | Action::Script { .. } => {
                self.history.record(&id);
                self.pending = Some(id);
            }
            Action::Builtin { .. } => self.pending = Some(id),
        }
    }

    /// Button B. Goes up one folder; at Home it does nothing yet (it will
    /// clear the search, step 5).
    fn back(&mut self) {
        if self.folder == HOME {
            return;
        }
        let from = self.folder.clone();
        let parent = self.tree.parent(&from).unwrap_or(HOME).to_string();
        self.show(&parent);
        // The folder you came from is selected if this folder has no
        // remembered cursor yet, e.g. it was reached some other way.
        if !self.cursors.contains_key(&parent) {
            self.selected = Some(RowKey {
                id: from,
                recent: false,
            });
        }
        self.fix_selection();
    }

    fn enter(&mut self, folder: &str) {
        self.show(folder);
    }

    /// Switches folder, saving the cursor of the old one and restoring the
    /// cursor of the new one.
    fn show(&mut self, folder: &str) {
        if let Some(sel) = self.selected.take() {
            self.cursors.insert(self.folder.clone(), sel);
        }
        self.folder = folder.to_string();
        self.selected = self.cursors.get(folder).cloned();
        self.fix_selection();
    }

    /// Makes sure the selection points at an existing row, falling back to
    /// the first one. Call after anything that changes the rows.
    pub fn fix_selection(&mut self) {
        let rows = self.rows();
        if self.selected_index(&rows).is_none() {
            self.selected = rows.iter().find_map(Row::key).cloned();
        }
    }
}

/// Items that count as "used" and can appear in Recent.
fn is_launchable(item: &Item) -> bool {
    matches!(item.action, Action::App { .. } | Action::Script { .. })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Layer, SHIPPED, merge_layers};
    use crossterm::event::KeyEvent;

    fn app() -> App {
        let (merged, warnings) = merge_layers(&[Layer {
            name: "shipped".into(),
            text: SHIPPED.into(),
        }]);
        App::with(Tree::build(merged, warnings), History::in_memory())
    }

    fn press(app: &mut App, code: KeyCode) {
        app.on_key(KeyEvent::from(code));
    }

    fn sel(app: &App) -> String {
        app.selected_item().unwrap().id.clone()
    }

    /// Moves the cursor down to the main-list row with this id.
    fn go_to(app: &mut App, id: &str) {
        press(app, KeyCode::Home);
        for _ in 0..50 {
            if app
                .selected
                .as_ref()
                .is_some_and(|k| k.id == id && !k.recent)
            {
                return;
            }
            press(app, KeyCode::Down);
        }
        panic!("{id} not found");
    }

    #[test]
    fn starts_on_first_item() {
        assert_eq!(sel(&app()), "browser");
    }

    #[test]
    fn up_down_stop_at_the_ends() {
        let mut a = app();
        press(&mut a, KeyCode::Up);
        assert_eq!(sel(&a), "browser");
        press(&mut a, KeyCode::End);
        assert_eq!(sel(&a), "terminal");
        press(&mut a, KeyCode::Down);
        assert_eq!(sel(&a), "terminal");
    }

    #[test]
    fn enter_folder_and_back_restores_cursor() {
        let mut a = app();
        go_to(&mut a, "settings");
        press(&mut a, KeyCode::Enter);
        assert_eq!(a.folder, "settings");
        assert_eq!(sel(&a), "about");

        press(&mut a, KeyCode::Down);
        press(&mut a, KeyCode::Down);
        let in_settings = sel(&a);

        press(&mut a, KeyCode::Esc);
        assert_eq!(a.folder, HOME);
        assert_eq!(sel(&a), "settings");

        press(&mut a, KeyCode::Enter);
        assert_eq!(sel(&a), in_settings);
    }

    #[test]
    fn back_at_home_does_nothing() {
        let mut a = app();
        press(&mut a, KeyCode::Down);
        press(&mut a, KeyCode::Esc);
        assert_eq!(a.folder, HOME);
        assert_eq!(sel(&a), "files");
    }

    #[test]
    fn launching_fills_recent_and_main_list_stays_put() {
        let mut a = app();
        let main_before: Vec<Row> = a.rows();

        go_to(&mut a, "settings");
        press(&mut a, KeyCode::Enter);
        go_to(&mut a, "wifi");
        press(&mut a, KeyCode::Enter);
        assert_eq!(a.pending.as_deref(), Some("wifi"));
        press(&mut a, KeyCode::Esc);

        go_to(&mut a, "files");
        press(&mut a, KeyCode::Enter);

        let rows = a.rows();
        let recent: Vec<&str> = rows
            .iter()
            .filter_map(Row::key)
            .filter(|k| k.recent)
            .map(|k| k.id.as_str())
            .collect();
        assert_eq!(recent, ["files", "wifi"]);
        assert_eq!(rows[0], Row::Header("Recent"));
        assert_eq!(rows[3], Row::Separator);
        // The main list is unchanged, after the Recent group.
        assert_eq!(rows[4..], main_before[..]);
        // The cursor stayed on Files in the main list.
        assert_eq!(
            a.selected,
            Some(RowKey {
                id: "files".into(),
                recent: false
            })
        );
    }

    #[test]
    fn folders_and_values_never_enter_recent() {
        let mut a = app();
        go_to(&mut a, "settings");
        press(&mut a, KeyCode::Enter);
        go_to(&mut a, "volume");
        press(&mut a, KeyCode::Enter);
        assert!(a.pending.is_none());
        press(&mut a, KeyCode::Esc);
        assert!(!a.rows().contains(&Row::Header("Recent")));
    }

    #[test]
    fn recent_is_capped() {
        let mut a = app();
        for id in [
            "browser",
            "files",
            "terminal",
            "games",
            "wifi",
            "bluetooth",
            "raspi_config",
        ] {
            a.history.record(id);
        }
        let n = a
            .rows()
            .iter()
            .filter_map(Row::key)
            .filter(|k| k.recent)
            .count();
        assert_eq!(n, RECENT_MAX);
    }
}
