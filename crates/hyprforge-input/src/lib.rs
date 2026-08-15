//! Keyboard, pointer and touchpad settings.
//!
//! Almost everything here lives in [`hyprforge_core::hlconfig`], the
//! generic `hl.config` overlay machinery every settings category shares.
//! This crate is the part that is actually about input: the [`catalog`] of
//! options, and the [`setup`] require line that decides where the
//! generated file is sourced.

pub mod apply;
pub mod catalog;
pub mod setup;
pub mod xkb;

pub use catalog::CATALOG;
pub use hyprforge_core::hlconfig::{
    codegen, import, model, storage, Invalid, Kind, Setting, Settings, Value,
};
