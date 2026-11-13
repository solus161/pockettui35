//! pockettui35: a single-tasking launcher for the PocketTerm35.
//!
//! See `docs/design.md`.

mod app;
mod config;
mod items;
mod ui;

use std::io;
use std::time::Duration;

use chrono::{Local, Timelike};
use crossterm::event::{self, Event, KeyEventKind};

use app::App;

fn main() -> io::Result<()> {
    // Enters raw mode and the alternate screen, and installs a panic hook
    // that restores the terminal.
    let mut terminal = ratatui::init();
    let result = run(&mut terminal);
    ratatui::restore();
    result
}

fn run(terminal: &mut ratatui::DefaultTerminal) -> io::Result<()> {
    let mut app = App::new();
    terminal.draw(|f| ui::draw(f, &app))?;

    while !app.should_quit {
        // Sleep until input or the next minute boundary (the clock).
        // No fixed-rate tick: idle CPU stays at zero.
        if event::poll(until_next_minute())? {
            match event::read()? {
                Event::Key(key) if key.kind == KeyEventKind::Press => app.on_key(key),
                Event::Resize(_, _) => {}
                _ => continue,
            }
        }
        terminal.draw(|f| ui::draw(f, &app))?;
    }
    Ok(())
}

/// Time left until the wall clock reaches the next whole minute.
fn until_next_minute() -> Duration {
    let now = Local::now();
    let elapsed_ms =
        u64::from(now.second()) * 1000 + u64::from(now.timestamp_subsec_millis() % 1000);
    Duration::from_millis(60_000 - elapsed_ms.min(59_999))
}
