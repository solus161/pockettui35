//! Running apps. The launcher gives up the screen, runs the app in the
//! foreground, waits for it, and takes the screen back.
//!
//! Each app runs in its own process group, made the terminal's foreground
//! group, as a shell runs a job. Ctrl+C and Ctrl+\ then reach the app and
//! not the launcher. The launcher waits with no CPU use.

use std::fs::File;
use std::io::{self, Write};
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::path::PathBuf;
use std::process::{Command, ExitStatus, Stdio};

use crossterm::cursor::{MoveTo, Show};
use crossterm::execute;
use crossterm::terminal::{
    Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};

use crate::items::{Action, Item, Kind};

/// The compositor for `wayland` apps.
const CAGE: &str = "cage";

/// Runs an item that takes the screen. Returns a message for the hint bar,
/// or `None` when the app exited cleanly.
pub fn run(terminal: &mut ratatui::DefaultTerminal, item: &Item) -> Option<String> {
    let (kind, exec) = match &item.action {
        Action::App { kind, exec, .. } => (*kind, exec.as_str()),
        // Script output screen: step 6. Built-ins: step 7.
        Action::Script { .. } => return Some("Scripts are not supported yet".into()),
        Action::Builtin { .. } => return Some(format!("{}: not supported yet", item.label)),
        Action::Folder | Action::Value(_) | Action::Choice(_) => return None,
    };

    let mut cmd = command(kind, exec);
    // cage and Wayland clients log to stderr. Keep that off the console and
    // in a file, so a failed launch can be looked into.
    let log = (kind == Kind::Wayland)
        .then(log_path)
        .flatten()
        .and_then(|p| redirect_output(&mut cmd, &p).ok().map(|()| p));

    if let Err(e) = suspend() {
        let _ = resume(terminal);
        return Some(format!("Could not release the screen: {e}"));
    }
    let result = run_foreground(&mut cmd);
    let resumed = resume(terminal);

    let msg = match result {
        Err(e) if e.kind() == io::ErrorKind::NotFound && kind == Kind::Wayland => {
            Some(format!("{CAGE} is not installed"))
        }
        Err(e) => Some(format!("{}: {e}", item.label)),
        Ok(status) => describe(&item.label, status, log.is_some()),
    };
    match resumed {
        Ok(()) => msg,
        Err(e) => Some(format!("Could not take the screen back: {e}")),
    }
}

/// The command line for an app. `exec` runs through `sh -c`, so it may use
/// quoting, `~` and `$VARS`.
fn command(kind: Kind, exec: &str) -> Command {
    let mut cmd = match kind {
        Kind::Tty | Kind::Kms => Command::new("sh"),
        Kind::Wayland => {
            let mut c = Command::new(CAGE);
            c.args(["--", "sh"]);
            c
        }
    };
    cmd.args(["-c", exec]);
    cmd
}

fn redirect_output(cmd: &mut Command, path: &PathBuf) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let out = File::create(path)?;
    let err = out.try_clone()?;
    cmd.stdout(Stdio::from(out)).stderr(Stdio::from(err));
    Ok(())
}

/// Output of the last `wayland` launch.
fn log_path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_STATE_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/state")))?;
    Some(base.join("pockettui35").join("launch.log"))
}

/// A hint-bar message for how an app ended. A clean exit says nothing.
fn describe(label: &str, status: ExitStatus, logged: bool) -> Option<String> {
    let see_log = if logged { ", see launch.log" } else { "" };
    match status.code() {
        Some(0) => None,
        Some(127) => Some(format!("{label}: command not found{see_log}")),
        Some(code) => Some(format!("{label} exited with status {code}{see_log}")),
        None => status
            .signal()
            .map(|sig| format!("{label} was killed by signal {sig}{see_log}")),
    }
}

/// Leaves the launcher's screen: raw mode off, primary screen, cleared,
/// cursor shown. The app starts on a clean console.
fn suspend() -> io::Result<()> {
    disable_raw_mode()?;
    let mut out = io::stdout();
    execute!(
        out,
        LeaveAlternateScreen,
        Clear(ClearType::All),
        MoveTo(0, 0),
        Show
    )?;
    out.flush()
}

/// Takes the screen back and forces a full redraw on the next frame.
fn resume(terminal: &mut ratatui::DefaultTerminal) -> io::Result<()> {
    enable_raw_mode()?;
    execute!(io::stdout(), EnterAlternateScreen, Clear(ClearType::All))?;
    // Not `terminal.clear()`: it reads the cursor position (ESC[6n) and fails
    // when the terminal does not answer. Swapping twice blanks both buffers,
    // so the next draw repaints every cell.
    terminal.swap_buffers();
    terminal.swap_buffers();
    Ok(())
}

/// Spawns `cmd` in a new process group, gives it the terminal, and waits
/// for it to exit.
fn run_foreground(cmd: &mut Command) -> io::Result<ExitStatus> {
    cmd.process_group(0);
    let child = cmd.spawn()?;
    let pid = libc::pid_t::try_from(child.id()).map_err(io::Error::other)?;

    // Fails harmlessly when stdin is not a terminal (tests, pipes).
    let tty = libc::STDIN_FILENO;
    // SAFETY: plain syscalls on our own fd and child pid.
    unsafe { libc::tcsetpgrp(tty, pid) };

    let status = wait(pid);

    // We are now a background process group. Taking the terminal back would
    // raise SIGTTOU and stop us, so block it for the call.
    // SAFETY: a signal mask change around one syscall, restored after.
    unsafe {
        let mut set: libc::sigset_t = std::mem::zeroed();
        let mut old: libc::sigset_t = std::mem::zeroed();
        libc::sigemptyset(&mut set);
        libc::sigaddset(&mut set, libc::SIGTTOU);
        libc::pthread_sigmask(libc::SIG_BLOCK, &set, &mut old);
        libc::tcsetpgrp(tty, libc::getpgrp());
        libc::pthread_sigmask(libc::SIG_SETMASK, &old, std::ptr::null_mut());
    }
    status
}

/// Waits for `pid` to exit. If it stops (Ctrl+Z), it is resumed: there is no
/// job control here, and a stopped app would hang the launcher.
fn wait(pid: libc::pid_t) -> io::Result<ExitStatus> {
    loop {
        let mut status = 0;
        // SAFETY: waits on our own child; `status` is a valid out-pointer.
        let r = unsafe { libc::waitpid(pid, &mut status, libc::WUNTRACED) };
        if r == -1 {
            let e = io::Error::last_os_error();
            if e.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return Err(e);
        }
        if libc::WIFSTOPPED(status) {
            // SAFETY: signals the child's own process group.
            unsafe { libc::kill(-pid, libc::SIGCONT) };
            continue;
        }
        return Ok(ExitStatus::from_raw(status));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(cmd: &Command) -> Vec<String> {
        std::iter::once(cmd.get_program())
            .chain(cmd.get_args())
            .map(|s| s.to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn commands_per_kind() {
        assert_eq!(
            args(&command(Kind::Tty, "htop -d 10")),
            ["sh", "-c", "htop -d 10"]
        );
        assert_eq!(args(&command(Kind::Kms, "mpv x")), ["sh", "-c", "mpv x"]);
        assert_eq!(
            args(&command(Kind::Wayland, "chromium")),
            ["cage", "--", "sh", "-c", "chromium"]
        );
    }

    #[test]
    fn exit_status_reaches_us() {
        let status = run_foreground(&mut command(Kind::Tty, "exit 3")).unwrap();
        assert_eq!(status.code(), Some(3));
    }

    #[test]
    fn stopped_child_is_resumed() {
        // Without the SIGCONT the wait would hang here.
        let status = run_foreground(&mut command(Kind::Tty, "kill -STOP $$; exit 4")).unwrap();
        assert_eq!(status.code(), Some(4));
    }

    #[test]
    fn missing_program_is_an_error() {
        let mut cmd = Command::new("pt35-no-such-program");
        let err = run_foreground(&mut cmd).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::NotFound);
    }

    #[test]
    fn messages() {
        let ok = ExitStatus::from_raw(0);
        let code1 = ExitStatus::from_raw(1 << 8);
        let code127 = ExitStatus::from_raw(127 << 8);
        let killed = ExitStatus::from_raw(libc::SIGKILL);
        assert_eq!(describe("X", ok, true), None);
        assert_eq!(
            describe("X", code1, false).unwrap(),
            "X exited with status 1"
        );
        assert_eq!(
            describe("X", code127, true).unwrap(),
            "X: command not found, see launch.log"
        );
        assert_eq!(
            describe("X", killed, false).unwrap(),
            "X was killed by signal 9"
        );
    }
}
