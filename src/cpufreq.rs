//! Power profiles: CPU governor and frequency cap through cpufreq sysfs.
//!
//! | Profile        | Governor                  | Max frequency          |
//! |----------------|---------------------------|------------------------|
//! | performance    | `performance`             | hardware max           |
//! | balanced       | `schedutil` (`ondemand`)  | hardware max           |
//! | battery saving | `schedutil` (`ondemand`)  | 60% of max, snapped    |
//!
//! Reading is open to anyone. Writing needs root: the launcher runs itself
//! as `sudo -n pockettui35 set-power-profile <name>`, which a shipped
//! sudoers rule allows without a password. The cap is a starting point, to
//! measure on the device.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

const CPUFREQ: &str = "/sys/devices/system/cpu/cpufreq";

/// Battery saving caps the CPU at this share of its maximum.
const BATTERY_CAP_PERCENT: u64 = 60;

/// The subcommand the sudoers rule allows.
pub const SUBCOMMAND: &str = "set-power-profile";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Profile {
    BatterySaving,
    Balanced,
    Performance,
}

impl Profile {
    /// Slowest to fastest: Left steps down this list, Right up.
    pub const ALL: [Profile; 3] = [Self::BatterySaving, Self::Balanced, Self::Performance];

    /// For the command line and the state file.
    pub fn name(self) -> &'static str {
        match self {
            Self::BatterySaving => "battery_saving",
            Self::Balanced => "balanced",
            Self::Performance => "performance",
        }
    }

    /// For the screen.
    pub fn label(self) -> &'static str {
        match self {
            Self::BatterySaving => "battery saving",
            Self::Balanced => "balanced",
            Self::Performance => "performance",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|p| p.name() == name.trim())
    }

    /// The next profile towards faster (`delta > 0`) or slower, stopping at
    /// the ends.
    pub fn step(self, delta: i8) -> Self {
        let i = Self::ALL.iter().position(|p| *p == self).unwrap_or(1);
        let j = i
            .saturating_add_signed(isize::from(delta))
            .min(Self::ALL.len() - 1);
        Self::ALL[j]
    }
}

/// One cpufreq policy, as sysfs shows it. Frequencies in kHz.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Policy {
    pub governor: String,
    pub governors: Vec<String>,
    pub max: u64,
    pub cpuinfo_min: u64,
    pub cpuinfo_max: u64,
    /// Empty when the driver does not list them.
    pub available: Vec<u64>,
}

impl Policy {
    fn read(dir: &Path) -> io::Result<Self> {
        let text = |name: &str| fs::read_to_string(dir.join(name)).map(|s| s.trim().to_string());
        let num = |name: &str| -> io::Result<u64> {
            text(name)?
                .parse()
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, format!("{name}: {e}")))
        };
        let list = |name: &str| text(name).unwrap_or_default();
        Ok(Self {
            governor: text("scaling_governor")?,
            governors: list("scaling_available_governors")
                .split_whitespace()
                .map(String::from)
                .collect(),
            max: num("scaling_max_freq")?,
            cpuinfo_min: num("cpuinfo_min_freq")?,
            cpuinfo_max: num("cpuinfo_max_freq")?,
            available: list("scaling_available_frequencies")
                .split_whitespace()
                .filter_map(|s| s.parse().ok())
                .collect(),
        })
    }

    /// The dynamic governor: `schedutil` when the kernel has it.
    fn dynamic_governor(&self) -> &str {
        if self.governors.is_empty() || self.governors.iter().any(|g| g == "schedutil") {
            "schedutil"
        } else {
            "ondemand"
        }
    }

    /// Battery saving's cap: a share of the maximum, rounded down to a
    /// frequency the driver offers, never under the minimum.
    pub fn battery_cap(&self) -> u64 {
        let target = self.cpuinfo_max * BATTERY_CAP_PERCENT / 100;
        let snapped = self
            .available
            .iter()
            .copied()
            .filter(|f| *f <= target)
            .max()
            .unwrap_or(target);
        snapped.max(self.cpuinfo_min)
    }

    /// The governor and max frequency a profile sets.
    pub fn targets(&self, profile: Profile) -> (&str, u64) {
        match profile {
            Profile::Performance => ("performance", self.cpuinfo_max),
            Profile::Balanced => (self.dynamic_governor(), self.cpuinfo_max),
            Profile::BatterySaving => (self.dynamic_governor(), self.battery_cap()),
        }
    }

    /// Which profile this state is, if any. Anything else is "custom".
    pub fn classify(&self) -> Option<Profile> {
        Profile::ALL.into_iter().find(|p| {
            let (governor, max) = self.targets(*p);
            self.governor == governor && self.max == max
        })
    }
}

fn policies() -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = fs::read_dir(CPUFREQ)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .is_some_and(|n| n.to_string_lossy().starts_with("policy"))
        })
        .collect();
    v.sort();
    v
}

/// The profile the CPU is in now, read from the first policy. `None` when
/// there is no cpufreq, or the state matches no profile.
pub fn current() -> Option<Profile> {
    let first = policies().into_iter().next()?;
    Policy::read(&first).ok()?.classify()
}

/// Writes a profile to every policy. Needs root.
fn apply(profile: Profile) -> io::Result<()> {
    let dirs = policies();
    if dirs.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "no cpufreq policies",
        ));
    }
    for dir in dirs {
        let policy = Policy::read(&dir)?;
        let (governor, max) = policy.targets(profile);
        fs::write(dir.join("scaling_max_freq"), max.to_string())?;
        fs::write(dir.join("scaling_governor"), governor)?;
    }
    Ok(())
}

/// Applies a profile, through sudo unless already root.
pub fn set(profile: Profile) -> Result<(), String> {
    // SAFETY: geteuid has no preconditions.
    if unsafe { libc::geteuid() } == 0 {
        return apply(profile).map_err(|e| e.to_string());
    }
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let out = Command::new("sudo")
        .arg("-n")
        .arg(exe)
        .args([SUBCOMMAND, profile.name()])
        .output()
        .map_err(|e| format!("sudo: {e}"))?;
    if out.status.success() {
        Ok(())
    } else {
        let err = String::from_utf8_lossy(&out.stderr);
        let first = err.lines().next().unwrap_or("failed").trim();
        Err(format!("needs the sudoers rule ({first})"))
    }
}

/// `pockettui35 set-power-profile <name>`: the root side, run by sudo.
pub fn subcommand(args: &[String]) -> ExitCode {
    let Some(profile) = args.first().and_then(|a| Profile::from_name(a)) else {
        eprintln!(
            "usage: pockettui35 {SUBCOMMAND} <{}>",
            Profile::ALL.map(Profile::name).join("|")
        );
        return ExitCode::from(2);
    };
    match apply(profile) {
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

    /// A Pi 4 as sysfs shows it.
    fn pi4() -> Policy {
        Policy {
            governor: "schedutil".into(),
            governors: [
                "conservative",
                "ondemand",
                "userspace",
                "powersave",
                "performance",
                "schedutil",
            ]
            .map(String::from)
            .to_vec(),
            max: 1_800_000,
            cpuinfo_min: 600_000,
            cpuinfo_max: 1_800_000,
            available: (600_000..=1_800_000).step_by(100_000).collect(),
        }
    }

    #[test]
    fn battery_cap_snaps_down_to_an_offered_frequency() {
        // 60% of 1.8 GHz is 1.08 GHz; the nearest offered below is 1.0 GHz.
        assert_eq!(pi4().battery_cap(), 1_000_000);
        // No list: the exact share.
        let p = Policy {
            available: vec![],
            ..pi4()
        };
        assert_eq!(p.battery_cap(), 1_080_000);
        // Never under the minimum.
        let p = Policy {
            cpuinfo_min: 1_500_000,
            ..pi4()
        };
        assert_eq!(p.battery_cap(), 1_500_000);
    }

    #[test]
    fn classifies_each_profile() {
        let mut p = pi4();
        assert_eq!(p.classify(), Some(Profile::Balanced));
        p.governor = "performance".into();
        assert_eq!(p.classify(), Some(Profile::Performance));
        p.governor = "schedutil".into();
        p.max = 1_000_000;
        assert_eq!(p.classify(), Some(Profile::BatterySaving));
        p.max = 1_200_000;
        assert_eq!(p.classify(), None, "custom");
    }

    #[test]
    fn falls_back_to_ondemand_without_schedutil() {
        let p = Policy {
            governors: vec!["ondemand".into(), "performance".into()],
            governor: "ondemand".into(),
            ..pi4()
        };
        assert_eq!(p.targets(Profile::Balanced), ("ondemand", 1_800_000));
        assert_eq!(p.classify(), Some(Profile::Balanced));
    }

    #[test]
    fn steps_stop_at_the_ends() {
        assert_eq!(Profile::Balanced.step(1), Profile::Performance);
        assert_eq!(Profile::Performance.step(1), Profile::Performance);
        assert_eq!(Profile::Balanced.step(-1), Profile::BatterySaving);
        assert_eq!(Profile::BatterySaving.step(-1), Profile::BatterySaving);
    }

    #[test]
    fn names_round_trip() {
        for p in Profile::ALL {
            assert_eq!(Profile::from_name(p.name()), Some(p));
        }
        assert_eq!(Profile::from_name("turbo"), None);
    }
}
