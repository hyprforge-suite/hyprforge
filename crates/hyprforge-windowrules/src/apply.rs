//! Writing `window-rules.lua` and getting Hyprland to load it.
//!
//! The sequence itself — write, reload, check three ways, roll back if
//! refused — lives in [`hyprforge_core::apply_lua`], shared with shortcuts.
//! This crate contributes only what's specific to window rules: what to
//! generate, and what an empty set of them looks like.

use crate::codegen::{generate, generate_all};
use crate::storage::Rules;
use std::path::Path;

pub use hyprforge_core::apply_lua::ApplyError;

pub fn apply(lua_path: &Path, rules: &Rules) -> Result<(), ApplyError> {
    hyprforge_core::apply_lua::apply(hyprforge_core::apply_lua::GeneratedFile {
        path: lua_path,
        contents: &generate_all(rules),
        empty: &generate(&[], &[]),
        subject: "rules",
    })
}
