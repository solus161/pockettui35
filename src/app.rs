//! Launcher state and input handling.
//!
//! Buttons arrive as keys through keyd: A is Enter, B is Escape.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::config;
use crate::cpufreq::Profile;
use crate::history::History;
use crate::items::{Action, Builtin, HOME, Item, Tree};
use crate::script::ScriptRun;
use crate::search::{Hit, Search};
use crate::system::{PowerAction, System, Volume};

/// How long "press A again" waits for the second press.
pub const CONFIRM_WINDOW: Duration = Duration::from_secs(3);

/// Volume changes in steps of this many percent.
const VOLUME_STEP: u8 = 5;
/// Brightness changes in steps of this many percent.
const BRIGHTNESS_STEP: u8 = 10;
/// The firmware's floor; asking for less gets this.
const BRIGHTNESS_MIN: u8 = 5;

/// The About screen: a script with no item behind it.
const ABOUT_EXEC: &str = concat!(
    "echo 'pockettui35 ",
    env!("CARGO_PKG_VERSION"),
    "'; echo; ",
    "tr -d '\\0' < /proc/device-tree/model 2>/dev/null && echo; ",
    "uname -sr; ",
    "echo \"host    $(hostname)\"; ",
    "echo \"address $(hostname -I 2>/dev/null)\"; ",
    "echo \"up      $(uptime -p 2>/dev/null)\""
);

/// Settings and status read from the system. Refreshed after the first
/// frame, on return from an app, on entering a folder, and (network and
/// profile) each minute.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Values {
    pub volume: Option<Volume>,
    pub brightness: Option<u8>,
    pub profile: Option<Profile>,
    pub net: Option<String>,
}

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
    /// Item to run, taken by the main loop.
    pub pending: Option<String>,
    /// One-shot note for the hint bar, e.g. why a launch failed. Cleared by
    /// the next key.
    pub message: Option<String>,
    /// The search text. Not empty means the menu shows search results.
    pub query: String,
    /// Results for `query`, best first.
    pub hits: Vec<Hit>,
    search: Search,
    /// Selection when the search started, restored when it ends.
    before_search: Option<RowKey>,
    /// The script output screen, shown over the menu while `Some`.
    pub script: Option<ScriptRun>,
    system: Box<dyn System>,
    pub values: Values,
    /// An item waiting for its second A, and when the first came.
    pub confirm: Option<(String, Instant)>,
    /// The screen is off; the next key only turns it back on.
    pub blanked: bool,
}

impl App {
    pub fn new(system: Box<dyn System>) -> Self {
        Self::with(config::load(), History::load(), system)
    }

    pub fn with(tree: Tree, history: History, system: Box<dyn System>) -> Self {
        let mut app = Self {
            should_quit: false,
            tree,
            history,
            folder: HOME.to_string(),
            selected: None,
            cursors: HashMap::new(),
            pending: None,
            message: None,
            query: String::new(),
            hits: Vec::new(),
            search: Search::new(),
            before_search: None,
            script: None,
            system,
            values: Values::default(),
            confirm: None,
            blanked: false,
        };
        app.fix_selection();
        app
    }

    /// Re-reads the config, keeping the folder and cursor where they still
    /// exist. Called each time the launcher gets the screen back.
    pub fn reload(&mut self) {
        self.set_tree(config::load());
        // A keyboard replugged or reflashed meanwhile is worth asking again.
        self.system.rescan();
        self.refresh_values();
    }

    /// Reads every setting and the status bar from the system.
    pub fn refresh_values(&mut self) {
        self.values = Values {
            volume: self.system.volume(),
            brightness: self.system.brightness(),
            profile: self.system.power_profile(),
            net: self.system.network(),
        };
    }

    /// The minute tick: the status bar's network and profile. Volume and
    /// brightness are left alone; asking the keyboard every minute is not
    /// free.
    pub fn on_minute(&mut self) {
        self.values.net = self.system.network();
        self.values.profile = self.system.power_profile();
    }

    /// When the main loop must wake even without input: the end of a
    /// confirmation window.
    pub fn deadline(&self) -> Option<Instant> {
        self.confirm.as_ref().map(|(_, at)| *at + CONFIRM_WINDOW)
    }

    /// Expires a confirmation whose window has passed. Returns true if the
    /// screen changed.
    pub fn tick(&mut self, now: Instant) -> bool {
        if self.deadline().is_some_and(|d| now >= d) {
            self.confirm = None;
            self.message = None;
            return true;
        }
        false
    }

    /// Right-hand text for a `value` or `choice` row.
    pub fn value_text(&self, item: &Item) -> Option<String> {
        let bar = |p: u8| {
            let filled = usize::from(p.div_ceil(10));
            format!("[{}{}]", "━".repeat(filled), "─".repeat(10 - filled))
        };
        match item.action {
            Action::Value(Builtin::Volume) => Some(match self.values.volume {
                Some(Volume {
                    percent,
                    muted: true,
                }) => format!("{} {percent:>3}% muted", bar(percent)),
                Some(v) => format!("{} {:>3}%", bar(v.percent), v.percent),
                None => "n/a".into(),
            }),
            Action::Value(Builtin::Brightness) => Some(match self.values.brightness {
                Some(b) => format!("{} {b:>3}%", bar(b)),
                None => "n/a".into(),
            }),
            Action::Choice(Builtin::PowerProfile) => Some(match self.values.profile {
                Some(p) => format!("< {} >", p.label()),
                None => "< custom >".into(),
            }),
            _ => None,
        }
    }

    fn set_tree(&mut self, tree: Tree) {
        self.tree = tree;
        if self.folder != HOME && self.tree.get(&self.folder).is_none_or(|i| !i.is_folder()) {
            self.folder = HOME.to_string();
            self.selected = self.cursors.get(HOME).cloned();
        }
        self.cursors.retain(|_, k| self.tree.get(&k.id).is_some());
        if self.searching() {
            self.refresh_hits();
        }
        self.fix_selection();
    }

    pub fn searching(&self) -> bool {
        !self.query.is_empty()
    }

    /// The rows on screen. While searching, the hits. Otherwise the current
    /// folder; Home starts with the Recent group: apps and scripts from
    /// anywhere in the tree, newest first. The main list below it never
    /// reorders, so it can be learned by position.
    pub fn rows(&self) -> Vec<Row> {
        if self.searching() {
            if self.hits.is_empty() {
                return vec![Row::Header("No matches")];
            }
            return self
                .hits
                .iter()
                .map(|h| {
                    Row::Item(RowKey {
                        id: h.id.clone(),
                        recent: false,
                    })
                })
                .collect();
        }
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
        // The key that wakes the screen does nothing else.
        if self.blanked {
            self.blanked = false;
            self.system.blank(false);
            return;
        }
        self.message = None;
        // Any key but a second A cancels a pending confirmation.
        if key.code != KeyCode::Enter {
            self.confirm = None;
        }
        // Quits. On the device, pockettui35-session then drops to a shell
        // for maintenance; leaving it brings the launcher back.
        if key.code == KeyCode::Char('q') && key.modifiers.contains(KeyModifiers::CONTROL) {
            self.should_quit = true;
            return;
        }
        if self.script.is_some() {
            self.script_key(key);
            return;
        }
        match key.code {
            KeyCode::Up => self.move_by(-1),
            KeyCode::Down => self.move_by(1),
            KeyCode::Left => self.adjust(-1),
            KeyCode::Right => self.adjust(1),
            KeyCode::Home => self.move_by(isize::MIN),
            KeyCode::End => self.move_by(isize::MAX),
            KeyCode::Enter => self.activate(),
            KeyCode::Esc if self.searching() => self.end_search(),
            KeyCode::Esc => self.back(),
            KeyCode::Backspace if self.searching() => {
                self.query.pop();
                if self.query.is_empty() {
                    self.end_search();
                } else {
                    self.refresh_hits();
                }
            }
            // Any printable key types into the search. No focus step.
            KeyCode::Char(c)
                if !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                self.type_char(c);
            }
            _ => {}
        }
    }

    /// Keys on the script output screen. Up/Down and PageUp/PageDown
    /// scroll, Home/End jump. A runs it again once it has ended. B stops a
    /// running script (TERM, then KILL on a second press), or closes the
    /// screen.
    fn script_key(&mut self, key: KeyEvent) {
        let Some(run) = &mut self.script else { return };
        let page = usize::from(run.view_height.get().max(2) - 1);
        let max = run
            .output
            .len()
            .saturating_sub(usize::from(run.view_height.get()));
        let scroll = |run: &mut ScriptRun, up: isize| {
            run.from_bottom = run.from_bottom.saturating_add_signed(up).min(max);
        };
        match key.code {
            KeyCode::Up => scroll(run, 1),
            KeyCode::Down => scroll(run, -1),
            KeyCode::PageUp => scroll(run, page.cast_signed()),
            KeyCode::PageDown => scroll(run, -page.cast_signed()),
            KeyCode::Home => run.from_bottom = max,
            KeyCode::End => run.from_bottom = 0,
            KeyCode::Enter if !run.running() => {
                self.history.record(&run.id);
                run.restart();
            }
            KeyCode::Esc if run.running() => run.stop(),
            KeyCode::Esc => self.script = None,
            _ => {}
        }
    }

    /// Start held while the launcher has the screen: stops a running
    /// script (TERM, then KILL on a second hold), as B does. Returns true if
    /// the screen changed.
    pub fn close_request(&mut self) -> bool {
        match &mut self.script {
            Some(run) if run.running() => {
                run.stop();
                true
            }
            _ => false,
        }
    }

    /// Takes in new script output. Returns true if the screen changed.
    pub fn pump_script(&mut self) -> bool {
        self.script.as_mut().is_some_and(ScriptRun::pump)
    }

    pub fn script_running(&self) -> bool {
        self.script.as_ref().is_some_and(ScriptRun::running)
    }

    fn type_char(&mut self, c: char) {
        if !self.searching() {
            // A leading space would match everything.
            if c.is_whitespace() {
                return;
            }
            self.before_search = self.selected.clone();
        }
        self.query.push(c);
        self.refresh_hits();
    }

    /// Re-runs the search and selects the best hit.
    fn refresh_hits(&mut self) {
        self.hits = self.search.run(&self.tree, &self.history, &self.query);
        self.selected = None;
        self.fix_selection();
    }

    /// Clears the search and returns to the folder and row it started from.
    fn end_search(&mut self) {
        self.query.clear();
        self.hits.clear();
        self.selected = self.before_search.take();
        self.fix_selection();
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
        let action = item.action.clone();
        // Values and choices adjust in place, so they keep the search.
        // Anything else ends it: a folder opens, an app runs.
        if !matches!(action, Action::Value(_) | Action::Choice(_)) && self.searching() {
            self.end_search();
            // B from a folder found by search lands on that folder.
            if action == Action::Folder {
                let parent = self.tree.parent(&id).unwrap_or(HOME).to_string();
                let key = RowKey {
                    id: id.clone(),
                    recent: false,
                };
                if parent == self.folder {
                    self.selected = Some(key);
                } else {
                    self.cursors.insert(parent, key);
                }
            }
        }
        match action {
            Action::Folder => self.enter(&id),
            // Adjusted with Left/Right, not A (step 7).
            Action::Value(_) | Action::Choice(_) => {}
            Action::App { .. } => {
                self.history.record(&id);
                self.pending = Some(id);
            }
            Action::Script { exec } => {
                self.history.record(&id);
                let label = self.tree.get(&id).map_or("", |i| i.label.as_str());
                self.script = Some(ScriptRun::start(&id, label, &exec));
            }
            Action::Builtin { builtin, confirm } => self.run_builtin(&id, builtin, confirm),
        }
    }

    fn run_builtin(&mut self, id: &str, builtin: Builtin, confirm: bool) {
        if confirm {
            let confirmed = self
                .confirm
                .take()
                .is_some_and(|(c, at)| c == id && at.elapsed() < CONFIRM_WINDOW);
            if !confirmed {
                let label = self.tree.get(id).map_or(id, |i| i.label.as_str());
                self.message = Some(format!("Press A again to {}", label.to_lowercase()));
                self.confirm = Some((id.to_string(), Instant::now()));
                return;
            }
        }
        match builtin {
            Builtin::Reboot | Builtin::Shutdown => {
                let (action, doing) = if builtin == Builtin::Reboot {
                    (PowerAction::Reboot, "Rebooting")
                } else {
                    (PowerAction::Shutdown, "Shutting down")
                };
                self.message = Some(match self.system.power(action) {
                    Ok(()) => format!("{doing}..."),
                    Err(e) => format!("{doing} failed: {e}"),
                });
            }
            Builtin::ScreenOff => {
                self.system.blank(true);
                self.blanked = true;
            }
            Builtin::About => self.script = Some(ScriptRun::start("about", "About", ABOUT_EXEC)),
            // Values and choices are adjusted, not run.
            Builtin::Volume | Builtin::Brightness | Builtin::PowerProfile => {}
        }
    }

    /// Left (`-1`) or Right (`1`) on the selected row: changes a `value` or
    /// `choice` in place.
    fn adjust(&mut self, dir: i8) {
        let Some(item) = self.selected_item() else {
            return;
        };
        let label = item.label.clone();
        match item.action {
            Action::Value(Builtin::Volume) => {
                let Some(cur) = self.values.volume else {
                    self.message = Some(format!("{label}: not available"));
                    return;
                };
                let next = step(cur.percent, dir, VOLUME_STEP, 0);
                self.values.volume = self.system.set_volume(next);
                if self.values.volume.is_none() {
                    self.message = Some(format!("{label}: could not set"));
                }
            }
            Action::Value(Builtin::Brightness) => {
                let Some(cur) = self.values.brightness else {
                    self.message = Some(format!("{label}: keyboard firmware not answering"));
                    return;
                };
                let next = step(cur, dir, BRIGHTNESS_STEP, BRIGHTNESS_MIN);
                self.values.brightness = self.system.set_brightness(next);
                if self.values.brightness.is_none() {
                    self.message = Some(format!("{label}: could not set"));
                }
            }
            Action::Choice(Builtin::PowerProfile) => {
                // From "custom", either way lands on balanced first.
                let next = self
                    .values
                    .profile
                    .map_or(Profile::Balanced, |p| p.step(dir));
                if self.values.profile == Some(next) {
                    return;
                }
                if let Err(e) = self.system.set_power_profile(next) {
                    self.message = Some(format!("{label}: {e}"));
                }
                self.values.profile = self.system.power_profile();
            }
            _ => {}
        }
    }

    /// Button B outside a search. Goes up one folder; at Home it does
    /// nothing.
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
        // Settings may have changed behind our back (Fn keys, other tools).
        self.refresh_values();
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

/// One step up or down from `cur`, snapped to multiples of `size` (42 goes
/// to 45 or 40 in steps of 5), kept in `min..=100`.
fn step(cur: u8, dir: i8, size: u8, min: u8) -> u8 {
    let next = if dir > 0 {
        (cur / size + 1) * size
    } else {
        cur.div_ceil(size).saturating_sub(1) * size
    };
    next.clamp(min, 100)
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
        App::with(
            Tree::build(merged, warnings),
            History::in_memory(),
            Box::new(crate::system::fake::Fake::working()),
        )
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

    #[test]
    fn reload_keeps_position_or_falls_back_home() {
        let mut a = app();
        go_to(&mut a, "settings");
        press(&mut a, KeyCode::Enter);
        go_to(&mut a, "wifi");

        // Same config: still on Wi-Fi in Settings.
        let same = a.tree.clone();
        a.set_tree(same);
        assert_eq!(a.folder, "settings");
        assert_eq!(sel(&a), "wifi");

        // Settings hidden by the user: back to Home, on its first row.
        let (merged, warnings) = merge_layers(&[
            Layer {
                name: "shipped".into(),
                text: SHIPPED.into(),
            },
            Layer {
                name: "user".into(),
                text: "[item.settings]\nhidden = true\n".into(),
            },
        ]);
        a.set_tree(Tree::build(merged, warnings));
        assert_eq!(a.folder, HOME);
        assert_eq!(sel(&a), "browser");
    }

    fn type_str(app: &mut App, text: &str) {
        for c in text.chars() {
            press(app, KeyCode::Char(c));
        }
    }

    #[test]
    fn typing_searches_the_whole_tree() {
        let mut a = app();
        type_str(&mut a, "reb");
        assert!(a.searching());
        assert_eq!(a.folder, HOME);
        assert_eq!(sel(&a), "reboot");
        assert_eq!(a.rows().len(), a.hits.len());
    }

    #[test]
    fn b_ends_search_where_it_started() {
        let mut a = app();
        go_to(&mut a, "settings");
        press(&mut a, KeyCode::Enter);
        go_to(&mut a, "volume");

        type_str(&mut a, "chr");
        assert_eq!(sel(&a), "browser");
        press(&mut a, KeyCode::Esc);
        assert!(!a.searching());
        assert_eq!(a.folder, "settings");
        assert_eq!(sel(&a), "volume");

        // A second B is a normal back.
        press(&mut a, KeyCode::Esc);
        assert_eq!(a.folder, HOME);
    }

    #[test]
    fn backspace_to_empty_ends_search() {
        let mut a = app();
        press(&mut a, KeyCode::Down);
        type_str(&mut a, "wi");
        press(&mut a, KeyCode::Backspace);
        assert_eq!(a.query, "w");
        assert!(a.searching());
        press(&mut a, KeyCode::Backspace);
        assert!(!a.searching());
        assert_eq!(sel(&a), "files");
    }

    #[test]
    fn a_on_a_folder_hit_opens_it() {
        let mut a = app();
        type_str(&mut a, "power");
        press(&mut a, KeyCode::Enter);
        assert!(!a.searching());
        assert_eq!(a.folder, "power");
        // B from there goes to Home, where the search started.
        press(&mut a, KeyCode::Esc);
        assert_eq!(a.folder, HOME);
        assert_eq!(sel(&a), "power");
    }

    #[test]
    fn a_on_an_app_hit_launches_and_clears() {
        let mut a = app();
        type_str(&mut a, "wifi");
        press(&mut a, KeyCode::Enter);
        assert_eq!(a.pending.as_deref(), Some("wifi"));
        assert!(!a.searching());
        assert_eq!(a.folder, HOME);
    }

    #[test]
    fn a_on_a_value_hit_keeps_the_search() {
        let mut a = app();
        type_str(&mut a, "volume");
        press(&mut a, KeyCode::Enter);
        assert!(a.searching());
        assert!(a.pending.is_none());
    }

    #[test]
    fn no_matches_has_no_selection() {
        let mut a = app();
        type_str(&mut a, "zzzz");
        assert_eq!(a.rows(), [Row::Header("No matches")]);
        assert!(a.selected_item().is_none());
        press(&mut a, KeyCode::Enter);
        assert!(a.pending.is_none());
    }

    #[test]
    fn leading_space_and_ctrl_keys_are_not_typed() {
        let mut a = app();
        press(&mut a, KeyCode::Char(' '));
        assert!(!a.searching());
        a.on_key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::CONTROL));
        assert!(!a.searching());
        // Spaces inside a query are kept: they separate words.
        type_str(&mut a, "sh d");
        assert_eq!(a.query, "sh d");
        assert_eq!(sel(&a), "shutdown");
    }

    fn app_with_script(exec: &str) -> App {
        let user = format!(
            "[item.job]\nlabel = \"Job\"\ntype = \"script\"\nexec = {exec:?}\npin = true\n"
        );
        let (merged, warnings) = merge_layers(&[
            Layer {
                name: "shipped".into(),
                text: SHIPPED.into(),
            },
            Layer {
                name: "user".into(),
                text: user,
            },
        ]);
        let tree = Tree::build(merged, warnings);
        assert!(tree.warnings.is_empty(), "{:?}", tree.warnings);
        App::with(
            tree,
            History::in_memory(),
            Box::new(crate::system::fake::Fake::working()),
        )
    }

    fn wait_script(a: &mut App) {
        let start = std::time::Instant::now();
        while a.script_running() {
            a.pump_script();
            assert!(start.elapsed().as_secs() < 5, "script hung");
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
        a.pump_script();
    }

    #[test]
    fn script_opens_its_screen_and_b_closes_it() {
        let mut a = app_with_script("echo hi");
        assert_eq!(sel(&a), "job");
        press(&mut a, KeyCode::Enter);
        assert!(a.script.is_some());
        assert!(a.pending.is_none(), "scripts do not give up the screen");
        wait_script(&mut a);
        assert_eq!(a.script.as_ref().unwrap().output.lines(), ["hi"]);

        // Typing does not leak into search behind the screen.
        press(&mut a, KeyCode::Char('x'));
        assert!(!a.searching());

        press(&mut a, KeyCode::Esc);
        assert!(a.script.is_none());
        assert_eq!(sel(&a), "job");
        // Recorded as used.
        assert!(a.rows().contains(&Row::Header("Recent")));
    }

    #[test]
    fn b_stops_a_running_script_before_closing() {
        let mut a = app_with_script("sleep 30");
        press(&mut a, KeyCode::Enter);
        press(&mut a, KeyCode::Esc);
        assert!(a.script.is_some(), "first B stops, does not close");
        wait_script(&mut a);
        assert_eq!(a.script.as_ref().unwrap().summary(), "stopped by signal 15");
        press(&mut a, KeyCode::Esc);
        assert!(a.script.is_none());
    }

    #[test]
    fn a_runs_again_only_after_the_end() {
        let mut a = app_with_script("echo run; sleep 0.3");
        press(&mut a, KeyCode::Enter);
        std::thread::sleep(std::time::Duration::from_millis(100));
        a.pump_script();
        // A while running does nothing: the output is kept.
        press(&mut a, KeyCode::Enter);
        assert_eq!(a.script.as_ref().unwrap().output.lines(), ["run"]);
        wait_script(&mut a);

        // A after the end clears the screen and runs it again.
        press(&mut a, KeyCode::Enter);
        assert!(a.script_running());
        assert_eq!(a.script.as_ref().unwrap().output.len(), 0);
        wait_script(&mut a);
        assert_eq!(a.script.as_ref().unwrap().output.lines(), ["run"]);
    }

    #[test]
    fn scrolling_is_clamped() {
        let mut a = app_with_script("seq 1 100");
        press(&mut a, KeyCode::Enter);
        wait_script(&mut a);
        a.script.as_ref().unwrap().view_height.set(20);
        press(&mut a, KeyCode::Home);
        assert_eq!(a.script.as_ref().unwrap().from_bottom, 80);
        press(&mut a, KeyCode::Up);
        assert_eq!(a.script.as_ref().unwrap().from_bottom, 80);
        press(&mut a, KeyCode::PageDown);
        assert_eq!(a.script.as_ref().unwrap().from_bottom, 61);
        press(&mut a, KeyCode::End);
        assert_eq!(a.script.as_ref().unwrap().from_bottom, 0);
        press(&mut a, KeyCode::Down);
        assert_eq!(a.script.as_ref().unwrap().from_bottom, 0);
    }

    use crate::system::fake::Fake;

    /// The shipped app on a fake system, values loaded, plus a handle on
    /// the fake's state.
    fn app_fake() -> (App, Fake) {
        let fake = Fake::working();
        let (merged, warnings) = merge_layers(&[Layer {
            name: "shipped".into(),
            text: SHIPPED.into(),
        }]);
        let mut a = App::with(
            Tree::build(merged, warnings),
            History::in_memory(),
            Box::new(fake.clone()),
        );
        a.refresh_values();
        (a, fake)
    }

    fn open_settings_at(a: &mut App, id: &str) {
        go_to(a, "settings");
        press(a, KeyCode::Enter);
        go_to(a, id);
    }

    #[test]
    fn steps_snap_and_clamp() {
        assert_eq!(step(42, 1, 5, 0), 45);
        assert_eq!(step(42, -1, 5, 0), 40);
        assert_eq!(step(40, -1, 5, 0), 35);
        assert_eq!(step(100, 1, 5, 0), 100);
        assert_eq!(step(0, -1, 5, 0), 0);
        assert_eq!(step(10, -1, 10, 5), 5);
        assert_eq!(step(5, 1, 10, 5), 10);
    }

    #[test]
    fn left_right_adjust_volume() {
        let (mut a, fake) = app_fake();
        open_settings_at(&mut a, "volume");
        press(&mut a, KeyCode::Right);
        press(&mut a, KeyCode::Right);
        press(&mut a, KeyCode::Left);
        assert_eq!(a.values.volume.unwrap().percent, 45);
        assert_eq!(
            fake.0.borrow().calls,
            ["volume 45", "volume 50", "volume 45"]
        );
        let item = a.selected_item().unwrap().clone();
        assert_eq!(a.value_text(&item).unwrap(), "[━━━━━─────]  45%");
    }

    #[test]
    fn brightness_respects_the_floor_and_silence() {
        let (mut a, fake) = app_fake();
        open_settings_at(&mut a, "brightness");
        for _ in 0..12 {
            press(&mut a, KeyCode::Left);
        }
        assert_eq!(a.values.brightness, Some(5));

        // Stock firmware: no change attempted, a message instead.
        fake.0.borrow_mut().brightness = None;
        a.refresh_values();
        fake.0.borrow_mut().calls.clear();
        press(&mut a, KeyCode::Right);
        assert!(fake.0.borrow().calls.is_empty());
        assert!(a.message.as_deref().unwrap().contains("not answering"));
        let item = a.selected_item().unwrap().clone();
        assert_eq!(a.value_text(&item).unwrap(), "n/a");
    }

    #[test]
    fn power_profile_steps_and_reports_errors() {
        let (mut a, fake) = app_fake();
        open_settings_at(&mut a, "power_profile");
        press(&mut a, KeyCode::Right);
        assert_eq!(a.values.profile, Some(Profile::Performance));
        // At the end: nothing to do.
        press(&mut a, KeyCode::Right);
        assert_eq!(fake.0.borrow().calls, ["profile performance"]);

        fake.0.borrow_mut().profile_error = Some("needs the sudoers rule".into());
        press(&mut a, KeyCode::Left);
        assert_eq!(a.values.profile, Some(Profile::Performance));
        assert_eq!(
            a.message.as_deref(),
            Some("Power profile: needs the sudoers rule")
        );
    }

    #[test]
    fn left_right_do_nothing_on_other_items() {
        let (mut a, fake) = app_fake();
        press(&mut a, KeyCode::Right);
        press(&mut a, KeyCode::Left);
        assert!(fake.0.borrow().calls.is_empty());
        assert!(a.pending.is_none());
    }

    #[test]
    fn reboot_needs_a_second_a() {
        let (mut a, fake) = app_fake();
        go_to(&mut a, "power");
        press(&mut a, KeyCode::Enter);
        go_to(&mut a, "reboot");
        press(&mut a, KeyCode::Enter);
        assert_eq!(a.message.as_deref(), Some("Press A again to reboot"));
        assert!(fake.0.borrow().calls.is_empty());
        press(&mut a, KeyCode::Enter);
        assert_eq!(fake.0.borrow().calls, ["power Reboot"]);
        assert_eq!(a.message.as_deref(), Some("Rebooting..."));
    }

    #[test]
    fn another_key_or_time_cancels_the_confirmation() {
        let (mut a, fake) = app_fake();
        go_to(&mut a, "power");
        press(&mut a, KeyCode::Enter);
        go_to(&mut a, "shutdown");

        press(&mut a, KeyCode::Enter);
        press(&mut a, KeyCode::Up);
        press(&mut a, KeyCode::Down);
        press(&mut a, KeyCode::Enter);
        assert!(fake.0.borrow().calls.is_empty(), "moving away cancelled it");

        // The window runs out.
        let first = a.confirm.as_ref().unwrap().1;
        assert!(!a.tick(first));
        assert!(a.tick(first + CONFIRM_WINDOW));
        assert!(a.confirm.is_none() && a.message.is_none());
        press(&mut a, KeyCode::Enter);
        assert!(fake.0.borrow().calls.is_empty(), "a fresh first press");
    }

    #[test]
    fn screen_off_and_the_waking_key_is_swallowed() {
        let (mut a, fake) = app_fake();
        go_to(&mut a, "power");
        press(&mut a, KeyCode::Enter);
        go_to(&mut a, "screen_off");
        press(&mut a, KeyCode::Enter);
        assert!(a.blanked && fake.0.borrow().blanked);

        press(&mut a, KeyCode::Esc);
        assert!(!a.blanked && !fake.0.borrow().blanked);
        assert_eq!(a.folder, "power", "B only woke the screen");
    }

    #[test]
    fn about_opens_an_info_screen() {
        let (mut a, _) = app_fake();
        open_settings_at(&mut a, "about");
        press(&mut a, KeyCode::Enter);
        wait_script(&mut a);
        let run = a.script.as_ref().unwrap();
        assert_eq!(run.label, "About");
        assert!(run.succeeded(), "{}", run.summary());
        let lines = run.output.lines();
        assert_eq!(lines[0], concat!("pockettui35 ", env!("CARGO_PKG_VERSION")));
        assert!(lines.iter().any(|l| l.starts_with("host ")), "{lines:?}");
    }

    #[test]
    fn minute_refreshes_network_only() {
        let (mut a, fake) = app_fake();
        fake.0.borrow_mut().net = Some("wired".into());
        fake.0.borrow_mut().volume = None;
        a.on_minute();
        assert_eq!(a.values.net.as_deref(), Some("wired"));
        assert!(a.values.volume.is_some(), "volume not re-read each minute");
    }
}
