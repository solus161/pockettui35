//! Running a `script` item with its output shown in a launcher screen.
//!
//! stdout and stderr share one pipe, so they stay in order. One thread reads
//! the pipe, another waits for the exit. The script gets no input
//! (`/dev/null`) and `TERM=dumb`, so tools skip colours and progress bars.
//! What escapes still arrive are stripped.

use std::cell::Cell;
use std::io::{self, Read};
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::process::{Command, ExitStatus, Stdio};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread;

/// Lines kept. Older ones are dropped.
const MAX_LINES: usize = 10_000;

enum Msg {
    Output(Vec<u8>),
    Exited(io::Result<ExitStatus>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    /// `term_sent`: B was pressed once; the next B kills.
    Running {
        pid: libc::pid_t,
        term_sent: bool,
    },
    Exited(ExitStatus),
    /// Could not start, or the wait failed.
    Failed,
}

pub struct ScriptRun {
    pub id: String,
    pub label: String,
    exec: String,
    pub output: Output,
    pub status: Status,
    /// Why it failed, when `status` is `Failed`.
    pub error: Option<String>,
    rx: Option<Receiver<Msg>>,
    /// Lines scrolled up from the end; 0 follows new output.
    pub from_bottom: usize,
    /// Rows the output area had at the last draw, for scrolling by page.
    pub view_height: Cell<u16>,
}

impl ScriptRun {
    pub fn start(id: &str, label: &str, exec: &str) -> Self {
        let mut run = Self {
            id: id.to_string(),
            label: label.to_string(),
            exec: exec.to_string(),
            output: Output::default(),
            status: Status::Failed,
            error: None,
            rx: None,
            from_bottom: 0,
            view_height: Cell::new(0),
        };
        run.spawn();
        run
    }

    /// Runs the script again in the same screen.
    pub fn restart(&mut self) {
        if self.running() {
            return;
        }
        self.output = Output::default();
        self.error = None;
        self.from_bottom = 0;
        self.spawn();
    }

    fn spawn(&mut self) {
        match spawn(&self.exec) {
            Ok((pid, rx)) => {
                self.status = Status::Running {
                    pid,
                    term_sent: false,
                };
                self.rx = Some(rx);
            }
            Err(e) => {
                self.status = Status::Failed;
                self.error = Some(e.to_string());
                self.rx = None;
            }
        }
    }

    pub fn running(&self) -> bool {
        matches!(self.status, Status::Running { .. })
    }

    /// Takes in whatever the threads sent. Returns true if anything changed.
    pub fn pump(&mut self) -> bool {
        let Some(rx) = &self.rx else { return false };
        let mut changed = false;
        loop {
            match rx.try_recv() {
                Ok(Msg::Output(bytes)) => {
                    self.output.push(&bytes);
                    changed = true;
                }
                Ok(Msg::Exited(result)) => {
                    match result {
                        Ok(status) => self.status = Status::Exited(status),
                        Err(e) => {
                            self.status = Status::Failed;
                            self.error = Some(e.to_string());
                        }
                    }
                    changed = true;
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    self.rx = None;
                    break;
                }
            }
        }
        changed
    }

    /// First call sends SIGTERM to the script's process group, the second
    /// SIGKILL.
    pub fn stop(&mut self) {
        if let Status::Running { pid, term_sent } = &mut self.status {
            let sig = if *term_sent {
                libc::SIGKILL
            } else {
                libc::SIGTERM
            };
            // SAFETY: signals the script's own process group.
            unsafe { libc::kill(-*pid, sig) };
            *term_sent = true;
        }
    }

    /// One line for the end of the screen: how the run ended.
    pub fn summary(&self) -> String {
        match self.status {
            Status::Running {
                term_sent: false, ..
            } => "running".into(),
            Status::Running {
                term_sent: true, ..
            } => "stopping".into(),
            Status::Exited(s) => match (s.code(), s.signal()) {
                (Some(0), _) => "done".into(),
                (Some(c), _) => format!("failed, exit {c}"),
                (None, Some(sig)) => format!("stopped by signal {sig}"),
                (None, None) => "ended".into(),
            },
            Status::Failed => format!(
                "could not run: {}",
                self.error.as_deref().unwrap_or("unknown error")
            ),
        }
    }

    pub fn succeeded(&self) -> bool {
        matches!(self.status, Status::Exited(s) if s.success())
    }
}

impl Drop for ScriptRun {
    /// The launcher is quitting or the screen is gone: do not leave the
    /// script running unseen.
    fn drop(&mut self) {
        if let Status::Running { pid, .. } = self.status {
            // SAFETY: signals the script's own process group.
            unsafe { libc::kill(-pid, libc::SIGTERM) };
        }
    }
}

fn spawn(exec: &str) -> io::Result<(libc::pid_t, Receiver<Msg>)> {
    let (mut reader, writer) = io::pipe()?;
    let mut child = Command::new("sh")
        .args(["-c", exec])
        .stdin(Stdio::null())
        .stdout(writer.try_clone()?)
        .stderr(writer)
        .env("TERM", "dumb")
        // Own group: stop() reaches the script's children too.
        .process_group(0)
        .spawn()?;
    // `writer` moved into the Command, which is dropped here, so the pipe
    // reaches EOF once the script and its children close it.
    let pid = libc::pid_t::try_from(child.id()).map_err(io::Error::other)?;

    let (tx, rx) = mpsc::channel();
    let out_tx = tx.clone();
    thread::spawn(move || {
        let mut buf = [0u8; 4096];
        loop {
            match reader.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    if out_tx.send(Msg::Output(buf[..n].to_vec())).is_err() {
                        break;
                    }
                }
            }
        }
    });
    thread::spawn(move || {
        let _ = tx.send(Msg::Exited(child.wait()));
    });
    Ok((pid, rx))
}

/// Script output as display lines: escapes stripped, tabs expanded, `\r`
/// rewriting the current line as a terminal would.
#[derive(Debug, Default)]
pub struct Output {
    lines: Vec<String>,
    /// Bytes of the line still being written.
    partial: Vec<u8>,
    /// Saw `\r`; a following `\n` ends the line, anything else rewrites it.
    cr: bool,
}

impl Output {
    pub fn push(&mut self, bytes: &[u8]) {
        for &b in bytes {
            if self.cr {
                self.cr = false;
                if b != b'\n' {
                    self.partial.clear();
                }
            }
            match b {
                b'\n' => {
                    let line = clean(&self.partial);
                    self.lines.push(line);
                    self.partial.clear();
                    if self.lines.len() > MAX_LINES {
                        let extra = self.lines.len() - MAX_LINES;
                        self.lines.drain(..extra);
                    }
                }
                b'\r' => self.cr = true,
                _ => self.partial.push(b),
            }
        }
    }

    /// All lines, including the one still being written.
    pub fn lines(&self) -> Vec<String> {
        let mut v = self.lines.clone();
        if !self.partial.is_empty() {
            v.push(clean(&self.partial));
        }
        v
    }

    pub fn len(&self) -> usize {
        self.lines.len() + usize::from(!self.partial.is_empty())
    }
}

/// Decodes one line: lossy UTF-8, ANSI escapes removed, tabs to 8-column
/// stops, other control characters dropped.
fn clean(bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(bytes);
    let mut out = String::with_capacity(text.len());
    let mut col = 0;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\x1b' => match chars.next() {
                // CSI: parameters, then a final byte in @..~.
                Some('[') => {
                    for c in chars.by_ref() {
                        if ('@'..='~').contains(&c) {
                            break;
                        }
                    }
                }
                // OSC: up to BEL or ESC \.
                Some(']') => {
                    while let Some(c) = chars.next() {
                        if c == '\x07' {
                            break;
                        }
                        if c == '\x1b' && chars.peek() == Some(&'\\') {
                            chars.next();
                            break;
                        }
                    }
                }
                // Two-character escapes.
                _ => {}
            },
            '\t' => {
                let n = 8 - col % 8;
                out.extend(std::iter::repeat_n(' ', n));
                col += n;
            }
            c if c.is_control() => {}
            c => {
                out.push(c);
                col += 1;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    fn lines(o: &Output) -> Vec<String> {
        o.lines()
    }

    #[test]
    fn splits_lines_across_chunks() {
        let mut o = Output::default();
        o.push(b"one\ntw");
        assert_eq!(lines(&o), ["one", "tw"]);
        o.push(b"o\nthree");
        assert_eq!(lines(&o), ["one", "two", "three"]);
        assert_eq!(o.len(), 3);
    }

    #[test]
    fn utf8_split_across_chunks() {
        let mut o = Output::default();
        let s = "héllo\n".as_bytes();
        o.push(&s[..2]);
        o.push(&s[2..]);
        assert_eq!(lines(&o), ["héllo"]);
    }

    #[test]
    fn carriage_return() {
        let mut o = Output::default();
        o.push(b"crlf\r\n10%\r50%\r100%\ndone\n");
        assert_eq!(lines(&o), ["crlf", "100%", "done"]);
    }

    #[test]
    fn strips_escapes_and_expands_tabs() {
        assert_eq!(clean(b"\x1b[1;31mred\x1b[0m plain"), "red plain");
        assert_eq!(clean(b"\x1b]0;title\x07after"), "after");
        assert_eq!(clean(b"\x1b]8;;http://x\x1b\\link"), "link");
        assert_eq!(clean(b"a\tb"), "a       b");
        assert_eq!(clean(b"bell\x07"), "bell");
    }

    #[test]
    fn keeps_a_bounded_number_of_lines() {
        let mut o = Output::default();
        for i in 0..MAX_LINES + 5 {
            o.push(format!("{i}\n").as_bytes());
        }
        assert_eq!(o.len(), MAX_LINES);
        assert_eq!(o.lines()[0], "5");
    }

    /// Pumps until the script ends, or panics after 5 s.
    fn finish(run: &mut ScriptRun) {
        let start = Instant::now();
        while run.running() {
            run.pump();
            assert!(start.elapsed() < Duration::from_secs(5), "script hung");
            thread::sleep(Duration::from_millis(5));
        }
        // Output sent before the exit message may still be queued.
        thread::sleep(Duration::from_millis(20));
        run.pump();
    }

    #[test]
    fn runs_and_captures_both_streams_in_order() {
        let mut run = ScriptRun::start("t", "T", "echo out; echo err >&2; echo $TERM");
        finish(&mut run);
        assert_eq!(run.output.lines(), ["out", "err", "dumb"]);
        assert!(run.succeeded());
        assert_eq!(run.summary(), "done");
    }

    #[test]
    fn reports_failure() {
        let mut run = ScriptRun::start("t", "T", "exit 3");
        finish(&mut run);
        assert!(!run.succeeded());
        assert_eq!(run.summary(), "failed, exit 3");
    }

    #[test]
    fn stop_terminates_the_group() {
        let mut run = ScriptRun::start("t", "T", "sleep 30 & wait");
        thread::sleep(Duration::from_millis(100));
        run.stop();
        assert_eq!(run.summary(), "stopping");
        finish(&mut run);
        assert_eq!(run.summary(), "stopped by signal 15");
    }

    #[test]
    fn second_stop_kills() {
        let mut run = ScriptRun::start("t", "T", "trap '' TERM; while :; do sleep 0.05; done");
        thread::sleep(Duration::from_millis(100));
        run.stop();
        thread::sleep(Duration::from_millis(100));
        assert!(run.running(), "TERM is trapped");
        run.stop();
        finish(&mut run);
        assert_eq!(run.summary(), "stopped by signal 9");
    }

    #[test]
    fn exit_is_seen_while_a_child_holds_the_pipe() {
        // The background sleep keeps the pipe open after sh exits.
        let mut run = ScriptRun::start("t", "T", "echo hi; sleep 30 &");
        let Status::Running { pid, .. } = run.status else {
            panic!("did not start")
        };
        finish(&mut run);
        assert_eq!(run.summary(), "done");
        assert_eq!(run.output.lines(), ["hi"]);
        // Clean up the background sleep, still in the script's group.
        // SAFETY: signals the test script's own process group.
        unsafe { libc::kill(-pid, libc::SIGKILL) };
    }

    #[test]
    fn restart_clears_and_reruns() {
        let mut run = ScriptRun::start("t", "T", "echo again");
        finish(&mut run);
        run.from_bottom = 3;
        run.restart();
        finish(&mut run);
        assert_eq!(run.output.lines(), ["again"]);
        assert_eq!(run.from_bottom, 0);
    }
}
