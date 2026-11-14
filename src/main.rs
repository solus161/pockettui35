//! pockettui35: a single-tasking launcher for the PocketTerm35.
//!
//! See `docs/design.md`.

mod app;
mod backlight;
mod config;
mod cpufreq;
mod history;
mod items;
mod keys;
mod launch;
mod script;
mod search;
mod signals;
mod system;
mod ui;

use std::io;
use std::process::ExitCode;
use std::time::{Duration, Instant};

use chrono::{Local, Timelike};
use crossterm::event::{self, Event, KeyEventKind};

use app::App;
use system::Host;

fn main() -> ExitCode {
    // First: SIGUSR1 (Start held) would otherwise end the process.
    signals::install();

    // The root sides of the power profile and the button layers, run
    // through sudo. No terminal.
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some(cpufreq::SUBCOMMAND) => return cpufreq::subcommand(&args[1..]),
        Some(keys::SUBCOMMAND) => return keys::subcommand(&args[1..]),
        _ => {}
    }

    // Enters raw mode and the alternate screen, and installs a panic hook
    // that restores the terminal.
    let mut terminal = ratatui::init();
    let result = run(&mut terminal);
    ratatui::restore();
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("pockettui35: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run(terminal: &mut ratatui::DefaultTerminal) -> io::Result<()> {
    let mut app = App::new(Box::new(Host::default()));
    // First frame before anything slow: asking the keyboard for brightness
    // can take 400 ms when its firmware is silent.
    terminal.draw(|f| ui::draw(f, &app))?;
    if let Err(e) = system::restore_power_profile() {
        app.message = Some(format!("Power profile not restored: {e}"));
    }
    // Raw buttons left on by a launcher that crashed during a game.
    if let Err(e) = keys::reset_if_left_raw() {
        app.message = Some(format!("Buttons not reset: {e}"));
    }
    app.refresh_values();
    terminal.draw(|f| ui::draw(f, &app))?;
    let mut drawn_minute = minute_now();

    while !app.should_quit {
        // Idle: sleep until input, the next minute boundary (the clock), or
        // the end of a "press A again" window, so idle CPU stays at zero.
        // While a script runs, also wake every 50 ms to take in its output.
        let mut timeout = until_next_minute();
        if app.script_running() {
            timeout = timeout.min(SCRIPT_POLL);
        }
        if let Some(deadline) = app.deadline() {
            timeout = timeout.min(deadline.saturating_duration_since(Instant::now()));
        }
        let mut redraw = false;
        // SIGUSR1 (Start held) may interrupt the wait: that is no error.
        if interrupted_ok(event::poll(timeout))?.unwrap_or(false) {
            match interrupted_ok(event::read())? {
                Some(Event::Key(key)) if key.kind == KeyEventKind::Press => {
                    app.on_key(key);
                    redraw = true;
                }
                Some(Event::Resize(_, _)) => redraw = true,
                _ => {}
            }
        }
        // Start held with no app running: stops a running script. In the
        // menu it does nothing.
        if signals::take_close() {
            redraw |= app.close_request();
        }
        if let Some(item) = app.pending.take().and_then(|id| app.tree.get(&id).cloned()) {
            app.message = launch::run(terminal, &item);
            // The user may have edited the config from inside the app.
            app.reload();
            redraw = true;
        }
        redraw |= app.pump_script();
        redraw |= app.tick(Instant::now());
        if minute_now() != drawn_minute {
            app.on_minute();
            redraw = true;
        }
        if redraw {
            terminal.draw(|f| ui::draw(f, &app))?;
            drawn_minute = minute_now();
        }
    }
    Ok(())
}

/// Turns `EINTR` into "nothing happened".
fn interrupted_ok<T>(r: io::Result<T>) -> io::Result<Option<T>> {
    match r {
        Ok(v) => Ok(Some(v)),
        Err(e) if e.kind() == io::ErrorKind::Interrupted => Ok(None),
        Err(e) => Err(e),
    }
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
