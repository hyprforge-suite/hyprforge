//! Blur behind notif's toasts and its centre panel.
//!
//! Two `hl.layer_rule`s in `window-rules.toml`, generated into
//! `window-rules.lua` by `hyprforge-windowrules` like every other rule.
//! The namespaces are notif's own, `notif` and `notif-center`
//! (notif-wl's module doc promises they stay stable for exactly this).
//! notif's README used to print hyprlang's `layerrule = blur, notif`,
//! which a Lua config cannot read at all.

use super::{Applied, Cx};
use crate::record::Change;
use crate::state::State;
use crate::system::Generated;
use hyprforge_windowrules::storage::Rules;
use hyprforge_windowrules::LayerRule;

/// `(name, namespace)`. Anchored, so `notif` does not also match
/// `notif-center` — they are two rules on purpose.
const RULES: [(&str, &str); 2] =
    [("hyprforge-notif-blur", "^notif$"), ("hyprforge-notif-center-blur", "^notif-center$")];

fn wanted(name: &str, namespace: &str) -> LayerRule {
    LayerRule {
        name: name.to_string(),
        enabled: true,
        namespace: namespace.to_string(),
        blur: Some(true),
    }
}

fn load(cx: &Cx<'_>) -> Result<Rules, String> {
    hyprforge_windowrules::storage::load(&cx.env.window_rules_toml()).map_err(|e| e.to_string())
}

pub(super) fn check(cx: &Cx<'_>) -> State {
    let rules = match load(cx) {
        Ok(r) => r,
        Err(e) => return State::Unknown { why: e },
    };
    let all_there = RULES.iter().all(|(name, namespace)| {
        rules.layer_rules.iter().any(|r| r == &wanted(name, namespace))
    });
    if all_there {
        State::Done
    } else {
        State::Todo { what: "Add blur layer rules for notif and notif-center".into() }
    }
}

fn generated(cx: &Cx<'_>, rules: &Rules) -> Generated {
    Generated {
        path: cx.env.window_rules_lua(),
        contents: hyprforge_windowrules::codegen::generate_all(rules),
        empty: hyprforge_windowrules::codegen::generate(&[], &[]),
        subject: "rules",
    }
}

/// Saves and loads `rules`, putting the old TOML back if Hyprland refuses
/// (it has already had its old Lua put back).
fn save_and_load(cx: &Cx<'_>, previous: &Rules, rules: &Rules) -> Result<Option<String>, String> {
    let path = cx.env.window_rules_toml();
    hyprforge_windowrules::storage::save(&path, rules).map_err(|e| e.to_string())?;
    match cx.sys.apply_lua(&generated(cx, rules)) {
        Ok(loaded) => Ok(super::loaded_note(loaded)),
        Err(e) => {
            if let Err(restore) = hyprforge_windowrules::storage::save(&path, previous) {
                return Err(format!("{e} — and putting window-rules.toml back failed too: {restore}"));
            }
            Err(e)
        }
    }
}

pub(super) fn apply(cx: &Cx<'_>) -> Result<Applied, String> {
    let before = load(cx)?;
    let mut rules = before.clone();
    let mut replaced = Vec::new();
    for (name, namespace) in RULES {
        // A rule under our name that differs (switched off by hand, say)
        // is replaced, and kept in the record so undo can put it back.
        if let Some(i) = rules.layer_rules.iter().position(|r| r.name == name) {
            replaced.push(rules.layer_rules.remove(i));
        }
        rules.layer_rules.push(wanted(name, namespace));
    }
    let note = save_and_load(cx, &before, &rules)?;
    Ok(Applied {
        change: Change::LayerRules {
            names: RULES.iter().map(|(n, _)| n.to_string()).collect(),
            previous: replaced,
        },
        note,
    })
}

pub(super) fn undo(
    cx: &Cx<'_>,
    names: &[String],
    previous: &[LayerRule],
) -> Result<Option<String>, String> {
    let before = load(cx)?;
    let mut rules = before.clone();
    rules.layer_rules.retain(|r| !names.contains(&r.name));
    rules.layer_rules.extend(previous.iter().cloned());
    if rules == before {
        return Ok(None);
    }
    save_and_load(cx, &before, &rules)
}
