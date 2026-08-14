//! Keyboard, pointer and touchpad settings, written as a `hl.config`
//! overlay the user's own config can still coexist with.
//!
//! See [`codegen`] for the two measured facts about `hl.config` the design
//! rests on, and [`setup`] for why this is the one module sourced *last*.

pub mod apply;
pub mod catalog;
pub mod codegen;
pub mod import;
pub mod model;
pub mod setup;
pub mod storage;

pub use catalog::{Kind, Setting};
pub use model::{Settings, Value};
