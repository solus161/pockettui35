//! Button layers through keyd, for apps with `raw_buttons = true`.
//!
//! Normally keyd maps the face buttons (F13-F18) and Select (F21) to
//! Enter, Escape, Tab and so on. A game wants them unmapped, so the
//! launcher binds them back to themselves around the launch, and resets
//! keyd after. Start (F22) is never touched: holding it must always close
//! the app.
//!
//! Access to keyd's socket is as good as root, so the user does not get it.
//! The launcher runs itself as `sudo -n pockettui35 keys raw|normal`, which
//! the sudoers rule allows as two exact command lines.

use std::path::PathBuf;
use std::process::{Command, ExitCode};

pub const SUBCOMMAND: &str = "keys";

/// The keys a game gets raw: A B X Y L R, then Select.
const RAW_KEYS: [&str; 7] = ["f13", "f14", "f15", "f16", "f17", "f18", "f21"];

/// The `keyd bind` arguments for a mode.
fn bind_args(raw: bool) -> Vec<String> {
    let mut args = vec!["bind".to_string()];
    if raw {
        args.extend(RAW_KEYS.iter().map(|k| format!("main.{k} = {k}")));
    } else {
        args.push("reset".into());
    }
    args
}

/// Switches the buttons, through sudo unless already root. While raw, a
/// marker file says so, so a launcher that crashed can reset at start-up.
pub fn set(raw: bool) -> Result<(), String> {
    let mode = if raw { "raw" } else { "normal" };
    if raw && let Some(m) = marker() {
        let _ = m
            .parent()
            .map_or(Ok(()), std::fs::create_dir_all)
            .and_then(|()| std::fs::write(&m, ""));
    }
    // SAFETY: geteuid has no preconditions.
    let result = if unsafe { libc::geteuid() } == 0 {
        run_keyd(raw)
    } else {
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        match Command::new("sudo")
            .arg("-n")
            .arg(exe)
            .args([SUBCOMMAND, mode])
            .output()
        {
            Ok(out) if out.status.success() => Ok(()),
            Ok(out) => {
                let err = String::from_utf8_lossy(&out.stderr);
                Err(err.lines().next().unwrap_or("failed").trim().to_string())
            }
            Err(e) => Err(format!("sudo: {e}")),
        }
    };
    if !raw
        && result.is_ok()
        && let Some(m) = marker()
    {
        let _ = std::fs::remove_file(m);
    }
    result
}

/// After a crash with raw buttons on, puts them back. Does nothing, and
/// costs nothing, in the normal case.
pub fn reset_if_left_raw() -> Result<(), String> {
    if marker().is_some_and(|m| m.exists()) {
        set(false)
    } else {
        Ok(())
    }
}

fn run_keyd(raw: bool) -> Result<(), String> {
    let out = Command::new("keyd")
        .args(bind_args(raw))
        .output()
        .map_err(|e| format!("keyd: {e}"))?;
    if out.status.success() {
        Ok(())
    } else {
        let err = String::from_utf8_lossy(&out.stderr);
        Err(format!(
            "keyd: {}",
            err.lines().next().unwrap_or("failed").trim()
        ))
    }
}

fn marker() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_STATE_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/state")))?;
    Some(base.join("pockettui35").join("keys-raw"))
}

/// `pockettui35 keys raw|normal`: the root side, run by sudo.
pub fn subcommand(args: &[String]) -> ExitCode {
    let raw = match args.first().map(String::as_str) {
        Some("raw") => true,
        Some("normal") => false,
        _ => {
            eprintln!("usage: pockettui35 {SUBCOMMAND} <raw|normal>");
            return ExitCode::from(2);
        }
    };
    match run_keyd(raw) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{SUBCOMMAND}: {e}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raw_binds_face_buttons_and_select_but_never_start() {
        let args = bind_args(true);
        assert_eq!(args[0], "bind");
        assert_eq!(args[1], "main.f13 = f13");
        assert_eq!(args.len(), 1 + RAW_KEYS.len());
        assert!(args.iter().any(|a| a == "main.f21 = f21"));
        assert!(
            !args.iter().any(|a| a.contains("f22")),
            "Start stays the way out"
        );
    }

    #[test]
    fn normal_resets() {
        assert_eq!(bind_args(false), ["bind", "reset"]);
    }
}
