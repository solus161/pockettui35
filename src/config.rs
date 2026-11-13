//! Loading and merging config layers.
//!
//! Layers, lowest first:
//! 1. shipped: `/usr/share/pockettui35/config.toml`, or the copy built into
//!    the binary when that file is absent (dev builds);
//! 2. user: `$XDG_CONFIG_HOME/pockettui35/config.toml`
//!    (default `~/.config/pockettui35/config.toml`).
//!
//! `[item.<id>]` tables merge field by field. If a higher layer changes an
//! item's `type`, it replaces the item instead, so fields of the old type do
//! not leak in.

use std::collections::BTreeMap;
use std::io::ErrorKind;
use std::path::PathBuf;

use crate::items::Tree;

/// The shipped item file, built in as a fallback.
pub const SHIPPED: &str = include_str!("../assets/config.toml");

const SHIPPED_PATH: &str = "/usr/share/pockettui35/config.toml";

/// One config source: a name for messages, and its text.
pub struct Layer {
    pub name: String,
    pub text: String,
}

/// Reads all layers from disk and builds the menu tree. Never fails: every
/// problem becomes a warning in the tree.
pub fn load() -> Tree {
    let mut warnings = Vec::new();
    let mut layers = Vec::new();

    match std::fs::read_to_string(SHIPPED_PATH) {
        Ok(text) => layers.push(Layer {
            name: SHIPPED_PATH.into(),
            text,
        }),
        Err(e) => {
            if e.kind() != ErrorKind::NotFound {
                warnings.push(format!("{SHIPPED_PATH}: {e}, using built-in defaults"));
            }
            layers.push(Layer {
                name: "built-in defaults".into(),
                text: SHIPPED.into(),
            });
        }
    }

    if let Some(path) = user_path() {
        match std::fs::read_to_string(&path) {
            Ok(text) => layers.push(Layer {
                name: path.display().to_string(),
                text,
            }),
            Err(e) if e.kind() == ErrorKind::NotFound => {}
            Err(e) => warnings.push(format!("{}: {e}", path.display())),
        }
    }

    let (merged, merge_warnings) = merge_layers(&layers);
    warnings.extend(merge_warnings);
    Tree::build(merged, warnings)
}

fn user_path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(base.join("pockettui35").join("config.toml"))
}

/// Merges the `[item.*]` tables of all layers. A layer that fails to parse is
/// skipped as a whole, with a warning.
pub fn merge_layers(layers: &[Layer]) -> (BTreeMap<String, toml::Table>, Vec<String>) {
    let mut merged: BTreeMap<String, toml::Table> = BTreeMap::new();
    let mut warnings = Vec::new();

    for layer in layers {
        let doc: toml::Table = match layer.text.parse() {
            Ok(doc) => doc,
            Err(e) => {
                let msg = e.to_string();
                warnings.push(format!("{}: {}", layer.name, msg.trim_end()));
                continue;
            }
        };

        for (key, value) in doc {
            if key != "item" {
                warnings.push(format!("{}: unknown section `{key}`", layer.name));
                continue;
            }
            let Some(items) = value.as_table() else {
                warnings.push(format!("{}: `item` must be a table", layer.name));
                continue;
            };
            for (id, fields) in items {
                let Some(fields) = fields.as_table() else {
                    warnings.push(format!("{}: `item.{id}` must be a table", layer.name));
                    continue;
                };
                let entry = merged.entry(id.clone()).or_default();
                let type_changed = matches!(
                    (entry.get("type"), fields.get("type")),
                    (Some(old), Some(new)) if old != new
                );
                if type_changed {
                    entry.clear();
                }
                entry.extend(fields.clone());
            }
        }
    }

    (merged, warnings)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layer(name: &str, text: &str) -> Layer {
        Layer {
            name: name.into(),
            text: text.into(),
        }
    }

    #[test]
    fn broken_layer_is_skipped() {
        let (merged, warnings) = merge_layers(&[
            layer("shipped", SHIPPED),
            layer("user", "[item.files\nlabel = 1"),
        ]);
        assert!(merged.contains_key("files"));
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].starts_with("user:"), "{warnings:?}");
    }

    #[test]
    fn unknown_section_warns() {
        let (_, warnings) = merge_layers(&[layer("user", "[apps.x]\nlabel = \"x\"\n")]);
        assert_eq!(warnings, ["user: unknown section `apps`"]);
    }
}
