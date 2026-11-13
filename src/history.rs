//! Last-used times, for the Recent group on Home.
//!
//! Stored in `$XDG_STATE_HOME/pockettui35/history.toml` (default
//! `~/.local/state/pockettui35/history.toml`) as `id = <unix seconds>`.
//! History is a convenience: a missing, broken or unwritable file means an
//! empty Recent group, never an error.

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

/// Entries kept on disk. Far more than Recent shows, so items that are
/// hidden or removed later do not empty the group.
const KEEP: usize = 50;

#[derive(Debug, Default)]
pub struct History {
    /// `None` keeps history in memory only (tests).
    path: Option<PathBuf>,
    used: HashMap<String, u64>,
}

impl History {
    /// Loads from the default path.
    pub fn load() -> Self {
        let Some(path) = default_path() else {
            return Self::default();
        };
        let used = std::fs::read_to_string(&path)
            .ok()
            .and_then(|text| text.parse::<toml::Table>().ok())
            .map(|table| {
                table
                    .into_iter()
                    .filter_map(|(id, v)| Some((id, u64::try_from(v.as_integer()?).ok()?)))
                    .collect()
            })
            .unwrap_or_default();
        Self {
            path: Some(path),
            used,
        }
    }

    /// History that never touches the disk.
    #[cfg(test)]
    pub fn in_memory() -> Self {
        Self::default()
    }

    /// Ids, most recently used first.
    pub fn recent(&self) -> impl Iterator<Item = &str> {
        let mut v: Vec<(&String, &u64)> = self.used.iter().collect();
        // Ties (same second) break by id so the order is stable.
        v.sort_by(|a, b| b.1.cmp(a.1).then_with(|| a.0.cmp(b.0)));
        v.into_iter().map(|(id, _)| id.as_str())
    }

    /// Marks an item as used now and saves.
    pub fn record(&mut self, id: &str) {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        // Never go backwards: a clock step back must not demote the newest.
        let newest = self.used.values().copied().max().unwrap_or(0);
        self.used.insert(id.to_string(), now.max(newest + 1));

        if self.used.len() > KEEP {
            let keep: Vec<String> = self.recent().take(KEEP).map(String::from).collect();
            self.used.retain(|id, _| keep.contains(id));
        }
        self.save();
    }

    /// Best effort. Written to a temp file and renamed, so a crash
    /// mid-write cannot leave a truncated file.
    fn save(&self) {
        let Some(path) = &self.path else { return };
        let table: toml::Table = self
            .used
            .iter()
            .map(|(id, t)| {
                (
                    id.clone(),
                    toml::Value::Integer(i64::try_from(*t).unwrap_or(i64::MAX)),
                )
            })
            .collect();
        let tmp = path.with_extension("toml.tmp");
        let _ = path
            .parent()
            .map_or(Ok(()), std::fs::create_dir_all)
            .and_then(|()| std::fs::write(&tmp, table.to_string()))
            .and_then(|()| std::fs::rename(&tmp, path));
    }
}

fn default_path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_STATE_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/state")))?;
    Some(base.join("pockettui35").join("history.toml"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn newest_first_even_within_one_second() {
        let mut h = History::in_memory();
        h.record("a");
        h.record("b");
        h.record("c");
        h.record("a");
        assert_eq!(h.recent().collect::<Vec<_>>(), ["a", "c", "b"]);
    }

    #[test]
    fn keeps_a_bounded_number() {
        let mut h = History::in_memory();
        for i in 0..KEEP + 10 {
            h.record(&format!("i{i}"));
        }
        assert_eq!(h.used.len(), KEEP);
        assert_eq!(h.recent().next(), Some(format!("i{}", KEEP + 9).as_str()));
    }

    #[test]
    fn round_trips_through_disk() {
        let dir = std::env::temp_dir().join(format!("pt35-history-{}", std::process::id()));
        let path = dir.join("history.toml");
        let mut h = History {
            path: Some(path.clone()),
            used: HashMap::new(),
        };
        h.record("x");
        h.record("y");
        let text = std::fs::read_to_string(&path).unwrap();
        let table: toml::Table = text.parse().unwrap();
        assert_eq!(table.len(), 2);
        assert!(table["y"].as_integer() > table["x"].as_integer());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
