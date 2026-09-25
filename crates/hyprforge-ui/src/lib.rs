//! What Hyprforge apps are built out of.
//!
//! The iced half of the shared look: one spacing scale, one palette, one
//! set of widgets, so a sidebar in Settings and a sidebar in the file
//! manager breathe the same amount and use the same accent.
//!
//! Deliberately knows nothing about Hyprland. A calculator should be
//! able to use these widgets without inheriting Lua codegen, which is
//! why the one widget that *did* know — the "your config can't take a
//! require line" banner — lives in the Settings app instead.

pub mod color;
pub mod density;
pub mod keys;
pub mod theme;
pub mod widgets;
