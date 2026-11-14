//! The item model and the menu tree built from merged config.
//!
//! Every menu entry is an item. Folders hold other items; "home" is the
//! implicit root and is not an item itself.

use std::collections::{BTreeMap, HashMap, HashSet};

use serde::Deserialize;

/// Id of the implicit root folder.
pub const HOME: &str = "home";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ItemType {
    Folder,
    App,
    Script,
    Value,
    Choice,
    Action,
}

/// How an `app` item takes the screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    /// Runs in the launcher's terminal, launcher suspended.
    Tty,
    /// Runs as `cage -- <exec>`.
    Wayland,
    /// Takes DRM directly.
    Kms,
}

/// Behaviour implemented inside the launcher.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Builtin {
    Volume,
    Brightness,
    PowerProfile,
    ScreenOff,
    Reboot,
    Shutdown,
    About,
}

impl Builtin {
    /// The item type this builtin belongs to.
    fn item_type(self) -> ItemType {
        match self {
            Self::Volume | Self::Brightness => ItemType::Value,
            Self::PowerProfile => ItemType::Choice,
            Self::ScreenOff | Self::Reboot | Self::Shutdown | Self::About => ItemType::Action,
        }
    }
}

/// What pressing A on an item does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Folder,
    App {
        kind: Kind,
        exec: String,
        raw_buttons: bool,
    },
    Script {
        exec: String,
    },
    Value(Builtin),
    Choice(Builtin),
    Builtin {
        builtin: Builtin,
        confirm: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    pub id: String,
    pub label: String,
    pub pin: bool,
    pub action: Action,
}

impl Item {
    pub fn is_folder(&self) -> bool {
        self.action == Action::Folder
    }
}

/// One `[item.<id>]` table as written, after layer merging.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawItem {
    label: Option<String>,
    #[serde(rename = "type")]
    ty: Option<ItemType>,
    parent: Option<String>,
    #[serde(default)]
    pin: bool,
    /// Declared so `deny_unknown_fields` accepts it. `Tree::build` reads it
    /// from the raw table, before deserializing.
    #[serde(default)]
    #[allow(dead_code)]
    hidden: bool,
    kind: Option<Kind>,
    exec: Option<String>,
    #[serde(default)]
    raw_buttons: bool,
    builtin: Option<Builtin>,
    #[serde(default)]
    confirm: bool,
}

impl RawItem {
    fn into_item(self, id: &str) -> Result<Item, String> {
        let label = self
            .label
            .filter(|l| !l.trim().is_empty())
            .ok_or("missing `label`")?;
        let ty = self.ty.ok_or("missing `type`")?;
        let exec = || {
            self.exec
                .clone()
                .filter(|e| !e.trim().is_empty())
                .ok_or(format!("type `{}` needs `exec`", type_name(ty)))
        };
        let builtin = || {
            let b = self
                .builtin
                .ok_or(format!("type `{}` needs `builtin`", type_name(ty)))?;
            if b.item_type() == ty {
                Ok(b)
            } else {
                Err(format!(
                    "builtin {b:?} is not a `{}` builtin",
                    type_name(ty)
                ))
            }
        };

        let action = match ty {
            ItemType::Folder => Action::Folder,
            ItemType::App => Action::App {
                kind: self.kind.ok_or("type `app` needs `kind`")?,
                exec: exec()?,
                raw_buttons: self.raw_buttons,
            },
            ItemType::Script => Action::Script { exec: exec()? },
            ItemType::Value => Action::Value(builtin()?),
            ItemType::Choice => Action::Choice(builtin()?),
            ItemType::Action => Action::Builtin {
                builtin: builtin()?,
                confirm: self.confirm,
            },
        };

        Ok(Item {
            id: id.to_string(),
            label,
            pin: self.pin,
            action,
        })
    }
}

fn type_name(ty: ItemType) -> &'static str {
    match ty {
        ItemType::Folder => "folder",
        ItemType::App => "app",
        ItemType::Script => "script",
        ItemType::Value => "value",
        ItemType::Choice => "choice",
        ItemType::Action => "action",
    }
}

/// The menu: items by id, and each folder's children in display order.
#[derive(Debug, Default, Clone)]
pub struct Tree {
    items: HashMap<String, Item>,
    children: HashMap<String, Vec<String>>,
    /// Each item's folder, after bad parents fell back to Home.
    parents: HashMap<String, String>,
    /// Problems found while loading. Shown to the user, never fatal.
    pub warnings: Vec<String>,
}

impl Tree {
    pub fn get(&self, id: &str) -> Option<&Item> {
        self.items.get(id)
    }

    /// The folder an item sits in. `None` for Home and unknown ids.
    pub fn parent(&self, id: &str) -> Option<&str> {
        self.parents.get(id).map(String::as_str)
    }

    /// Every item in the tree, in no particular order.
    pub fn all(&self) -> impl Iterator<Item = &Item> {
        self.items.values()
    }

    /// Children of a folder, pinned first, then alphabetical by label.
    pub fn children(&self, folder: &str) -> &[String] {
        self.children.get(folder).map_or(&[], Vec::as_slice)
    }

    /// Builds the tree from merged `[item.*]` tables. Bad items are dropped
    /// with a warning; the rest of the menu still loads.
    pub fn build(merged: BTreeMap<String, toml::Table>, mut warnings: Vec<String>) -> Self {
        let parent_of = |id: &str| -> Option<&str> {
            merged
                .get(id)
                .and_then(|t| t.get("parent"))
                .and_then(|v| v.as_str())
        };
        let is_hidden = |id: &str| -> bool {
            merged
                .get(id)
                .and_then(|t| t.get("hidden"))
                .and_then(|v| v.as_bool())
                == Some(true)
        };
        // True if the item or any folder above it is hidden. Bounded so a
        // parent cycle cannot loop forever.
        let under_hidden = |id: &str| -> bool {
            let mut cur = Some(id);
            for _ in 0..=merged.len() {
                match cur {
                    Some(c) if c == HOME => return false,
                    Some(c) if is_hidden(c) => return true,
                    Some(c) => cur = parent_of(c),
                    None => return false,
                }
            }
            false
        };

        // 1. Validate each visible item.
        let mut items = HashMap::new();
        let mut parents = HashMap::new();
        for (id, table) in &merged {
            if id == HOME {
                warnings.push(format!("item.{id}: `{HOME}` is reserved for the root"));
                continue;
            }
            if under_hidden(id) {
                continue;
            }
            let raw = match RawItem::deserialize(toml::Value::Table(table.clone())) {
                Ok(raw) => raw,
                Err(e) => {
                    warnings.push(format!("item.{id}: {}", e.message()));
                    continue;
                }
            };
            let parent = raw.parent.clone().unwrap_or_else(|| HOME.to_string());
            match raw.into_item(id) {
                Ok(item) => {
                    items.insert(id.clone(), item);
                    parents.insert(id.clone(), parent);
                }
                Err(e) => warnings.push(format!("item.{id}: {e}")),
            }
        }

        // 2. Attach each item to its parent. A bad parent falls back to Home
        //    so the item is not lost.
        let mut children: HashMap<String, Vec<String>> = HashMap::new();
        let mut ids: Vec<String> = items.keys().cloned().collect();
        ids.sort();
        for id in &ids {
            let parent = &parents[id];
            let ok = parent == HOME || items.get(parent).is_some_and(Item::is_folder);
            let parent = if ok {
                parent.clone()
            } else {
                let why = if items.contains_key(parent) {
                    "is not a folder"
                } else {
                    "does not exist"
                };
                warnings.push(format!("item.{id}: parent `{parent}` {why}, shown in Home"));
                HOME.to_string()
            };
            children.entry(parent.clone()).or_default().push(id.clone());
            parents.insert(id.clone(), parent);
        }

        // 3. Drop folders not reachable from Home (parent cycles).
        let mut reachable = HashSet::new();
        let mut stack = vec![HOME.to_string()];
        while let Some(f) = stack.pop() {
            for c in children.get(&f).into_iter().flatten() {
                if reachable.insert(c.clone()) {
                    stack.push(c.clone());
                }
            }
        }
        let mut lost: Vec<&String> = items.keys().filter(|id| !reachable.contains(*id)).collect();
        lost.sort();
        for id in lost {
            warnings.push(format!("item.{id}: in a parent cycle, not shown"));
        }
        items.retain(|id, _| reachable.contains(id));
        parents.retain(|id, _| reachable.contains(id));
        children.retain(|f, _| f == HOME || reachable.contains(f));

        // 4. Sort: pinned first, then by label, case-insensitive.
        for list in children.values_mut() {
            list.sort_by_cached_key(|id| {
                let item = &items[id];
                (!item.pin, item.label.to_lowercase(), id.clone())
            });
        }

        Self {
            items,
            children,
            parents,
            warnings,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Layer, merge_layers};

    fn tree(layers: &[(&str, &str)]) -> Tree {
        let layers: Vec<Layer> = layers
            .iter()
            .map(|(n, t)| Layer {
                name: n.to_string(),
                text: t.to_string(),
            })
            .collect();
        let (merged, warnings) = merge_layers(&layers);
        Tree::build(merged, warnings)
    }

    fn labels(t: &Tree, folder: &str) -> Vec<String> {
        t.children(folder)
            .iter()
            .map(|id| t.get(id).unwrap().label.clone())
            .collect()
    }

    #[test]
    fn shipped_config_loads_clean() {
        let t = tree(&[("shipped", crate::config::SHIPPED)]);
        assert!(t.warnings.is_empty(), "{:?}", t.warnings);
        assert_eq!(
            labels(&t, HOME),
            [
                "Chromium", "Files", "Games", "Power", "Settings", "Terminal"
            ]
        );
        assert_eq!(labels(&t, "power"), ["Reboot", "Screen off", "Shut down"]);
        assert_eq!(
            t.get("reboot").unwrap().action,
            Action::Builtin {
                builtin: Builtin::Reboot,
                confirm: true
            }
        );
    }

    #[test]
    fn pinned_sort_first() {
        let t = tree(&[(
            "u",
            r#"
            [item.a]
            label = "Alpha"
            type = "folder"
            [item.z]
            label = "Zulu"
            type = "folder"
            pin = true
            "#,
        )]);
        assert_eq!(labels(&t, HOME), ["Zulu", "Alpha"]);
    }

    #[test]
    fn user_layer_merges_fields() {
        let t = tree(&[
            ("shipped", crate::config::SHIPPED),
            ("user", "[item.files]\nexec = \"ranger\"\npin = true\n"),
        ]);
        assert!(t.warnings.is_empty(), "{:?}", t.warnings);
        let files = t.get("files").unwrap();
        assert_eq!(files.label, "Files");
        assert!(files.pin);
        assert!(matches!(&files.action, Action::App { exec, .. } if exec == "ranger"));
        assert_eq!(t.children(HOME)[0], "files");
    }

    #[test]
    fn changing_type_replaces_the_item() {
        // `kind` from the shipped app must not leak into the script.
        let t = tree(&[
            ("shipped", crate::config::SHIPPED),
            (
                "user",
                "[item.files]\nlabel = \"Files\"\ntype = \"script\"\nexec = \"ls\"\n",
            ),
        ]);
        assert!(t.warnings.is_empty(), "{:?}", t.warnings);
        assert_eq!(
            t.get("files").unwrap().action,
            Action::Script { exec: "ls".into() }
        );
    }

    #[test]
    fn hidden_folder_hides_its_children() {
        let t = tree(&[
            ("shipped", crate::config::SHIPPED),
            ("user", "[item.power]\nhidden = true\n"),
        ]);
        assert!(t.warnings.is_empty(), "{:?}", t.warnings);
        assert!(t.get("power").is_none());
        assert!(t.get("reboot").is_none());
        assert!(!labels(&t, HOME).contains(&"Power".to_string()));
    }

    #[test]
    fn bad_items_warn_and_rest_loads() {
        let t = tree(&[(
            "u",
            r#"
            [item.ok]
            label = "Ok"
            type = "script"
            exec = "true"

            [item.noexec]
            label = "No exec"
            type = "app"
            kind = "tty"

            [item.typo]
            label = "Typo"
            type = "script"
            exec = "true"
            pinn = true

            [item.wrongbuiltin]
            label = "Wrong"
            type = "value"
            builtin = "reboot"

            [item.orphan]
            label = "Orphan"
            type = "script"
            exec = "true"
            parent = "nowhere"
            "#,
        )]);
        assert_eq!(labels(&t, HOME), ["Ok", "Orphan"]);
        let w = t.warnings.join("\n");
        assert!(w.contains("item.noexec: type `app` needs `exec`"), "{w}");
        assert!(w.contains("item.typo:") && w.contains("pinn"), "{w}");
        assert!(w.contains("item.wrongbuiltin:"), "{w}");
        assert!(
            w.contains("item.orphan: parent `nowhere` does not exist"),
            "{w}"
        );
    }

    #[test]
    fn parent_cycle_is_dropped() {
        let t = tree(&[(
            "u",
            r#"
            [item.a]
            label = "A"
            type = "folder"
            parent = "b"
            [item.b]
            label = "B"
            type = "folder"
            parent = "a"
            "#,
        )]);
        assert!(t.children(HOME).is_empty());
        assert_eq!(t.warnings.len(), 2, "{:?}", t.warnings);
    }
}
