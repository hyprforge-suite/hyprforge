//! The six keybinds, each its own item.
//!
//! They go into `shortcuts.toml` exactly as the Shortcuts page would put
//! them there — a `Shortcut` with the usual `hyprforge: ` description —
//! so that page lists them and can edit or remove them, and `hyprctl
//! binds` shows whose they are.
//!
//! **A chord that is taken is skipped and named, never overwritten.**
//! Taken means a shortcut in `shortcuts.toml` already uses it, or the
//! compositor reports a bind for it (`hyprctl binds -j`, which sees the
//! user's hand-written `mainMod .. " + V"` that no text search could).
//! Keybinds are required *last*, so a bind written here would silently
//! win over the user's own — which is the reason not to write one.

use super::{Applied, Cx};
use crate::record::Change;
use crate::state::State;
use crate::system::Generated;
use hyprforge_shortcuts::binds::{conflicts_for, LiveBind};
use hyprforge_shortcuts::model::{generate_shortcut_name, Action, KeyCombo, Modifier, ParamValue};
use hyprforge_shortcuts::Shortcut;
use std::collections::BTreeMap;

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct BindSpec {
    pub mods: &'static [Modifier],
    /// The xkb keysym name Hyprland binds against — `period`, not `.`;
    /// `Escape`, not `Esc`. The Settings app's key capture writes these
    /// names (`keycapture.rs`), and a hand-written config may use either
    /// spelling for the full stop; [`spellings`] checks both.
    pub key: &'static str,
    /// What `exec_cmd` runs.
    pub command: &'static str,
    /// The description, without the `hyprforge: ` prefix codegen adds.
    pub label: &'static str,
    pub requires: &'static [&'static str],
}

pub(crate) const CLIPBOARD: BindSpec = BindSpec {
    mods: &[Modifier::Super],
    key: "V",
    command: "hyprforge-clipmenu",
    label: "Clipboard history",
    requires: &["hyprforge-clipmenu"],
};
pub(crate) const EMOJI: BindSpec = BindSpec {
    mods: &[Modifier::Super],
    key: "period",
    command: "hyprforge-emojimenu",
    label: "Emoji picker",
    requires: &["hyprforge-emojimenu"],
};
pub(crate) const NOTIFICATIONS: BindSpec = BindSpec {
    mods: &[Modifier::Super],
    key: "N",
    command: "notifctl center",
    label: "Notification centre",
    requires: &["notifctl"],
};
pub(crate) const DND: BindSpec = BindSpec {
    mods: &[Modifier::Super, Modifier::Shift],
    key: "N",
    command: "notifctl dnd",
    label: "Do Not Disturb",
    requires: &["notifctl"],
};
pub(crate) const FILES: BindSpec = BindSpec {
    mods: &[Modifier::Super],
    key: "E",
    command: "hyprforge-files",
    label: "Files",
    requires: &["hyprforge-files"],
};
pub(crate) const LOCK: BindSpec = BindSpec {
    mods: &[Modifier::Super],
    key: "Escape",
    command: "hyprforge-lock",
    label: "Lock screen",
    requires: &["hyprforge-lock"],
};

impl BindSpec {
    fn combo(&self) -> KeyCombo {
        KeyCombo { mods: self.mods.to_vec(), key: self.key.to_string() }
    }

    /// Every spelling of this chord a bind could have used. Hyprland
    /// resolves both `period` and `.`, and reports back whichever the
    /// config wrote.
    fn spellings(&self) -> Vec<KeyCombo> {
        let mut out = vec![self.combo()];
        if self.key == "period" {
            out.push(KeyCombo { mods: self.mods.to_vec(), key: ".".into() });
        }
        out
    }

    /// `Super+.`, the way a person reads it.
    pub(crate) fn chord(&self) -> String {
        let mut parts: Vec<&str> = Modifier::ALL
            .into_iter()
            .filter(|m| self.mods.contains(m))
            .rev()
            .map(|m| match m {
                Modifier::Super => "Super",
                Modifier::Shift => "Shift",
                Modifier::Ctrl => "Ctrl",
                Modifier::Alt => "Alt",
                _ => m.keyword(),
            })
            .collect();
        parts.push(match self.key {
            "period" => ".",
            other => other,
        });
        parts.join("+")
    }

    fn shortcut(&self, existing: &[Shortcut]) -> Shortcut {
        let names: Vec<String> = existing.iter().map(|s| s.name.clone()).collect();
        let mut params = BTreeMap::new();
        params.insert("cmd".to_string(), ParamValue::Str(self.command.to_string()));
        Shortcut {
            name: generate_shortcut_name(self.label, &names),
            enabled: true,
            combo: self.combo(),
            action: Action::with_params("exec_cmd", params),
            description: self.label.to_string(),
            flags: Default::default(),
        }
    }
}

/// The command an `exec_cmd` shortcut runs, however it was stored: the
/// structured `cmd` parameter, or a raw Lua string from an older file or
/// an import.
fn command_of(shortcut: &Shortcut) -> Option<String> {
    if shortcut.action.dispatcher.trim() != "exec_cmd" {
        return None;
    }
    if let Some(raw) = &shortcut.action.raw {
        let raw = raw.trim();
        let unquoted = raw
            .strip_prefix("[[")
            .and_then(|r| r.strip_suffix("]]"))
            .or_else(|| raw.strip_prefix('"').and_then(|r| r.strip_suffix('"')))
            .unwrap_or(raw);
        return Some(unquoted.trim().to_string());
    }
    match shortcut.action.params.get("cmd") {
        Some(ParamValue::Str(cmd)) => Some(cmd.trim().to_string()),
        _ => None,
    }
}

fn load(cx: &Cx<'_>) -> Result<Vec<Shortcut>, String> {
    hyprforge_shortcuts::storage::load(&cx.env.shortcuts_toml()).map_err(|e| e.to_string())
}

pub(super) fn check(cx: &Cx<'_>, spec: &BindSpec) -> State {
    let shortcuts = match load(cx) {
        Ok(s) => s,
        Err(e) => return State::Unknown { why: e },
    };
    let enabled = || shortcuts.iter().filter(|s| s.enabled);

    // Bound to something already, on any chord: the user may have moved
    // it, and that is still done.
    if enabled().any(|s| command_of(s).as_deref() == Some(spec.command)) {
        return State::Done;
    }
    let chord = spec.chord();
    if let Some(taken) =
        enabled().find(|s| spec.spellings().iter().any(|c| s.combo.conflicts_with(c)))
    {
        let what = if taken.description.trim().is_empty() { &taken.name } else { &taken.description };
        return State::Unavailable {
            why: format!(
                "{chord} is already your Hyprforge shortcut \"{what}\" — bind {} to another \
                 chord in Settings → Shortcuts",
                spec.command
            ),
        };
    }
    let live = match cx.sys.live_binds() {
        Ok(binds) => binds,
        Err(e) => {
            return State::Unknown {
                why: format!("couldn't ask Hyprland what {chord} is bound to: {e}"),
            }
        }
    };
    if let Some(label) = live_conflict(&live, spec) {
        return State::Unavailable {
            why: format!(
                "{chord} is already bound in your Hyprland config ({label}) — bind {} to \
                 another chord in Settings → Shortcuts",
                spec.command
            ),
        };
    }
    State::Todo { what: format!("Bind {chord} to {}", spec.command) }
}

fn live_conflict(live: &[LiveBind], spec: &BindSpec) -> Option<String> {
    spec.spellings()
        .iter()
        .flat_map(|combo| conflicts_for(live, combo, None))
        .map(|b| b.display_label().to_string())
        .next()
}

/// The generated keybinds file for `shortcuts`.
fn keybinds(cx: &Cx<'_>, shortcuts: &[Shortcut]) -> Generated {
    Generated {
        path: cx.env.keybinds_lua(),
        contents: hyprforge_shortcuts::codegen::generate(shortcuts),
        empty: hyprforge_shortcuts::codegen::generate(&[]),
        subject: "shortcuts",
    }
}

/// Saves `shortcuts` and loads them into Hyprland. If Hyprland refuses,
/// `apply_lua` has already put the old Lua back, and this puts the old
/// TOML back too — so the page never lists a bind that is not live.
fn save_and_load(cx: &Cx<'_>, previous: &[Shortcut], shortcuts: &[Shortcut]) -> Result<Option<String>, String> {
    let path = cx.env.shortcuts_toml();
    hyprforge_shortcuts::storage::save(&path, shortcuts).map_err(|e| e.to_string())?;
    match cx.sys.apply_lua(&keybinds(cx, shortcuts)) {
        Ok(loaded) => Ok(super::loaded_note(loaded)),
        Err(e) => {
            if let Err(restore) = hyprforge_shortcuts::storage::save(&path, previous) {
                return Err(format!("{e} — and putting shortcuts.toml back failed too: {restore}"));
            }
            Err(e)
        }
    }
}

pub(super) fn apply(cx: &Cx<'_>, spec: &BindSpec) -> Result<Applied, String> {
    let previous = load(cx)?;
    let shortcut = spec.shortcut(&previous);
    let name = shortcut.name.clone();
    let mut shortcuts = previous.clone();
    shortcuts.push(shortcut);
    let note = save_and_load(cx, &previous, &shortcuts)?;
    Ok(Applied { change: Change::Bind { name }, note })
}

/// Removes the shortcut setup added, by its name. One the user has since
/// deleted is already gone, which is what undo wanted.
pub(super) fn undo(cx: &Cx<'_>, _spec: &BindSpec, name: &str) -> Result<Option<String>, String> {
    let previous = load(cx)?;
    if !previous.iter().any(|s| s.name == name) {
        return Ok(None);
    }
    let shortcuts: Vec<Shortcut> = previous.iter().filter(|s| s.name != name).cloned().collect();
    save_and_load(cx, &previous, &shortcuts)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chords_read_the_way_a_person_says_them() {
        assert_eq!(CLIPBOARD.chord(), "Super+V");
        assert_eq!(EMOJI.chord(), "Super+.");
        assert_eq!(DND.chord(), "Super+Shift+N");
        assert_eq!(LOCK.chord(), "Super+Escape");
    }

    /// What Hyprland is given is the keysym name — `SUPER + period`, which
    /// is what the Shortcuts page's own codegen emits.
    #[test]
    fn the_generated_bind_uses_the_keysym_name() {
        let lua = hyprforge_shortcuts::codegen::generate(&[EMOJI.shortcut(&[])]);
        assert!(
            lua.contains("hl.bind([[SUPER + period]], hl.dsp.exec_cmd([[hyprforge-emojimenu]])"),
            "{lua}"
        );
        assert!(lua.contains("description = [[hyprforge: Emoji picker]]"), "{lua}");
    }

    /// A raw `[[cmd]]` from an older file counts the same as a parameter.
    #[test]
    fn a_raw_exec_command_is_recognised() {
        let mut s = FILES.shortcut(&[]);
        s.action = Action::with_raw("exec_cmd", "[[hyprforge-files]]");
        assert_eq!(command_of(&s).as_deref(), Some("hyprforge-files"));
    }
}
