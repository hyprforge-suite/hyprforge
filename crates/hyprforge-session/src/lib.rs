//! The parts of a Hyprland session that aren't windows or appearance:
//! what starts with it, what environment it runs in, what the touchpad
//! does, and what applications are allowed to do.
//!
//! All four are `hl.*` calls in the user's Lua config, so they share the
//! Lua plumbing the other modules use — one generated file, one
//! `require()` line, placed last.
//!
//! [`gestures`] is the exception worth knowing about: Hyprland *refuses*
//! a gesture that duplicates an earlier one instead of overriding it, so
//! that module detects conflicts before writing rather than relying on
//! last-one-wins.

pub mod apply;
pub mod autostart;
pub mod environment;
pub mod gestures;
pub mod permissions;
pub mod setup;
pub mod storage;
