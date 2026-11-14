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

pub fn draw(f: &mut Frame, app: &App) {
    let [status, search, menu, hints] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(3),
        Constraint::Min(0),
        Constraint::Length(1),
    ])
    .areas(f.area());

    draw_status(f, status);
    draw_search(f, search);
    draw_menu(f, menu, app);
    draw_hints(f, hints, app);
}

fn draw_status(f: &mut Frame, area: Rect) {
    let [left, right] =
        Layout::horizontal([Constraint::Min(0), Constraint::Length(20)]).areas(area);
    let style = Style::new().fg(Color::Black).bg(Color::Gray);

    // Static until step 7 (NetworkManager, power profile).
    f.render_widget(Paragraph::new(" wlan0 --   balanced").style(style), left);

    let clock = Local::now().format("%a %d %b  %H:%M ").to_string();
    f.render_widget(Paragraph::new(clock).style(style).right_aligned(), right);
}

fn draw_search(f: &mut Frame, area: Rect) {
    let block = Block::new().borders(Borders::ALL);
    let text = Line::from(vec![
        Span::styled("/ ", Style::new().fg(Color::DarkGray)),
        Span::styled("type to search", Style::new().fg(Color::DarkGray)),
    ]);
    f.render_widget(Paragraph::new(text).block(block), area);
}

fn draw_menu(f: &mut Frame, area: Rect, app: &App) {
    let block = Block::new()
        .borders(Borders::LEFT | Borders::RIGHT | Borders::BOTTOM)
        .title(format!(" {} ", breadcrumb(app)));

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
                // Right column: `>` for folders; in Recent, the folder an
                // item lives in when that is not Home.
                let right = if item.is_folder() {
                    Some(Span::raw("> "))
                } else if key.recent {
                    app.tree
                        .parent(&item.id)
                        .filter(|p| *p != HOME)
                        .and_then(|p| app.tree.get(p))
                        .map(|p| Span::styled(format!("{} ", p.label), dim))
                } else {
                    None
                };
                let mut spans = vec![Span::raw(item.label.as_str())];
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
    if app.folder != HOME {
        hints.extend([Span::styled(" B ", key), Span::raw(" back  ")]);
    }
    hints.push(Span::raw(" type to search"));
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
}
