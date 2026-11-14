//! Everything the launcher reads from or does to the system, behind one
//! trait so the app logic can be tested without hardware.

use std::path::PathBuf;
use std::process::Command;

use crate::backlight::{self, Answer};
use crate::cpufreq::{self, Profile};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Volume {
    pub percent: u8,
    pub muted: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PowerAction {
    Reboot,
    Shutdown,
}

pub trait System {
    fn volume(&mut self) -> Option<Volume>;
    fn set_volume(&mut self, percent: u8) -> Option<Volume>;
    /// `None`: no patched firmware answering.
    fn brightness(&mut self) -> Option<u8>;
    fn set_brightness(&mut self, percent: u8) -> Option<u8>;
    /// `None`: no cpufreq, or a state that is no profile.
    fn power_profile(&mut self) -> Option<Profile>;
    fn set_power_profile(&mut self, profile: Profile) -> Result<(), String>;
    /// Short connection text for the status bar; `None` if unknown.
    fn network(&mut self) -> Option<String>;
    fn power(&mut self, action: PowerAction) -> Result<(), String>;
    /// Blanks (`true`) or unblanks the console.
    fn blank(&mut self, on: bool);
    /// Forget cached failures, e.g. a keyboard that did not answer.
    fn rescan(&mut self) {}
}

/// The real system.
#[derive(Debug, Default)]
pub struct Host {
    /// The keyboard gave no answer: do not ask again until `rescan`, or
    /// every key press would wait 400 ms.
    backlight_silent: bool,
    /// The last level, kept while `pt35-kbd` holds the port.
    backlight_level: Option<u8>,
}

/// Puts back the power profile saved last time, if the CPU is not in it.
/// cpufreq settings do not survive a reboot.
pub fn restore_power_profile() -> Result<(), String> {
    let Some(saved) = state_path("power-profile")
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| Profile::from_name(&s))
    else {
        return Ok(());
    };
    if cpufreq::current() == Some(saved) {
        return Ok(());
    }
    cpufreq::set(saved)
}

impl Host {
    fn backlight_answer(&mut self, answer: Answer) -> Option<u8> {
        match answer {
            Answer::Level(l) => {
                self.backlight_level = Some(l);
                Some(l)
            }
            Answer::Busy => self.backlight_level,
            Answer::Silent => {
                self.backlight_silent = true;
                self.backlight_level = None;
                None
            }
        }
    }
}

impl System for Host {
    fn volume(&mut self) -> Option<Volume> {
        let out = Command::new("wpctl")
            .args(["get-volume", "@DEFAULT_AUDIO_SINK@"])
            .output()
            .ok()?;
        out.status
            .success()
            .then(|| parse_wpctl(&String::from_utf8_lossy(&out.stdout)))
            .flatten()
    }

    fn set_volume(&mut self, percent: u8) -> Option<Volume> {
        let ok = Command::new("wpctl")
            .args(["set-volume", "-l", "1.0", "@DEFAULT_AUDIO_SINK@"])
            .arg(format!("{}%", percent.min(100)))
            .status()
            .is_ok_and(|s| s.success());
        if ok { self.volume() } else { None }
    }

    fn brightness(&mut self) -> Option<u8> {
        if self.backlight_silent {
            return None;
        }
        let answer = backlight::read();
        self.backlight_answer(answer)
    }

    fn set_brightness(&mut self, percent: u8) -> Option<u8> {
        if self.backlight_silent {
            return None;
        }
        let answer = backlight::set(percent);
        self.backlight_answer(answer)
    }

    fn power_profile(&mut self) -> Option<Profile> {
        cpufreq::current()
    }

    fn set_power_profile(&mut self, profile: Profile) -> Result<(), String> {
        cpufreq::set(profile)?;
        if let Some(path) = state_path("power-profile") {
            let _ = path
                .parent()
                .map_or(Ok(()), std::fs::create_dir_all)
                .and_then(|()| std::fs::write(&path, profile.name()));
        }
        Ok(())
    }

    fn network(&mut self) -> Option<String> {
        let out = Command::new("nmcli")
            .args(["-t", "-f", "TYPE,STATE,CONNECTION", "device"])
            .output()
            .ok()?;
        out.status
            .success()
            .then(|| parse_nmcli(&String::from_utf8_lossy(&out.stdout)))
    }

    fn power(&mut self, action: PowerAction) -> Result<(), String> {
        // logind lets the active local session do this without a password.
        let verb = match action {
            PowerAction::Reboot => "reboot",
            PowerAction::Shutdown => "poweroff",
        };
        let out = Command::new("systemctl")
            .arg(verb)
            .output()
            .map_err(|e| format!("systemctl: {e}"))?;
        if out.status.success() {
            Ok(())
        } else {
            let err = String::from_utf8_lossy(&out.stderr);
            Err(err.lines().next().unwrap_or("failed").trim().to_string())
        }
    }

    fn blank(&mut self, on: bool) {
        // TIOCLINUX subcodes from linux/tiocl.h. BLANKSCREEN blanks until
        // explicitly unblanked: a key does not wake it, the launcher does.
        const TIOCL_UNBLANKSCREEN: u8 = 4;
        const TIOCL_BLANKSCREEN: u8 = 14;
        let mut arg = [if on {
            TIOCL_BLANKSCREEN
        } else {
            TIOCL_UNBLANKSCREEN
        }];
        // Fails harmlessly off the Linux console (a terminal emulator).
        // SAFETY: TIOCLINUX reads one subcode byte from a valid buffer.
        unsafe { libc::ioctl(libc::STDIN_FILENO, libc::TIOCLINUX, arg.as_mut_ptr()) };
    }

    fn rescan(&mut self) {
        self.backlight_silent = false;
    }
}

fn state_path(name: &str) -> Option<PathBuf> {
    let base = std::env::var_os("XDG_STATE_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/state")))?;
    Some(base.join("pockettui35").join(name))
}

/// `Volume: 0.40` or `Volume: 0.40 [MUTED]`.
fn parse_wpctl(text: &str) -> Option<Volume> {
    let rest = text.trim().strip_prefix("Volume:")?.trim();
    let mut words = rest.split_whitespace();
    let level: f64 = words.next()?.parse().ok()?;
    let percent = (level * 100.0).round().clamp(0.0, 100.0);
    Some(Volume {
        // In 0..=100 after the clamp.
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        percent: percent as u8,
        muted: words.any(|w| w == "[MUTED]"),
    })
}

/// `nmcli -t -f TYPE,STATE,CONNECTION device` to one short text. A wired
/// link carries the traffic when both are up, so it wins.
fn parse_nmcli(text: &str) -> String {
    let mut wifi = None;
    let mut wired = false;
    for line in text.lines() {
        let fields = split_terse(line);
        let [ty, state, conn] = fields.as_slice() else {
            continue;
        };
        if !state.starts_with("connected") {
            continue;
        }
        match ty.as_str() {
            "ethernet" => wired = true,
            "wifi" if wifi.is_none() => wifi = Some(conn.clone()),
            _ => {}
        }
    }
    match (wired, wifi) {
        (true, _) => "wired".into(),
        (false, Some(ssid)) => format!("wifi {ssid}"),
        (false, None) => "offline".into(),
    }
}

/// Splits nmcli's terse format: `:` separates, `\:` and `\\` are escapes.
fn split_terse(line: &str) -> Vec<String> {
    let mut fields = vec![String::new()];
    let mut chars = line.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => {
                if let Some(n) = chars.next() {
                    fields.last_mut().unwrap().push(n);
                }
            }
            ':' => fields.push(String::new()),
            c => fields.last_mut().unwrap().push(c),
        }
    }
    fields
}

/// A system for tests: state in memory, calls logged.
#[cfg(test)]
pub mod fake {
    use std::cell::RefCell;
    use std::rc::Rc;

    use super::*;

    #[derive(Debug, Default)]
    pub struct State {
        pub volume: Option<Volume>,
        pub brightness: Option<u8>,
        pub profile: Option<Profile>,
        pub profile_error: Option<String>,
        pub net: Option<String>,
        pub blanked: bool,
        pub calls: Vec<String>,
    }

    /// Cloning shares the state, so a test keeps a handle after giving the
    /// app its own.
    #[derive(Debug, Clone, Default)]
    pub struct Fake(pub Rc<RefCell<State>>);

    impl Fake {
        pub fn working() -> Self {
            Self(Rc::new(RefCell::new(State {
                volume: Some(Volume {
                    percent: 40,
                    muted: false,
                }),
                brightness: Some(80),
                profile: Some(Profile::Balanced),
                net: Some("wifi Home".into()),
                ..State::default()
            })))
        }
    }

    impl System for Fake {
        fn volume(&mut self) -> Option<Volume> {
            self.0.borrow().volume
        }
        fn set_volume(&mut self, percent: u8) -> Option<Volume> {
            let mut s = self.0.borrow_mut();
            s.calls.push(format!("volume {percent}"));
            if let Some(v) = &mut s.volume {
                v.percent = percent;
            }
            s.volume
        }
        fn brightness(&mut self) -> Option<u8> {
            self.0.borrow().brightness
        }
        fn set_brightness(&mut self, percent: u8) -> Option<u8> {
            let mut s = self.0.borrow_mut();
            s.calls.push(format!("brightness {percent}"));
            if s.brightness.is_some() {
                // The firmware's 5% floor.
                s.brightness = Some(percent.max(5));
            }
            s.brightness
        }
        fn power_profile(&mut self) -> Option<Profile> {
            self.0.borrow().profile
        }
        fn set_power_profile(&mut self, profile: Profile) -> Result<(), String> {
            let mut s = self.0.borrow_mut();
            s.calls.push(format!("profile {}", profile.name()));
            if let Some(e) = &s.profile_error {
                return Err(e.clone());
            }
            s.profile = Some(profile);
            Ok(())
        }
        fn network(&mut self) -> Option<String> {
            self.0.borrow().net.clone()
        }
        fn power(&mut self, action: PowerAction) -> Result<(), String> {
            self.0.borrow_mut().calls.push(format!("power {action:?}"));
            Ok(())
        }
        fn blank(&mut self, on: bool) {
            let mut s = self.0.borrow_mut();
            s.blanked = on;
            s.calls.push(format!("blank {on}"));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wpctl() {
        assert_eq!(
            parse_wpctl("Volume: 0.40\n"),
            Some(Volume {
                percent: 40,
                muted: false
            })
        );
        assert_eq!(
            parse_wpctl("Volume: 0.35 [MUTED]\n"),
            Some(Volume {
                percent: 35,
                muted: true
            })
        );
        assert_eq!(parse_wpctl("Volume: 1.50").unwrap().percent, 100);
        assert_eq!(parse_wpctl("nope"), None);
    }

    #[test]
    fn nmcli() {
        let wifi = "wifi:connected:Home\\:Net\nethernet:unavailable:\nloopback:connected (externally):lo\n";
        assert_eq!(parse_nmcli(wifi), "wifi Home:Net");
        let both = "wifi:connected:Home\nethernet:connected:Wired connection 1\n";
        assert_eq!(parse_nmcli(both), "wired");
        let none = "wifi:disconnected:\nethernet:unavailable:\n";
        assert_eq!(parse_nmcli(none), "offline");
        assert_eq!(parse_nmcli(""), "offline");
    }

    #[test]
    fn terse_escapes() {
        assert_eq!(split_terse("a\\:b:c\\\\d:"), ["a:b", "c\\d", ""]);
    }
}
