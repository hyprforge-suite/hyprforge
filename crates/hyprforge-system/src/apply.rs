//! Writing `system.lua` and getting Hyprland to load it.

use crate::catalog::CATALOG;
use hyprforge_core::hlconfig::{codegen, Settings};
use std::path::Path;

pub use hyprforge_core::apply_lua::ApplyError;

const SUBJECT: &str = "system settings";

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
