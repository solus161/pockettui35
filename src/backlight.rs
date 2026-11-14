//! The backlight through the keyboard's RP2040, which owns the PWM pin.
//!
//! Ported from pt35-desktop's `pt35d/src/backlight.rs`, same protocol and
//! the same care for a port in any state; `libc` replaces the hand-written
//! FFI.
//!
//! Linux has no `/sys/class/backlight` on this board. The patched firmware
//! listens on its USB console for `B<0-100>` and `B?` and answers
//! `PT35 B=<percent>`, with a 5% floor. Stock firmware ignores the lines,
//! the reply never comes, and brightness stays what it was: Fn with `-` and
//! `=` on the keyboard.

use std::io::{Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// What asking the keyboard got.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Answer {
    Level(u8),
    /// No reply: stock firmware, or no console at all.
    Silent,
    /// `pt35-kbd` holds the port. Says nothing about the firmware.
    Busy,
}

/// Asks for the level, which Fn with `-` and `=` changes behind our back.
pub fn read() -> Answer {
    match console() {
        Some(path) => ask(&path, "B?"),
        None => Answer::Silent,
    }
}

/// Sets a level; the answer is what the firmware says it is now.
pub fn set(percent: u8) -> Answer {
    match console() {
        Some(path) => ask(&path, &format!("B{}", percent.min(100))),
        None => Answer::Silent,
    }
}

/// One line to the firmware and its answer, in bounded time whatever state
/// the port is in. Stock firmware never reads its console: once its buffer
/// is full a blocking write would wait for ever, and a close with unsent
/// bytes waits 30 s for them.
fn ask(path: &Path, command: &str) -> Answer {
    let Ok(mut port) = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .custom_flags(libc::O_NONBLOCK)
        .open(path)
    else {
        return Answer::Silent;
    };
    // `pt35-kbd` holds this lock while it writes the firmware: a line from
    // us landing in its REPL breaks the flash, and so would changing the
    // port's settings under it. Skip, do not wait.
    if !try_lock(&port) {
        return Answer::Busy;
    }
    // Raw and no echo, set every time: `pt35-kbd` leaves its own settings
    // behind. Set here and not by `stty`: the board sends a title on every
    // open, and a new tty echoes it back until the echo is off. The pt35
    // firmware up to 0.4.1 died on that echo. No hang-up on close: every
    // request opens the port anew.
    if !make_raw(&port) {
        return Answer::Silent;
    }
    let mut answer = Answer::Silent;
    if port.write_all(format!("{command}\n").as_bytes()).is_ok() {
        let deadline = Instant::now() + Duration::from_millis(400);
        let mut seen = Vec::new();
        let mut chunk = [0u8; 256];
        while Instant::now() < deadline {
            match port.read(&mut chunk) {
                Ok(n) if n > 0 => {
                    seen.extend_from_slice(&chunk[..n]);
                    if let Some(level) = parse_reply(&String::from_utf8_lossy(&seen)) {
                        answer = Answer::Level(level);
                        break;
                    }
                }
                Ok(_) => std::thread::sleep(Duration::from_millis(10)),
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(_) => break,
            }
        }
    }
    if answer == Answer::Silent {
        discard_unsent(&port);
    }
    answer
}

/// `stty raw -echo -hupcl` on an open port.
fn make_raw(file: &std::fs::File) -> bool {
    // SAFETY: a valid open fd, and a zeroed `termios` filled by tcgetattr.
    unsafe {
        let mut t: libc::termios = std::mem::zeroed();
        if libc::tcgetattr(file.as_raw_fd(), &mut t) != 0 {
            return false;
        }
        libc::cfmakeraw(&mut t);
        t.c_cflag &= !libc::HUPCL;
        libc::tcsetattr(file.as_raw_fd(), libc::TCSANOW, &t) == 0
    }
}

/// An exclusive, non-blocking flock, released when the file closes.
fn try_lock(file: &std::fs::File) -> bool {
    // SAFETY: a valid open fd and plain integer flags.
    unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) == 0 }
}

/// Drops what the keyboard has not taken, so the close does not wait for it.
fn discard_unsent(file: &std::fs::File) {
    // SAFETY: a valid open fd and a plain integer selector.
    unsafe {
        libc::tcflush(file.as_raw_fd(), libc::TCOFLUSH);
    }
}

/// The keyboard's console under /dev/serial/by-id, else the first ACM port.
fn console() -> Option<PathBuf> {
    let named = std::fs::read_dir("/dev/serial/by-id").ok().and_then(|dir| {
        dir.flatten()
            .map(|e| e.path())
            .find(|p| p.to_string_lossy().contains("Pico"))
    });
    named.or_else(|| Some(PathBuf::from("/dev/ttyACM0")).filter(|p| p.exists()))
}

/// The last whole `PT35 B=<n>` line in what came back. The firmware prints
/// debug lines on the same port, and CircuitPython a title-bar escape with no
/// newline the moment the port opens, so the reply is searched for, not read
/// as a line. Its line end is what says the number is all there.
pub fn parse_reply(text: &str) -> Option<u8> {
    text.rmatch_indices("PT35 B=").find_map(|(at, tag)| {
        let after = &text[at + tag.len()..];
        let digits = after.len() - after.trim_start_matches(|c: char| c.is_ascii_digit()).len();
        if digits == 0 || !after[digits..].starts_with(['\r', '\n']) {
            return None;
        }
        after[..digits].parse().ok()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_reply_is_found_among_debug_lines() {
        assert_eq!(parse_reply("[HID] ready\r\nPT35 B=40\r\n"), Some(40));
        assert_eq!(
            parse_reply("PT35 B=40\nPT35 B=55\n"),
            Some(55),
            "the latest"
        );
        assert_eq!(parse_reply("bl_pwm: 3000\n"), None);
        let opened = "\x1b]0;code.py | 10.0.0\x1b\\PT35 B=92\r\n";
        assert_eq!(parse_reply(opened), Some(92), "after the title escape");
    }

    #[test]
    fn half_a_reply_is_not_one() {
        assert_eq!(parse_reply("PT35 B=4"), None, "the 0 is still on its way");
        assert_eq!(parse_reply("PT35 B=40\r\nPT35 B=5"), Some(40));
        assert_eq!(parse_reply("PT35 B=\r\n"), None);
    }
}
