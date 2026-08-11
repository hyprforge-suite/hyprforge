//! One-time insertion of `require("hyprforge/window-rules")` into the user's
//! `hyprland.lua`.
//!
//! The mechanism — detect, back up, insert, write atomically — is generic and
//! lives in [`hyprforge_core::lua_setup`]; this module supplies the two things
//! that are specific to window rules: which file is being sourced, and where
//! the line belongs.
//!
//! Hyprland 0.56 evaluates all *named* rules first, then all anonymous ones,
//! and the **last** match wins. Hyprforge rules are always named (see
//! [`crate::model::Rule::name`]), so they are placed **first** among the
//! user's `require()` calls: this makes Hyprforge's named rules the
//! earliest-evaluated, so the user's own rules — named or anonymous —
//! override them by default. Appending last would invert that and let
//! Hyprforge silently beat the user's own named rules.

use hyprforge_core::lua_setup::{self, Placement};
use std::path::Path;

pub use hyprforge_core::lua_setup::{HyprConfig, SetupError, SetupPlan};

/// Window rules go first, so the user's own rules win. See the module docs.
const PLACEMENT: Placement = Placement::BeforeUserRequires;

const MODULE: &str = "window-rules";

/// The exact line inserted into `hyprland.lua`. Spelled out rather than
/// computed so it can be a `const` callers match on; a test below pins it
/// against the generic builder so the two can't drift apart.
pub const REQUIRE_LINE: &str = "require(\"hyprforge/window-rules\")";

fn require_line_string() -> String {
    REQUIRE_LINE.to_string()
}

pub fn discover(hypr_dir: &Path) -> HyprConfig {
    lua_setup::discover(hypr_dir)
}

/// Detects whether `hyprland_lua` already sources the window-rules file.
pub fn detect(hyprland_lua: &str) -> SetupPlan {
    lua_setup::detect(hyprland_lua, &require_line_string(), PLACEMENT)
}

/// Applies `plan`, returning the new full file contents.
pub fn apply(hyprland_lua: &str, plan: &SetupPlan) -> String {
    lua_setup::apply(hyprland_lua, plan, &require_line_string())
}

/// Renders the line to be inserted, for the GUI's confirmation dialog.
pub fn preview_line() -> String {
    require_line_string()
}

/// Renders the file [`create_lua_config`] would write.
pub fn preview_lua_config() -> String {
    lua_setup::preview_lua_config(&[require_line_string()])
}

/// Creates a minimal `hyprland.lua` sourcing the window-rules file.
pub fn create_lua_config(path: &Path) -> Result<(), SetupError> {
    lua_setup::create_lua_config(path, &[require_line_string()])
}

/// Backs up and inserts the require line, if it isn't already there.
pub fn install(path: &Path) -> Result<SetupPlan, SetupError> {
    lua_setup::install(path, &require_line_string(), PLACEMENT)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The const above is hand-written; this is what stops it drifting from
    /// the path the generic installer actually uses.
    #[test]
    fn the_spelled_out_line_matches_the_generated_one() {
        assert_eq!(REQUIRE_LINE, lua_setup::require_line(MODULE));
    }

    /// Window rules must evaluate before the user's own requires, so their
    /// rules win. Getting this backwards would let Hyprforge silently beat
    /// hand-written named rules.
    #[test]
    fn the_line_goes_before_the_users_own_requires() {
        let cfg = "require(\"theirs\")\n";
        let out = apply(cfg, &detect(cfg));
        assert_eq!(out.lines().next().unwrap(), REQUIRE_LINE);
    }
}
