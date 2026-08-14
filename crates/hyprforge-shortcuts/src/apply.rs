//! Writing `keybinds.lua` and getting Hyprland to load it.
//!
//! The sequence itself — write, reload, check three ways, roll back if
//! refused — lives in [`hyprforge_core::apply_lua`], shared with window
//! rules. This crate contributes only what's specific to shortcuts: what to
//! generate, and what an empty set of them looks like.

use crate::codegen::generate;
use crate::model::Shortcut;
use std::path::Path;

pub use hyprforge_core::apply_lua::ApplyError;

pub fn apply(lua_path: &Path, shortcuts: &[Shortcut]) -> Result<(), ApplyError> {
    hyprforge_core::apply_lua::apply(hyprforge_core::apply_lua::GeneratedFile {
        path: lua_path,
        contents: &generate(shortcuts),
        empty: &generate(&[]),
        subject: "shortcuts",
    })
}
