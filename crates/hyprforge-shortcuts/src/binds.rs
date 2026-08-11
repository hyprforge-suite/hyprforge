//! What's already bound, read from the compositor.
//!
//! The obvious approach — parse `hl.bind` lines out of `hyprland.lua` — does
//! not work, and it's worth being explicit about why, because it looks like it
//! should. A real config binds things like:
//!
//! ```lua
//! hl.bind(mainMod .. " + C", hl.dsp.window.close())
//! ```
//!
//! Static parsing yields the literal `mainMod .. " + C"`. Resolving that means
//! evaluating the user's Lua, with their variables, in their order. Hyprland
//! has already done exactly that, and `hyprctl binds -j` reports the result as
//! a resolved modmask and key — so the compositor is both easier to ask and
//! more correct than anything this crate could parse.
//!
//! The limit is that a Lua-defined bind reports its dispatcher as `__lua` with
//! an opaque callback index, so *what* a bind does isn't recoverable. That's
//! fine for the job here: conflict detection needs to know a chord is taken,
//! not what it's taken by.

use crate::model::{KeyCombo, Modifier, DESCRIPTION_PREFIX};
use serde::Deserialize;
use std::process::Command;

#[derive(Debug, thiserror::Error)]
pub enum BindsError {
    #[error("could not run hyprctl: {0}")]
    Spawn(#[source] std::io::Error),
    #[error("hyprctl binds failed: {stderr}")]
    Failed { stderr: String },
    #[error("could not parse hyprctl binds output: {0}")]
    Parse(#[source] serde_json::Error),
}

/// One bind currently live in the compositor, whoever defined it.
#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct LiveBind {
    pub modmask: u32,
    pub key: String,
    /// `__lua` for anything defined in a Lua config, which in practice is
    /// everything.
    pub dispatcher: String,
    pub description: String,
    /// Binds inside a submap only apply in that mode, so they don't conflict
    /// with a global one.
    pub submap: String,
    pub locked: bool,
}

impl LiveBind {
    pub fn combo(&self) -> KeyCombo {
        KeyCombo {
            mods: Modifier::from_mask(self.modmask),
            key: self.key.clone(),
        }
    }

    /// Whether this is one of ours, by the description prefix Hyprforge
    /// always writes. Hyprland doesn't report which file defined a bind, so
    /// this is the only runtime signal — and it's a reliable one, because the
    /// prefix is written by the same code that generates the bind.
    pub fn is_hyprforge(&self) -> bool {
        self.description.starts_with(DESCRIPTION_PREFIX)
    }

    /// The description with the prefix removed, for display.
    pub fn label(&self) -> &str {
        self.description
            .strip_prefix(DESCRIPTION_PREFIX)
            .unwrap_or(&self.description)
    }
}

pub fn list_binds() -> Result<Vec<LiveBind>, BindsError> {
    let out = Command::new("hyprctl")
        .arg("binds")
        .arg("-j")
        .output()
        .map_err(BindsError::Spawn)?;
    if !out.status.success() {
        return Err(BindsError::Failed {
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        });
    }
    parse_binds(&String::from_utf8_lossy(&out.stdout))
}

pub fn parse_binds(json: &str) -> Result<Vec<LiveBind>, BindsError> {
    serde_json::from_str(json).map_err(BindsError::Parse)
}

/// What already claims `combo`, ignoring any bind Hyprforge itself defined
/// under `own_name`.
///
/// Submap binds are skipped: they only fire inside that mode, so they aren't
/// a conflict for a global shortcut. Excluding the shortcut's own live bind
/// is what stops editing a saved shortcut from reporting a conflict with
/// itself.
pub fn conflicts_for<'a>(
    binds: &'a [LiveBind],
    combo: &KeyCombo,
    own_description: Option<&str>,
) -> Vec<&'a LiveBind> {
    binds
        .iter()
        .filter(|b| b.submap.is_empty())
        .filter(|b| b.combo().conflicts_with(combo))
        .filter(|b| match own_description {
            Some(own) => b.description != own,
            None => true,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Field names and shapes taken from this machine's `hyprctl binds -j`.
    const SAMPLE: &str = r#"[
        {"modmask":64,"key":"C","dispatcher":"__lua","arg":"8","description":"","submap":"","locked":false},
        {"modmask":12,"key":"T","dispatcher":"__lua","arg":"6","description":"","submap":"","locked":false},
        {"modmask":65,"key":"Q","dispatcher":"__lua","arg":"9","description":"hyprforge: Close window","submap":"","locked":false},
        {"modmask":64,"key":"R","dispatcher":"__lua","arg":"3","description":"","submap":"resize","locked":false}
    ]"#;

    fn combo(mask: u32, key: &str) -> KeyCombo {
        KeyCombo { mods: Modifier::from_mask(mask), key: key.to_string() }
    }

    #[test]
    fn parses_what_hyprctl_reports() {
        let binds = parse_binds(SAMPLE).unwrap();
        assert_eq!(binds.len(), 4);
        assert_eq!(binds[0].combo(), combo(64, "C"));
        assert_eq!(binds[0].dispatcher, "__lua");
    }

    #[test]
    fn an_existing_bind_is_reported_as_a_conflict() {
        let binds = parse_binds(SAMPLE).unwrap();
        let hits = conflicts_for(&binds, &combo(64, "C"), None);
        assert_eq!(hits.len(), 1);
    }

    #[test]
    fn a_free_combination_has_no_conflicts() {
        let binds = parse_binds(SAMPLE).unwrap();
        assert!(conflicts_for(&binds, &combo(64, "Z"), None).is_empty());
    }

    /// A submap bind only fires inside that mode, so it doesn't stand in the
    /// way of a global shortcut.
    #[test]
    fn submap_binds_are_not_global_conflicts() {
        let binds = parse_binds(SAMPLE).unwrap();
        assert!(conflicts_for(&binds, &combo(64, "R"), None).is_empty());
    }

    /// Editing a saved shortcut must not report a conflict with the bind it
    /// already installed.
    #[test]
    fn a_shortcut_does_not_conflict_with_itself() {
        let binds = parse_binds(SAMPLE).unwrap();
        let own = "hyprforge: Close window";
        assert!(conflicts_for(&binds, &combo(65, "Q"), Some(own)).is_empty());
        // ...but it still conflicts for anyone else.
        assert_eq!(conflicts_for(&binds, &combo(65, "Q"), None).len(), 1);
    }

    /// The prefix is the only runtime signal for "this one is ours".
    #[test]
    fn hyprforge_binds_are_distinguishable_from_the_users() {
        let binds = parse_binds(SAMPLE).unwrap();
        let ours: Vec<_> = binds.iter().filter(|b| b.is_hyprforge()).collect();
        assert_eq!(ours.len(), 1);
        assert_eq!(ours[0].label(), "Close window");
        assert!(!binds[0].is_hyprforge());
    }

    /// A user's bind with no description shows as itself, not as an empty
    /// strip of a prefix that was never there.
    #[test]
    fn a_bind_without_a_description_keeps_its_empty_label() {
        let binds = parse_binds(SAMPLE).unwrap();
        assert_eq!(binds[0].label(), "");
    }

    #[test]
    fn unknown_and_missing_keys_survive() {
        let binds = parse_binds(r#"[{"modmask":64,"key":"K","somethingNew":1}]"#).unwrap();
        assert_eq!(binds[0].description, "");
        assert!(binds[0].submap.is_empty());
    }

    #[test]
    fn malformed_json_is_an_error_rather_than_a_panic() {
        assert!(parse_binds("nope").is_err());
    }
}
