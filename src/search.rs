//! Fuzzy search over every item in the tree.

use nucleo_matcher::pattern::{AtomKind, CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher, Utf32Str};

use crate::history::History;
use crate::items::Tree;

/// One search result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hit {
    pub id: String,
    /// Positions of matched characters in the label, in order, for
    /// highlighting.
    pub indices: Vec<u32>,
}

pub struct Search {
    /// Allocates ~135 KB, so it is made once and reused.
    matcher: Matcher,
}

impl Search {
    pub fn new() -> Self {
        Self {
            matcher: Matcher::new(Config::DEFAULT),
        }
    }

    /// Matches `query` against every item's label. Each space-separated word
    /// must match. No special syntax: `!`, `^`, `$` and `'` are plain
    /// characters. Ranked by score, then most recently used, then label.
    pub fn run(&mut self, tree: &Tree, history: &History, query: &str) -> Vec<Hit> {
        let pattern = Pattern::new(
            query,
            CaseMatching::Ignore,
            Normalization::Smart,
            AtomKind::Fuzzy,
        );
        let recency: Vec<&str> = history.recent().collect();
        let rank = |id: &str| recency.iter().position(|r| *r == id).unwrap_or(usize::MAX);

        let mut buf = Vec::new();
        let mut hits: Vec<(u32, usize, String, Hit)> = tree
            .all()
            .filter_map(|item| {
                let mut indices = Vec::new();
                let score = pattern.indices(
                    Utf32Str::new(&item.label, &mut buf),
                    &mut self.matcher,
                    &mut indices,
                )?;
                // Words are matched one at a time, so indices can repeat or
                // come out of order.
                indices.sort_unstable();
                indices.dedup();
                Some((
                    score,
                    rank(&item.id),
                    item.label.to_lowercase(),
                    Hit {
                        id: item.id.clone(),
                        indices,
                    },
                ))
            })
            .collect();
        hits.sort_by(|a, b| {
            b.0.cmp(&a.0)
                .then(a.1.cmp(&b.1))
                .then_with(|| a.2.cmp(&b.2))
                .then_with(|| a.3.id.cmp(&b.3.id))
        });
        hits.into_iter().map(|h| h.3).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Layer, SHIPPED, merge_layers};

    fn tree() -> Tree {
        let (merged, warnings) = merge_layers(&[Layer {
            name: "shipped".into(),
            text: SHIPPED.into(),
        }]);
        Tree::build(merged, warnings)
    }

    fn ids(hits: &[Hit]) -> Vec<&str> {
        hits.iter().map(|h| h.id.as_str()).collect()
    }

    #[test]
    fn finds_items_in_every_folder() {
        let t = tree();
        let h = History::in_memory();
        let mut s = Search::new();
        assert_eq!(ids(&s.run(&t, &h, "reboot")), ["reboot"]);
        assert_eq!(ids(&s.run(&t, &h, "wifi"))[0], "wifi");
        // Folders are results too.
        assert_eq!(ids(&s.run(&t, &h, "settings"))[0], "settings");
    }

    #[test]
    fn fuzzy_and_case_insensitive() {
        let t = tree();
        let h = History::in_memory();
        let mut s = Search::new();
        let hits = s.run(&t, &h, "RPI");
        assert_eq!(hits[0].id, "raspi_config");
        // "Raspberry Pi config": R, then p and i of "Pi".
        assert_eq!(hits[0].indices, [0, 10, 11]);
    }

    #[test]
    fn every_word_must_match() {
        let t = tree();
        let h = History::in_memory();
        let mut s = Search::new();
        assert_eq!(ids(&s.run(&t, &h, "shut down")), ["shutdown"]);
        assert!(s.run(&t, &h, "shut zzz").is_empty());
    }

    #[test]
    fn recent_breaks_ties() {
        let t = tree();
        let mut h = History::in_memory();
        let mut s = Search::new();
        // "b" scores the same as the first letter of Bluetooth and
        // Brightness. Alphabetical first, until Brightness is used.
        let first = |s: &mut Search, h: &History| s.run(&t, h, "b")[0].id.clone();
        assert_eq!(first(&mut s, &h), "bluetooth");
        h.record("brightness");
        assert_eq!(first(&mut s, &h), "brightness");
    }

    #[test]
    fn no_special_syntax() {
        let t = tree();
        let h = History::in_memory();
        let mut s = Search::new();
        // fzf syntax would read this as "does not contain x".
        assert!(s.run(&t, &h, "!x").is_empty());
    }
}
