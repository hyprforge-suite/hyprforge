//! What Hyprland's dispatchers are, and what each one takes.
//!
//! This exists so the editor can ask "which action?" and then render the
//! right fields, instead of asking the user to hand-write a Lua expression
//! that gets spliced into their config unvalidated. A malformed dispatcher
//! call isn't a shortcut that doesn't work — Hyprland rejects the whole
//! generated file, taking every other shortcut with it.
//!
//! **Entries are user-facing actions, not dispatchers one-to-one.** Hyprland
//! overloads several dispatchers by which key you pass — `focus` alone means
//! five different things depending on whether you give it `direction`,
//! `monitor`, `workspace`, `window` or `last`. Modelling that as one entry
//! with five mutually-exclusive optional fields would be worse than the free
//! text it replaces, so each *meaning* is its own entry and they happen to
//! share a `dispatcher`. [`resolve`] tells them apart by which keys are set,
//! which works precisely because the distinguishing keys are disjoint.
//!
//! Source: the Hyprland wiki's `Configuring/Basics/Dispatchers.md`, read for
//! 0.56. Nothing is guessed at — a dispatcher whose argument shape wasn't
//! verified is simply absent, and [`crate::model::Action::raw`] carries it
//! instead. `tests/live_lua.rs` loads every entry here into a real Hyprland,
//! so an entry that's wrong fails a test rather than a user's config.

use crate::model::{Action, ParamValue};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Category {
    Window,
    Workspace,
    Monitor,
    Launch,
    Session,
    Layout,
}

impl Category {
    pub const ALL: [Category; 6] = [
        Category::Window,
        Category::Workspace,
        Category::Monitor,
        Category::Launch,
        Category::Session,
        Category::Layout,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Category::Window => "Window",
            Category::Workspace => "Workspace",
            Category::Monitor => "Monitor",
            Category::Launch => "Launch",
            Category::Session => "Session",
            Category::Layout => "Layout",
        }
    }
}

/// How the dispatcher is called.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallShape {
    /// `hl.dsp.focus({ direction = "left" })` — the common case.
    Table,
    /// `hl.dsp.exec_cmd("ghostty")` — one bare value, not a table.
    Positional,
    /// `hl.dsp.exit()`
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParamKind {
    /// A fixed set of accepted strings, rendered as a dropdown.
    Enum(&'static [&'static str]),
    Text,
    /// Free text that means a number when it looks like one — `3` is
    /// workspace 3, `special:magic` and `e+1` are not. Getting this wrong
    /// emits `workspace = "3"` where Hyprland wants `workspace = 3`.
    Workspace,
    Int,
    Bool,
}

#[derive(Debug, Clone, Copy)]
pub struct Param {
    /// The Lua table key, or — for [`CallShape::Positional`] — just a name.
    pub key: &'static str,
    pub label: &'static str,
    pub kind: ParamKind,
    /// A required param is what tells two entries sharing a dispatcher
    /// apart; see [`resolve`].
    pub required: bool,
    /// Shown under the field. Empty for anything self-evident.
    pub hint: &'static str,
}

const fn req(key: &'static str, label: &'static str, kind: ParamKind) -> Param {
    Param { key, label, kind, required: true, hint: "" }
}

const fn opt(key: &'static str, label: &'static str, kind: ParamKind) -> Param {
    Param { key, label, kind, required: false, hint: "" }
}

const fn hinted(mut p: Param, hint: &'static str) -> Param {
    p.hint = hint;
    p
}

#[derive(Debug, Clone, Copy)]
pub struct Entry {
    /// Stable identifier. Not stored — [`resolve`] re-derives the entry from
    /// the dispatcher and keys — but stable anyway so it can key UI state.
    pub id: &'static str,
    pub label: &'static str,
    pub category: Category,
    /// Path under `hl.dsp`, e.g. `window.move`.
    pub dispatcher: &'static str,
    pub shape: CallShape,
    pub params: &'static [Param],
}

impl PartialEq for Entry {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

impl Eq for Entry {}

impl std::fmt::Display for Entry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.label)
    }
}

const DIRECTIONS: &[&str] = &["left", "right", "up", "down"];
const TOGGLE_ACTIONS: &[&str] = &["toggle", "set", "unset"];

pub const ENTRIES: &[Entry] = &[
    // ---- Window -------------------------------------------------------
    Entry {
        id: "window.close",
        label: "Close window",
        category: Category::Window,
        dispatcher: "window.close",
        shape: CallShape::Table,
        params: &[],
    },
    Entry {
        id: "window.kill",
        label: "Force-kill window",
        category: Category::Window,
        dispatcher: "window.kill",
        shape: CallShape::Table,
        params: &[],
    },
    Entry {
        id: "window.float",
        label: "Float window",
        category: Category::Window,
        dispatcher: "window.float",
        shape: CallShape::Table,
        params: &[opt("action", "Action", ParamKind::Enum(TOGGLE_ACTIONS))],
    },
    Entry {
        id: "window.fullscreen",
        label: "Fullscreen window",
        category: Category::Window,
        dispatcher: "window.fullscreen",
        shape: CallShape::Table,
        params: &[
            opt("mode", "Mode", ParamKind::Enum(&["fullscreen", "maximized"])),
            opt("action", "Action", ParamKind::Enum(TOGGLE_ACTIONS)),
        ],
    },
    Entry {
        id: "window.pseudo",
        label: "Pseudo-tile window",
        category: Category::Window,
        dispatcher: "window.pseudo",
        shape: CallShape::Table,
        params: &[opt("action", "Action", ParamKind::Enum(TOGGLE_ACTIONS))],
    },
    Entry {
        id: "window.pin",
        label: "Pin window",
        category: Category::Window,
        dispatcher: "window.pin",
        shape: CallShape::Table,
        params: &[opt("action", "Action", ParamKind::Enum(TOGGLE_ACTIONS))],
    },
    Entry {
        id: "window.center",
        label: "Center window",
        category: Category::Window,
        dispatcher: "window.center",
        shape: CallShape::Table,
        params: &[],
    },
    Entry {
        id: "window.move.direction",
        label: "Move window in direction",
        category: Category::Window,
        dispatcher: "window.move",
        shape: CallShape::Table,
        params: &[
            req("direction", "Direction", ParamKind::Enum(DIRECTIONS)),
            opt("group_aware", "Move in and out of groups", ParamKind::Bool),
        ],
    },
    Entry {
        id: "window.move.workspace",
        label: "Move window to workspace",
        category: Category::Window,
        dispatcher: "window.move",
        shape: CallShape::Table,
        params: &[
            hinted(
                req("workspace", "Workspace", ParamKind::Workspace),
                "A number, or name:gaming, special:magic, e+1, previous",
            ),
            opt("follow", "Follow the window there", ParamKind::Bool),
        ],
    },
    Entry {
        id: "window.move.monitor",
        label: "Move window to monitor",
        category: Category::Window,
        dispatcher: "window.move",
        shape: CallShape::Table,
        params: &[
            hinted(req("monitor", "Monitor", ParamKind::Text), "A name, or +1 / -1"),
            opt("follow", "Follow the window there", ParamKind::Bool),
        ],
    },
    Entry {
        id: "window.swap",
        label: "Swap window in direction",
        category: Category::Window,
        dispatcher: "window.swap",
        shape: CallShape::Table,
        params: &[req("direction", "Direction", ParamKind::Enum(DIRECTIONS))],
    },
    Entry {
        id: "window.cycle_next",
        label: "Focus next window",
        category: Category::Window,
        dispatcher: "window.cycle_next",
        shape: CallShape::Table,
        params: &[
            opt("tiled", "Tiled windows only", ParamKind::Bool),
            opt("floating", "Floating windows only", ParamKind::Bool),
        ],
    },
    Entry {
        id: "window.tag",
        label: "Tag window",
        category: Category::Window,
        dispatcher: "window.tag",
        shape: CallShape::Table,
        params: &[hinted(
            req("tag", "Tag", ParamKind::Text),
            "+name adds, -name removes, bare name toggles",
        )],
    },
    Entry {
        id: "window.clear_tags",
        label: "Clear window tags",
        category: Category::Window,
        dispatcher: "window.clear_tags",
        shape: CallShape::Table,
        params: &[],
    },
    Entry {
        id: "window.alter_zorder",
        label: "Raise or lower window",
        category: Category::Window,
        dispatcher: "window.alter_zorder",
        shape: CallShape::Table,
        params: &[req("mode", "Mode", ParamKind::Enum(&["top", "bottom"]))],
    },
    Entry {
        id: "window.toggle_swallow",
        label: "Toggle swallowed windows",
        category: Category::Window,
        dispatcher: "window.toggle_swallow",
        shape: CallShape::None,
        params: &[],
    },
    Entry {
        id: "window.drag",
        label: "Drag window (mouse bind)",
        category: Category::Window,
        dispatcher: "window.drag",
        shape: CallShape::None,
        params: &[],
    },
    Entry {
        id: "window.resize",
        label: "Resize window (mouse bind)",
        category: Category::Window,
        dispatcher: "window.resize",
        shape: CallShape::None,
        params: &[],
    },
    Entry {
        id: "focus.direction",
        label: "Focus window in direction",
        category: Category::Window,
        dispatcher: "focus",
        shape: CallShape::Table,
        params: &[req("direction", "Direction", ParamKind::Enum(DIRECTIONS))],
    },
    // ---- Workspace ----------------------------------------------------
    Entry {
        id: "focus.workspace",
        label: "Focus workspace",
        category: Category::Workspace,
        dispatcher: "focus",
        shape: CallShape::Table,
        params: &[
            hinted(
                req("workspace", "Workspace", ParamKind::Workspace),
                "A number, or name:gaming, e+1, previous",
            ),
            opt("on_current_monitor", "Stay on the current monitor", ParamKind::Bool),
        ],
    },
    Entry {
        id: "workspace.toggle_special",
        label: "Toggle special workspace",
        category: Category::Workspace,
        dispatcher: "workspace.toggle_special",
        shape: CallShape::Positional,
        params: &[hinted(
            req("name", "Name", ParamKind::Text),
            "The bare name, e.g. magic — not special:magic",
        )],
    },
    Entry {
        id: "workspace.move",
        label: "Move workspace to monitor",
        category: Category::Workspace,
        dispatcher: "workspace.move",
        shape: CallShape::Table,
        params: &[
            hinted(req("monitor", "Monitor", ParamKind::Text), "A name, or +1 / -1"),
            opt("workspace", "Workspace (current if empty)", ParamKind::Workspace),
        ],
    },
    Entry {
        id: "workspace.rename",
        label: "Rename workspace",
        category: Category::Workspace,
        dispatcher: "workspace.rename",
        shape: CallShape::Table,
        params: &[
            req("workspace", "Workspace", ParamKind::Workspace),
            opt("name", "New name", ParamKind::Text),
        ],
    },
    // ---- Monitor ------------------------------------------------------
    Entry {
        id: "focus.monitor",
        label: "Focus monitor",
        category: Category::Monitor,
        dispatcher: "focus",
        shape: CallShape::Table,
        params: &[hinted(
            req("monitor", "Monitor", ParamKind::Text),
            "A name, or +1 / -1",
        )],
    },
    Entry {
        id: "workspace.swap_monitors",
        label: "Swap two monitors' workspaces",
        category: Category::Monitor,
        dispatcher: "workspace.swap_monitors",
        shape: CallShape::Table,
        params: &[
            req("monitor1", "First monitor", ParamKind::Text),
            req("monitor2", "Second monitor", ParamKind::Text),
        ],
    },
    Entry {
        id: "dpms",
        label: "Turn monitors on or off",
        category: Category::Monitor,
        dispatcher: "dpms",
        shape: CallShape::Table,
        params: &[
            opt("action", "Action", ParamKind::Enum(&["toggle", "on", "off"])),
            opt("monitor", "Monitor (all if empty)", ParamKind::Text),
        ],
    },
    // ---- Launch -------------------------------------------------------
    Entry {
        id: "exec_cmd",
        label: "Run a command",
        category: Category::Launch,
        dispatcher: "exec_cmd",
        shape: CallShape::Positional,
        params: &[hinted(
            req("cmd", "Command", ParamKind::Text),
            "Run through sh -c, so pipes and && work",
        )],
    },
    Entry {
        id: "exec_raw",
        label: "Run a command without a shell",
        category: Category::Launch,
        dispatcher: "exec_raw",
        shape: CallShape::Positional,
        params: &[hinted(
            req("cmd", "Command", ParamKind::Text),
            "No sh -c, so no pipes, redirects or &&",
        )],
    },
    Entry {
        id: "global",
        label: "Trigger a global shortcut",
        category: Category::Launch,
        dispatcher: "global",
        shape: CallShape::Positional,
        params: &[req("name", "Name", ParamKind::Text)],
    },
    // ---- Session ------------------------------------------------------
    Entry {
        id: "exit",
        label: "Exit Hyprland",
        category: Category::Session,
        dispatcher: "exit",
        shape: CallShape::None,
        params: &[],
    },
    Entry {
        id: "submap",
        label: "Switch to a submap",
        category: Category::Session,
        dispatcher: "submap",
        shape: CallShape::Positional,
        params: &[hinted(
            req("name", "Submap", ParamKind::Text),
            "reset returns to the default submap",
        )],
    },
    Entry {
        id: "pass",
        label: "Pass the key to a window",
        category: Category::Session,
        dispatcher: "pass",
        // The wiki documents `window` as optional here; the compositor
        // rejects the call without it ("hl.pass: 'window' is required").
        // Verified against 0.56.1 by `tests/live_lua.rs`, which is how the
        // discrepancy surfaced.
        shape: CallShape::Table,
        params: &[hinted(
            req("window", "Window", ParamKind::Text),
            "A selector: class:foo, title:bar, address:0x…, or activewindow",
        )],
    },
    Entry {
        id: "event",
        label: "Send an event to socket2",
        category: Category::Session,
        dispatcher: "event",
        shape: CallShape::Positional,
        params: &[req("event", "Event", ParamKind::Text)],
    },
    Entry {
        id: "force_renderer_reload",
        label: "Reload the renderer",
        category: Category::Session,
        dispatcher: "force_renderer_reload",
        shape: CallShape::None,
        params: &[],
    },
    Entry {
        id: "no_op",
        label: "Do nothing (block the key)",
        category: Category::Session,
        dispatcher: "no_op",
        shape: CallShape::None,
        params: &[],
    },
    // ---- Layout -------------------------------------------------------
    Entry {
        id: "layout",
        label: "Send a layout message",
        category: Category::Layout,
        dispatcher: "layout",
        shape: CallShape::Positional,
        params: &[hinted(
            req("message", "Message", ParamKind::Text),
            "Layout-specific, e.g. swapwithmaster or togglesplit",
        )],
    },
];

pub fn by_id(id: &str) -> Option<&'static Entry> {
    ENTRIES.iter().find(|e| e.id == id)
}

pub fn in_category(category: Category) -> impl Iterator<Item = &'static Entry> {
    ENTRIES.iter().filter(move |e| e.category == category)
}

/// The catalog entry a stored action came from, if any.
///
/// Matching is by dispatcher plus the set of keys present, because that's
/// exactly what distinguishes the overloaded dispatchers from each other:
/// `focus` with `direction` is a different action to `focus` with
/// `workspace`. An entry only matches when every one of its required keys is
/// present and no stored key is one the entry doesn't know — so a call
/// carrying an argument this catalog doesn't model falls through to `None`
/// and gets the raw editor rather than being silently reshaped into
/// something similar.
///
/// Ties (several entries matching equally) are broken by most required keys
/// matched, so a more specific entry always wins over a barer one.
pub fn resolve(action: &Action) -> Option<&'static Entry> {
    if action.raw.is_some() {
        return None;
    }
    let dispatcher = action.dispatcher.trim();
    ENTRIES
        .iter()
        .filter(|entry| {
            entry.dispatcher == dispatcher
                && entry.params.iter().filter(|p| p.required).all(|p| action.params.contains_key(p.key))
                && action.params.keys().all(|k| entry.params.iter().any(|p| p.key == k.as_str()))
        })
        .max_by_key(|entry| entry.params.iter().filter(|p| p.required).count())
}

/// Parses text from the editor into the value type the param calls for.
///
/// Empty text means "not set" — the caller drops the key entirely rather
/// than emitting `direction = ""`, which Hyprland would reject.
pub fn parse_value(kind: ParamKind, text: &str) -> Option<ParamValue> {
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    Some(match kind {
        ParamKind::Bool => ParamValue::Bool(matches!(text, "true" | "yes" | "1")),
        ParamKind::Int => ParamValue::Int(text.parse().ok()?),
        // A workspace is a number when it looks like one and a string
        // otherwise — `3` and `special:magic` are both valid, and quoting
        // the former makes it a workspace *named* "3".
        ParamKind::Workspace => match text.parse::<i64>() {
            Ok(n) => ParamValue::Int(n),
            Err(_) => ParamValue::Str(text.to_string()),
        },
        ParamKind::Text | ParamKind::Enum(_) => ParamValue::Str(text.to_string()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn params(pairs: &[(&str, ParamValue)]) -> BTreeMap<String, ParamValue> {
        pairs.iter().map(|(k, v)| (k.to_string(), v.clone())).collect()
    }

    /// The overload case the whole matching scheme exists for.
    #[test]
    fn focus_resolves_by_which_key_is_set() {
        let direction = Action::with_params(
            "focus",
            params(&[("direction", ParamValue::Str("left".into()))]),
        );
        let workspace =
            Action::with_params("focus", params(&[("workspace", ParamValue::Int(3))]));
        assert_eq!(resolve(&direction).unwrap().id, "focus.direction");
        assert_eq!(resolve(&workspace).unwrap().id, "focus.workspace");
    }

    #[test]
    fn window_move_resolves_by_which_key_is_set() {
        for (key, value, expected) in [
            ("direction", ParamValue::Str("left".into()), "window.move.direction"),
            ("workspace", ParamValue::Int(2), "window.move.workspace"),
            ("monitor", ParamValue::Str("DP-1".into()), "window.move.monitor"),
        ] {
            let action = Action::with_params("window.move", params(&[(key, value)]));
            assert_eq!(resolve(&action).unwrap().id, expected);
        }
    }

    /// An optional key doesn't change which entry it is.
    #[test]
    fn an_optional_key_still_resolves() {
        let action = Action::with_params(
            "window.move",
            params(&[("workspace", ParamValue::Int(2)), ("follow", ParamValue::Bool(true))]),
        );
        assert_eq!(resolve(&action).unwrap().id, "window.move.workspace");
    }

    /// The escape hatch has to stay an escape hatch: an argument shape the
    /// catalog doesn't model must not be quietly matched to a near-miss
    /// entry and reshaped on the next save.
    #[test]
    fn an_unmodelled_key_does_not_resolve() {
        let action = Action::with_params(
            "focus",
            params(&[("something_new", ParamValue::Bool(true))]),
        );
        assert!(resolve(&action).is_none());
    }

    #[test]
    fn a_raw_action_never_resolves() {
        assert!(resolve(&Action::with_raw("focus", "{ direction = [[left]] }")).is_none());
    }

    #[test]
    fn a_missing_required_key_does_not_resolve() {
        assert!(resolve(&Action::with_params("window.swap", BTreeMap::new())).is_none());
    }

    #[test]
    fn a_no_argument_dispatcher_resolves_on_its_own() {
        assert_eq!(resolve(&Action::with_params("exit", BTreeMap::new())).unwrap().id, "exit");
    }

    /// `workspace = 3` and `workspace = "special:magic"` are different Lua
    /// types, and quoting the number would mean a workspace *named* "3".
    #[test]
    fn a_numeric_workspace_parses_as_a_number() {
        assert_eq!(parse_value(ParamKind::Workspace, "3"), Some(ParamValue::Int(3)));
        assert_eq!(
            parse_value(ParamKind::Workspace, "special:magic"),
            Some(ParamValue::Str("special:magic".into()))
        );
        assert_eq!(parse_value(ParamKind::Workspace, "e+1"), Some(ParamValue::Str("e+1".into())));
    }

    #[test]
    fn empty_text_is_not_a_value() {
        assert_eq!(parse_value(ParamKind::Text, "   "), None);
    }

    /// Ids key UI state and test expectations, so a duplicate would be a
    /// silent aliasing bug.
    #[test]
    fn entry_ids_are_unique() {
        let mut ids: Vec<&str> = ENTRIES.iter().map(|e| e.id).collect();
        ids.sort_unstable();
        let count = ids.len();
        ids.dedup();
        assert_eq!(ids.len(), count);
    }

    /// Every entry must round-trip: the action it describes has to resolve
    /// back to it, or editing a shortcut would silently switch its action.
    #[test]
    fn every_entry_resolves_back_to_itself() {
        for entry in ENTRIES {
            let params: BTreeMap<String, ParamValue> = entry
                .params
                .iter()
                .filter(|p| p.required)
                .map(|p| {
                    let value = match p.kind {
                        ParamKind::Bool => ParamValue::Bool(true),
                        ParamKind::Int => ParamValue::Int(1),
                        ParamKind::Workspace => ParamValue::Int(1),
                        ParamKind::Enum(values) => ParamValue::Str(values[0].to_string()),
                        ParamKind::Text => ParamValue::Str("x".to_string()),
                    };
                    (p.key.to_string(), value)
                })
                .collect();
            let action = Action::with_params(entry.dispatcher, params);
            assert_eq!(
                resolve(&action).map(|e| e.id),
                Some(entry.id),
                "{} did not resolve back to itself",
                entry.id
            );
        }
    }
}
