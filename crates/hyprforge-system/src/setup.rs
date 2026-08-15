//! One-time insertion of `require("hyprforge/system")`.
//!
//! Placed last, like every other `hl.config` module: a later call wins,
//! so sourcing first would let the user's own config silently override
//! anything saved here.

use hyprforge_core::lua_setup::{self, Placement};
use std::path::Path;

pub use hyprforge_core::lua_setup::{HyprConfig, SetupError, SetupPlan};

pub const PLACEMENT: Placement = Placement::AtEnd;

#[cfg(test)]
const MODULE: &str = "system";

pub const REQUIRE_LINE: &str = "require(\"hyprforge/system\")";

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

    #[test]
    fn the_line_goes_after_the_users_own_config() {
        let cfg = "hl.config({ misc = { vrr = 1 } })\n";
        let out = apply(cfg, &detect(cfg));
        assert!(out.find(REQUIRE_LINE) > out.find("vrr"), "{out}");
    }
}
