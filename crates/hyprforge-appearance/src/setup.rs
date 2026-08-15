//! One-time insertion of `require("hyprforge/appearance")` into the
//! user's `hyprland.lua`.
//!
//! Placed **last**, for the same reason input is: `hl.config` is a scalar
//! overwrite and a later call wins. Sourced before the user's own
//! `hl.config({ decoration = ... })` block, every setting saved in the app
//! would be silently beaten by their config file and the app would report
//! saves that changed nothing.
//!
//! Animations share the placement rather than needing their own. They look
//! append-shaped, but a later `hl.animation` for the same leaf overrides
//! the earlier one (measured on 0.56.1), so the same "last wins" reasoning
//! applies and both halves can live in one file.

use hyprforge_core::lua_setup::{self, Placement};
use std::path::Path;

pub use hyprforge_core::lua_setup::{HyprConfig, SetupError, SetupPlan};

/// Appearance goes last, so a saved setting actually takes effect.
pub const PLACEMENT: Placement = Placement::AtEnd;

#[cfg(test)]
const MODULE: &str = "appearance";

/// The exact line inserted into `hyprland.lua`. Spelled out rather than
/// computed so it can be a `const` callers match on; a test below pins it
/// against the generic builder so the two can't drift apart.
pub const REQUIRE_LINE: &str = "require(\"hyprforge/appearance\")";

fn require_line_string() -> String {
    REQUIRE_LINE.to_string()
}

pub fn discover(hypr_dir: &Path) -> HyprConfig {
    lua_setup::discover(hypr_dir)
}

pub fn detect(hyprland_lua: &str) -> SetupPlan {
    lua_setup::detect(hyprland_lua, &require_line_string(), PLACEMENT)
}

pub fn apply(hyprland_lua: &str, plan: &SetupPlan) -> String {
    lua_setup::apply(hyprland_lua, plan, &require_line_string(), PLACEMENT)
}

pub fn preview_line() -> String {
    require_line_string()
}

pub fn preview_lua_config() -> String {
    lua_setup::preview_lua_config(&require_line_string(), PLACEMENT)
}

pub fn create_lua_config(path: &Path) -> Result<(), SetupError> {
    lua_setup::create_lua_config(path, &require_line_string(), PLACEMENT)
}

pub fn install(path: &Path) -> Result<SetupPlan, SetupError> {
    lua_setup::install(path, &require_line_string(), PLACEMENT)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_spelled_out_line_matches_the_generated_one() {
        assert_eq!(REQUIRE_LINE, lua_setup::require_line(MODULE));
    }

    /// The load-bearing one. Sourced before the user's own config, every
    /// setting saved in the app would be overwritten by their block and
    /// the GUI would be lying about what's running.
    #[test]
    fn the_line_goes_after_the_users_own_config() {
        let cfg = "require(\"theirs\")\nhl.config({ decoration = { rounding = 10 } })\n";
        let out = apply(cfg, &detect(cfg));
        let theirs = out.find("rounding").expect("their block should survive");
        let ours = out.find(REQUIRE_LINE).expect("our line should be inserted");
        assert!(ours > theirs, "appearance must be sourced last:\n{out}");
    }

    #[test]
    fn an_already_present_line_is_not_inserted_twice() {
        let cfg = format!("{REQUIRE_LINE}\n");
        let out = apply(&cfg, &detect(&cfg));
        assert_eq!(out.matches(REQUIRE_LINE).count(), 1, "{out}");
    }
}
