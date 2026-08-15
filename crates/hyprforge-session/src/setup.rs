//! One-time insertion of `require("hyprforge/session")` into the user's
//! `hyprland.lua`.
//!
//! Placed last, like the other settings modules. `hl.env` and
//! `hl.permission` resolve last-one-wins, so sourcing first would let the
//! user's own config silently override anything saved here.
//!
//! Autostart doesn't care about placement — it registers a handler for an
//! event that fires after the whole config is loaded — and gestures can't
//! be resolved by ordering at all, since Hyprland refuses a duplicate
//! rather than taking the later one. See [`crate::gestures`].

use hyprforge_core::lua_setup::{self, Placement};
use std::path::Path;

pub use hyprforge_core::lua_setup::{HyprConfig, SetupError, SetupPlan};

pub const PLACEMENT: Placement = Placement::AtEnd;

#[cfg(test)]
const MODULE: &str = "session";

pub const REQUIRE_LINE: &str = "require(\"hyprforge/session\")";

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

pub fn create_lua_config(path: &Path) -> Result<(), SetupError> {
    lua_setup::create_lua_config(path, &require_line_string(), PLACEMENT)
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
        let cfg = "hl.env(\"GTK_THEME\", \"Nord\")\n";
        let out = apply(cfg, &detect(cfg));
        assert!(
            out.find(REQUIRE_LINE) > out.find("GTK_THEME"),
            "session must be sourced last:\n{out}"
        );
    }

    #[test]
    fn an_already_present_line_is_not_inserted_twice() {
        let cfg = format!("{REQUIRE_LINE}\n");
        assert_eq!(apply(&cfg, &detect(&cfg)).matches(REQUIRE_LINE).count(), 1);
    }
}
