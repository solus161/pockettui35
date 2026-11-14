//! pockettui35: a single-tasking launcher for the PocketTerm35.
//!
//! See `docs/design.md`.

mod app;
mod config;
mod history;
mod items;
mod launch;
mod script;
mod search;
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
    let mut drawn_minute = minute_now();

    while !app.should_quit {
        // Idle: sleep until input or the next minute boundary (the clock),
        // so idle CPU stays at zero. While a script runs, also wake every
        // 50 ms to take in its output.
        let timeout = if app.script_running() {
            SCRIPT_POLL.min(until_next_minute())
        } else {
            until_next_minute()
        };
        let mut redraw = false;
        if event::poll(timeout)? {
            match event::read()? {
                Event::Key(key) if key.kind == KeyEventKind::Press => {
                    app.on_key(key);
                    redraw = true;
                }
                Event::Resize(_, _) => redraw = true,
                _ => {}
            }
        }
        if let Some(item) = app.pending.take().and_then(|id| app.tree.get(&id).cloned()) {
            app.message = launch::run(terminal, &item);
            // The user may have edited the config from inside the app.
            app.reload();
            redraw = true;
        }
        redraw |= app.pump_script();
        redraw |= minute_now() != drawn_minute;
        if redraw {
            terminal.draw(|f| ui::draw(f, &app))?;
            drawn_minute = minute_now();
        }
    }
    Ok(())
}

/// How often a running script's output is taken in.
const SCRIPT_POLL: Duration = Duration::from_millis(50);

fn minute_now() -> u32 {
    let now = Local::now();
    now.hour() * 60 + now.minute()
}

/// Time left until the wall clock reaches the next whole minute.
fn until_next_minute() -> Duration {
    let now = Local::now();
    let elapsed_ms =
        u64::from(now.second()) * 1000 + u64::from(now.timestamp_subsec_millis() % 1000);
    Duration::from_millis(60_000 - elapsed_ms.min(59_999))
}
