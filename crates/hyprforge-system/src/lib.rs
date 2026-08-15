//! Behaviour and platform settings: the `hl.config` categories that
//! aren't about how things look.
//!
//! Everything here is [`hyprforge_core::hlconfig`] — this crate is the
//! [`catalog`] and the require line, nothing more. `misc`, `binds`,
//! `xwayland`, `opengl`, `render`, `ecosystem` and `quirks`.
//!
//! `render` in particular is where the settings that can make the screen
//! stop working live. They are catalogued rather than hidden, because a
//! user who needs `direct_scanout` off has no other way to reach it from
//! a GUI — but the ones that bite are marked in their help text.

pub mod apply;
pub mod catalog;
pub mod setup;

pub use catalog::CATALOG;
pub use hyprforge_core::hlconfig::{
    codegen, import, model, storage, Invalid, Kind, Setting, Settings, Value,
};
