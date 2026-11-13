//! Drawing. 16 ANSI colours and ASCII/box-drawing glyphs only: this runs on
//! the Linux console.

use chrono::Local;
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph};

use crate::app::App;

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
    let title = match app.tree.get(&app.folder) {
        Some(folder) => format!(" {} ", folder.label),
        None => " Home ".to_string(),
    };
    let block = Block::new()
        .borders(Borders::LEFT | Borders::RIGHT | Borders::BOTTOM)
        .title(title);

    // Inner width minus the 2-column highlight symbol.
    let width = usize::from(area.width.saturating_sub(2 + 2));
    let items: Vec<ListItem> = app
        .tree
        .children(&app.folder)
        .iter()
        .filter_map(|id| app.tree.get(id))
        .map(|item| {
            if item.is_folder() {
                let label = &item.label;
                let pad = width.saturating_sub(label.chars().count() + 2);
                ListItem::new(format!("{label}{:pad$}> ", ""))
            } else {
                ListItem::new(item.label.as_str())
            }
        })
        .collect();
    let list = List::new(items)
        .block(block)
        .highlight_symbol("> ")
        .highlight_style(Style::new().fg(Color::Yellow).add_modifier(Modifier::BOLD));

    let mut state = ListState::default().with_selected(Some(app.selected));
    f.render_stateful_widget(list, area, &mut state);
}

fn draw_hints(f: &mut Frame, area: Rect, app: &App) {
    let key = Style::new().fg(Color::Black).bg(Color::Cyan);
    let mut hints = vec![
        Span::styled(" A ", key),
        Span::raw(" open  "),
        Span::styled(" B ", key),
        Span::raw(" back  "),
        Span::raw(" type to search"),
    ];
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
        let app = App::with_tree(crate::items::Tree::build(merged, warnings));
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
        assert!(text.contains("Home"));
        assert!(text.contains("Settings") && text.contains("> │"));
        assert!(!text.contains("config warning"));
    }
}
