//! Drawing. 16 ANSI colours and ASCII/box-drawing glyphs only: this runs on
//! the Linux console.

use chrono::Local;
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph};

use crate::app::{App, Row};
use crate::items::{Action, HOME};
use crate::script::{ScriptRun, Status};

pub fn draw(f: &mut Frame, app: &App) {
    if let Some(run) = &app.script {
        let [status, body, hints] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Min(0),
            Constraint::Length(1),
        ])
        .areas(f.area());
        draw_status(f, status, app);
        draw_script(f, body, run);
        draw_script_hints(f, hints, run);
        return;
    }
    let [status, search, menu, hints] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(3),
        Constraint::Min(0),
        Constraint::Length(1),
    ])
    .areas(f.area());

    draw_status(f, status, app);
    draw_search(f, search, app);
    draw_menu(f, menu, app);
    draw_hints(f, hints, app);
}

fn draw_status(f: &mut Frame, area: Rect, app: &App) {
    let [left, right] =
        Layout::horizontal([Constraint::Min(0), Constraint::Length(20)]).areas(area);
    let style = Style::new().fg(Color::Black).bg(Color::Gray);

    let net = app.values.net.as_deref().unwrap_or("net ?");
    let profile = app.values.profile.map_or("custom", |p| p.label());
    f.render_widget(
        Paragraph::new(format!(" {net}   {profile}")).style(style),
        left,
    );

    let clock = Local::now().format("%a %d %b  %H:%M ").to_string();
    f.render_widget(Paragraph::new(clock).style(style).right_aligned(), right);
}

fn draw_search(f: &mut Frame, area: Rect, app: &App) {
    let dim = Style::new().fg(Color::DarkGray);
    let block = Block::new().borders(Borders::ALL);
    let text = if app.searching() {
        Line::from(vec![
            Span::styled("/ ", dim),
            Span::styled(
                app.query.as_str(),
                Style::new().fg(Color::White).add_modifier(Modifier::BOLD),
            ),
            // A drawn cursor: the real one stays hidden so its blink timer
            // does not wake the CPU.
            Span::styled("_", Style::new().fg(Color::Yellow)),
        ])
    } else {
        Line::from(vec![
            Span::styled("/ ", dim),
            Span::styled("type to search", dim),
        ])
    };
    let inner = block.inner(area);
    f.render_widget(Paragraph::new(text).block(block), area);
    if app.searching() {
        let n = app.hits.len();
        let s = if n == 1 { "" } else { "s" };
        let count = Paragraph::new(format!("{n} result{s} "))
            .style(dim)
            .right_aligned();
        f.render_widget(count, inner);
    }
}

/// The script output screen. Lines longer than the width are cut, not
/// wrapped, so one output line is one screen row and scrolling stays exact.
fn draw_script(f: &mut Frame, area: Rect, run: &ScriptRun) {
    let dim = Style::new().fg(Color::DarkGray);
    let summary_style = if run.running() {
        Style::new().fg(Color::Yellow)
    } else if run.succeeded() {
        Style::new().fg(Color::Green)
    } else {
        Style::new().fg(Color::Red).add_modifier(Modifier::BOLD)
    };
    let mut block = Block::new()
        .borders(Borders::ALL)
        .title(format!(" {} ", run.label))
        .title_bottom(Line::styled(format!(" {} ", run.summary()), summary_style).right_aligned());

    let lines = run.output.lines();
    let inner = block.inner(area);
    let height = usize::from(inner.height);
    run.view_height.set(inner.height);
    let from_bottom = run.from_bottom.min(lines.len().saturating_sub(height));
    let end = lines.len() - from_bottom;
    let start = end.saturating_sub(height);
    if from_bottom > 0 {
        block = block.title_bottom(Line::styled(format!(" v {from_bottom} more "), dim));
    }

    let text: Vec<Line> = lines[start..end]
        .iter()
        .map(|l| Line::raw(l.as_str()))
        .collect();
    f.render_widget(Paragraph::new(text).block(block), area);
}

fn draw_script_hints(f: &mut Frame, area: Rect, run: &ScriptRun) {
    let key = Style::new().fg(Color::Black).bg(Color::Cyan);
    let mut hints = Vec::new();
    match run.status {
        Status::Running { term_sent, .. } => {
            let b = if term_sent { " kill  " } else { " stop  " };
            hints.extend([Span::styled(" B ", key), Span::raw(b)]);
        }
        _ => hints.extend([
            Span::styled(" A ", key),
            Span::raw(" run again  "),
            Span::styled(" B ", key),
            Span::raw(" close  "),
        ]),
    }
    hints.extend([Span::styled(" ^/v ", key), Span::raw(" scroll")]);
    f.render_widget(Paragraph::new(Line::from(hints)), area);
}

/// A label with the characters at `indices` highlighted.
fn label_spans<'a>(label: &'a str, indices: &[u32]) -> Vec<Span<'a>> {
    if indices.is_empty() {
        return vec![Span::raw(label)];
    }
    let hl = Style::new().fg(Color::Cyan).add_modifier(Modifier::BOLD);
    let mut spans = Vec::new();
    let mut run_start = 0;
    let mut run_hl = false;
    for (i, (byte, _)) in label.char_indices().enumerate() {
        let is_hl = u32::try_from(i).is_ok_and(|i| indices.binary_search(&i).is_ok());
        if is_hl != run_hl {
            if byte > run_start {
                let text = &label[run_start..byte];
                spans.push(if run_hl {
                    Span::styled(text, hl)
                } else {
                    Span::raw(text)
                });
            }
            run_start = byte;
            run_hl = is_hl;
        }
    }
    let text = &label[run_start..];
    spans.push(if run_hl {
        Span::styled(text, hl)
    } else {
        Span::raw(text)
    });
    spans
}

fn draw_menu(f: &mut Frame, area: Rect, app: &App) {
    let title = if app.searching() {
        " Search ".to_string()
    } else {
        format!(" {} ", breadcrumb(app))
    };
    let block = Block::new()
        .borders(Borders::LEFT | Borders::RIGHT | Borders::BOTTOM)
        .title(title);

    let dim = Style::new().fg(Color::DarkGray);
    // Inner width minus the 2-column highlight symbol.
    let width = usize::from(area.width.saturating_sub(2 + 2));
    let rows = app.rows();
    let items: Vec<ListItem> = rows
        .iter()
        .map(|row| match row {
            Row::Header(text) => ListItem::new(Line::styled(*text, dim)),
            Row::Separator => ListItem::new(Line::styled("─".repeat(width), dim)),
            Row::Item(key) => {
                let Some(item) = app.tree.get(&key.id) else {
                    return ListItem::new("");
                };
                // Right column: `>` for folders; in Recent and in search
                // results, the folder an item lives in when that is not Home.
                // Right column: `>` for folders, the value for settings, and
                // in Recent and search results the folder an item lives in
                // when that is not Home.
                let folder = (key.recent || app.searching())
                    .then(|| app.tree.parent(&item.id))
                    .flatten()
                    .filter(|p| *p != HOME)
                    .and_then(|p| app.tree.get(p))
                    .map(|p| p.label.as_str());
                let right = if item.is_folder() {
                    Some(Span::raw("> "))
                } else if let Some(value) = app.value_text(item) {
                    let text = match folder {
                        Some(f) => format!("{value}   {f} "),
                        None => format!("{value} "),
                    };
                    Some(Span::raw(text))
                } else {
                    folder.map(|f| Span::styled(format!("{f} "), dim))
                };
                let indices = app
                    .hits
                    .iter()
                    .find(|h| h.id == item.id)
                    .map_or(&[][..], |h| h.indices.as_slice());
                let mut spans = label_spans(&item.label, indices);
                if let Some(right) = right {
                    let used = item.label.chars().count() + right.content.chars().count();
                    spans.push(Span::raw(" ".repeat(width.saturating_sub(used))));
                    spans.push(right);
                }
                ListItem::new(Line::from(spans))
            }
        })
        .collect();
    let list = List::new(items)
        .block(block)
        .highlight_symbol("> ")
        .highlight_style(Style::new().fg(Color::Yellow).add_modifier(Modifier::BOLD));

    let mut state = ListState::default().with_selected(app.selected_index(&rows));
    f.render_stateful_widget(list, area, &mut state);
}

/// `Home / Settings`, from the root to the current folder.
fn breadcrumb(app: &App) -> String {
    let mut parts = Vec::new();
    let mut cur = app.folder.as_str();
    while cur != HOME {
        let Some(item) = app.tree.get(cur) else { break };
        parts.push(item.label.as_str());
        cur = app.tree.parent(cur).unwrap_or(HOME);
    }
    parts.push("Home");
    parts.reverse();
    parts.join(" / ")
}

fn draw_hints(f: &mut Frame, area: Rect, app: &App) {
    if let Some(msg) = &app.message {
        let line = Line::styled(format!(" {msg}"), Style::new().fg(Color::Yellow));
        f.render_widget(Paragraph::new(line), area);
        return;
    }
    let key = Style::new().fg(Color::Black).bg(Color::Cyan);
    let mut hints = Vec::new();
    // What A does depends on the selected item.
    let a = app.selected_item().and_then(|item| match item.action {
        Action::Folder => Some("open"),
        Action::App { .. } => Some("launch"),
        Action::Script { .. } | Action::Builtin { .. } => Some("run"),
        Action::Value(_) | Action::Choice(_) => None,
    });
    if let Some(a) = a {
        hints.extend([Span::styled(" A ", key), Span::raw(format!(" {a}  "))]);
    }
    if app
        .selected_item()
        .is_some_and(|i| matches!(i.action, Action::Value(_) | Action::Choice(_)))
    {
        hints.extend([Span::styled(" </> ", key), Span::raw(" adjust  ")]);
    }
    if app.searching() {
        hints.extend([Span::styled(" B ", key), Span::raw(" clear  ")]);
    } else {
        if app.folder != HOME {
            hints.extend([Span::styled(" B ", key), Span::raw(" back  ")]);
        }
        hints.push(Span::raw(" type to search"));
    }
    // Config problems never stop the launcher, but must not go unnoticed.
    // A screen to read them comes later.
    let n = app.tree.warnings.len();
    if n > 0 {
        let s = if n == 1 { "" } else { "s" };
        hints.push(Span::styled(
            format!("   ! {n} config warning{s}"),
            Style::new().fg(Color::Red).add_modifier(Modifier::BOLD),
        ));
    }
    f.render_widget(Paragraph::new(Line::from(hints)), area);
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{Terminal, backend::TestBackend};

    /// Renders the home screen at the device's 80x30 console size.
    #[test]
    fn renders_at_device_size() {
        let mut terminal = Terminal::new(TestBackend::new(80, 30)).unwrap();
        // Shipped config only, so the test does not depend on the machine's
        // ~/.config.
        let (merged, warnings) = crate::config::merge_layers(&[crate::config::Layer {
            name: "shipped".into(),
            text: crate::config::SHIPPED.into(),
        }]);
        let mut app = App::with(
            crate::items::Tree::build(merged, warnings),
            crate::history::History::in_memory(),
            Box::new(crate::system::fake::Fake::working()),
        );
        app.history.record("wifi");
        app.history.record("browser");
        app.fix_selection();
        terminal.draw(|f| draw(f, &app)).unwrap();
        let buf = terminal.backend().buffer();
        let text: String = (0..buf.area.height)
            .map(|y| {
                (0..buf.area.width)
                    .map(|x| buf[(x, y)].symbol())
                    .collect::<String>()
                    + "\n"
            })
            .collect();
        println!("{text}");
        assert!(text.contains("> Chromium"));
        assert!(text.contains("Recent"));
        // Recent shows where an item lives when it is not in Home.
        let wifi = text.lines().find(|l| l.contains("Wi-Fi")).unwrap();
        assert!(wifi.trim_end().ends_with("Settings │"), "{wifi}");
        assert!(text.contains("Home"));
        assert!(text.contains("Settings") && text.contains("> │"));
        assert!(!text.contains("config warning"));
    }

    fn shipped_app() -> App {
        let (merged, warnings) = crate::config::merge_layers(&[crate::config::Layer {
            name: "shipped".into(),
            text: crate::config::SHIPPED.into(),
        }]);
        App::with(
            crate::items::Tree::build(merged, warnings),
            crate::history::History::in_memory(),
            Box::new(crate::system::fake::Fake::working()),
        )
    }

    fn render(app: &App) -> String {
        let mut terminal = Terminal::new(TestBackend::new(80, 30)).unwrap();
        terminal.draw(|f| draw(f, app)).unwrap();
        let buf = terminal.backend().buffer();
        (0..buf.area.height)
            .map(|y| {
                (0..buf.area.width)
                    .map(|x| buf[(x, y)].symbol())
                    .collect::<String>()
                    + "\n"
            })
            .collect()
    }

    #[test]
    fn renders_search() {
        use crossterm::event::{KeyCode, KeyEvent};
        let mut app = shipped_app();
        for c in "rpi".chars() {
            app.on_key(KeyEvent::from(KeyCode::Char(c)));
        }
        let text = render(&app);
        println!("{text}");
        assert!(text.contains("/ rpi_"));
        assert!(text.contains(" Search "));
        assert!(text.contains("result"));
        assert!(text.contains(" B  clear"));
        assert!(!text.contains("type to search"));
        let hit = text
            .lines()
            .find(|l| l.contains("Raspberry Pi config"))
            .unwrap();
        assert!(hit.trim_end().ends_with("Settings │"), "{hit}");
    }

    #[test]
    fn label_spans_split_into_runs() {
        let spans = label_spans("Raspberry Pi config", &[0, 10, 11]);
        let parts: Vec<(&str, bool)> = spans
            .iter()
            .map(|s| (s.content.as_ref(), s.style.fg == Some(Color::Cyan)))
            .collect();
        assert_eq!(
            parts,
            [
                ("R", true),
                ("aspberry ", false),
                ("Pi", true),
                (" config", false)
            ]
        );
        assert_eq!(label_spans("Files", &[]).len(), 1);
    }

    #[test]
    fn renders_script_screen() {
        let mut app = shipped_app();
        let mut run = ScriptRun::start("t", "Sync notes", "seq 1 40; exit 2");
        let start = std::time::Instant::now();
        while run.running() {
            run.pump();
            assert!(start.elapsed().as_secs() < 5);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
        run.pump();
        run.from_bottom = 3;
        app.script = Some(run);
        let text = render(&app);
        println!("{text}");
        assert!(text.contains(" Sync notes "));
        assert!(text.contains(" failed, exit 2 "));
        assert!(text.contains(" v 3 more "));
        assert!(text.contains("37") && !text.contains("38"));
        assert!(text.contains(" A  run again"));
        assert!(!text.contains("type to search"));
    }

    #[test]
    fn renders_settings_values_and_status() {
        use crossterm::event::{KeyCode, KeyEvent};
        let mut app = shipped_app();
        app.refresh_values();
        // Home: Chromium, Files, Games, Power, Settings, Terminal.
        for _ in 0..4 {
            app.on_key(KeyEvent::from(KeyCode::Down));
        }
        app.on_key(KeyEvent::from(KeyCode::Enter));
        let text = render(&app);
        println!("{text}");
        assert!(text.starts_with(" wifi Home   balanced"));
        assert!(text.contains(" Home / Settings "));
        let line = |label: &str| {
            text.lines()
                .find(|l| l.contains(label))
                .unwrap()
                .to_string()
        };
        assert!(
            line("Volume").ends_with("[━━━━──────]  40% │"),
            "{}",
            line("Volume")
        );
        assert!(line("Brightness").ends_with("[━━━━━━━━──]  80% │"));
        assert!(line("Power profile").ends_with("< balanced > │"));
    }
}
