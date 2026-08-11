//! One-time insertion of `require("hyprforge/keybinds")` into `hyprland.lua`.
//!
//! The mechanism lives in [`hyprforge_core::lua_setup`]; this supplies the
//! module name and the placement.
//!
//! Keybinds go **last**, which is the opposite of window rules. Hyprland
//! resolves a repeated bind by last-one-wins, so appending means a shortcut
//! created here takes effect even though the user's config already binds that
//! chord. That's the right default for a settings app: you changed it *here*,
//! most recently, and expecting it to lose silently to a line in a file
//! you may not have opened in months would be the surprise. The conflict is
//! surfaced before saving either way — see [`crate::binds::conflicts_for`].

use hyprforge_core::lua_setup::{self, Placement};
use std::path::Path;

pub use hyprforge_core::lua_setup::{HyprConfig, SetupError, SetupPlan};

const PLACEMENT: Placement = Placement::AtEnd;

#[cfg(test)]
const MODULE: &str = "keybinds";

/// The exact line inserted into `hyprland.lua`.
pub const REQUIRE_LINE: &str = "require(\"hyprforge/keybinds\")";

pub fn discover(hypr_dir: &Path) -> HyprConfig {
    lua_setup::discover(hypr_dir)
}

pub fn detect(hyprland_lua: &str) -> SetupPlan {
    lua_setup::detect(hyprland_lua, REQUIRE_LINE, PLACEMENT)
}

pub fn apply(hyprland_lua: &str, plan: &SetupPlan) -> String {
    lua_setup::apply(hyprland_lua, plan, REQUIRE_LINE)
}

pub fn preview_line() -> String {
    REQUIRE_LINE.to_string()
}

pub fn install(path: &Path) -> Result<SetupPlan, SetupError> {
    lua_setup::install(path, REQUIRE_LINE, PLACEMENT)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_spelled_out_line_matches_the_generated_one() {
        assert_eq!(REQUIRE_LINE, lua_setup::require_line(MODULE));
    }

    /// Keybinds go last so a shortcut set here beats a chord the user's own
    /// config already binds — last bind wins in Hyprland.
    #[test]
    fn the_line_goes_after_the_users_own_requires() {
        let cfg = "require(\"theirs\")\n";
        let out = apply(cfg, &detect(cfg));
        assert_eq!(out.lines().last().unwrap(), REQUIRE_LINE);
    }
}
