//! Writing `input.lua` and getting Hyprland to load it.
//!
//! The sequence — write, reload, check three ways, roll back if refused —
//! lives in [`hyprforge_core::apply_lua`], shared with shortcuts and
//! window rules. This crate contributes only what to generate and what an
//! empty set looks like.

use crate::catalog::CATALOG;
use hyprforge_core::hlconfig::{codegen, Settings};
use std::path::Path;

pub use hyprforge_core::apply_lua::ApplyError;

/// Named in the generated file's header and in the apply/rollback
/// messages, so a user who meets either knows which screen owns it.
const SUBJECT: &str = "input settings";

pub fn generate(settings: &Settings) -> String {
    codegen::generate(settings, &CATALOG, SUBJECT)
}

pub fn apply(lua_path: &Path, settings: &Settings) -> Result<(), ApplyError> {
    hyprforge_core::apply_lua::apply(hyprforge_core::apply_lua::GeneratedFile {
        path: lua_path,
        contents: &generate(settings),
        empty: &generate(&Settings::default()),
        subject: SUBJECT,
    })
}
