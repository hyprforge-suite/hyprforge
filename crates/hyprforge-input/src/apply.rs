//! Writing `input.lua` and getting Hyprland to load it.
//!
//! The sequence — write, reload, check three ways, roll back if refused —
//! lives in [`hyprforge_core::apply_lua`], shared with shortcuts and window
//! rules. This crate contributes only what to generate and what an empty
//! set looks like.

use crate::codegen::generate;
use crate::model::Settings;
use std::path::Path;

pub use hyprforge_core::apply_lua::ApplyError;

pub fn apply(lua_path: &Path, settings: &Settings) -> Result<(), ApplyError> {
    hyprforge_core::apply_lua::apply(hyprforge_core::apply_lua::GeneratedFile {
        path: lua_path,
        contents: &generate(settings),
        empty: &generate(&Settings::default()),
        subject: "input settings",
    })
}
