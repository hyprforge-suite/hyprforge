//! One-time insertion of `require("hyprforge/input")` into the user's
//! `hyprland.lua`.
//!
//! The mechanism — detect, back up, insert, write atomically — is generic
//! and lives in [`hyprforge_core::lua_setup`]; this module supplies what is
//! specific to input: which file is sourced, and where the line belongs.
//!
//! **Input is the one module placed last, and that is deliberate.**
//! `hl.config` is a scalar overwrite: a later call wins over an earlier one
//! for the keys it passes (measured on 0.56.1 — see [`crate::codegen`]).
//! Window rules go first so the user's own rules override Hyprforge's;
//! input cannot, because a user's `hl.config({ input = ... })` block would
//! then silently beat every setting saved in the app, and the app would
//! report a save that changed nothing — a dead end with no way to explain
//! itself. Placed last, the app owns the keys it writes and nothing else,
//! which is the only arrangement where what the GUI shows is what is
//! running.

use hyprforge_core::lua_setup::{self, Placement};
use std::path::Path;

pub use hyprforge_core::lua_setup::{HyprConfig, SetupError, SetupPlan};

/// Input settings go last, so a saved setting actually takes effect. See
/// the module docs.
pub const PLACEMENT: Placement = Placement::AtEnd;

/// Only the drift test needs this separately from [`REQUIRE_LINE`].
#[cfg(test)]
const MODULE: &str = "input";

/// The exact line inserted into `hyprland.lua`. Spelled out rather than
/// computed so it can be a `const` callers match on; a test below pins it
/// against the generic builder so the two can't drift apart.
pub const REQUIRE_LINE: &str = "require(\"hyprforge/input\")";

fn require_line_string() -> String {
    REQUIRE_LINE.to_string()
}

pub fn discover(hypr_dir: &Path) -> HyprConfig {
    lua_setup::discover(hypr_dir)
}

/// Detects whether `hyprland_lua` already sources the input file.
pub fn detect(hyprland_lua: &str) -> SetupPlan {
    lua_setup::detect(hyprland_lua, &require_line_string(), PLACEMENT)
}

/// Applies `plan`, returning the new full file contents.
pub fn apply(hyprland_lua: &str, plan: &SetupPlan) -> String {
    lua_setup::apply(hyprland_lua, plan, &require_line_string(), PLACEMENT)
}

/// Renders the line to be inserted, for the GUI's confirmation dialog.
pub fn preview_line() -> String {
    require_line_string()
}

/// Renders the file [`create_lua_config`] would write.
pub fn preview_lua_config() -> String {
    lua_setup::preview_lua_config(&require_line_string(), PLACEMENT)
}

/// Creates a minimal `hyprland.lua` sourcing the input file.
pub fn create_lua_config(path: &Path) -> Result<(), SetupError> {
    lua_setup::create_lua_config(path, &require_line_string(), PLACEMENT)
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

    /// The load-bearing one. Sourced before the user's own config, every
    /// setting saved in the app would be overwritten by their `hl.config`
    /// block and the GUI would be lying about what's running.
    #[test]
    fn the_line_goes_after_the_users_own_config() {
        let cfg = "require(\"theirs\")\nhl.config({ input = { kb_layout = \"us\" } })\n";
        let out = apply(cfg, &detect(cfg));
        let their_config = out.find("kb_layout").expect("their block should survive");
        let ours = out.find(REQUIRE_LINE).expect("our line should be inserted");
        assert!(
            ours > their_config,
            "input must be sourced after the user's own config:\n{out}"
        );
    }

    /// Window rules and input have opposite placements on purpose, so this
    /// pins them apart. If they ever became the same constant, one of the
    /// two would be silently wrong.
    #[test]
    fn input_is_placed_opposite_to_window_rules() {
        assert_eq!(PLACEMENT, Placement::AtEnd);
        assert_ne!(PLACEMENT, Placement::BeforeUserRequires);
    }

    #[test]
    fn an_already_present_line_is_not_inserted_twice() {
        let cfg = format!("{REQUIRE_LINE}\n");
        let out = apply(&cfg, &detect(&cfg));
        assert_eq!(out.matches(REQUIRE_LINE).count(), 1, "{out}");
    }
}
