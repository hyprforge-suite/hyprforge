//! One screen for how the desktop looks.
//!
//! Three things a user thinks of as one decision, which the system keeps
//! in three unrelated places:
//!
//! - **Hyprland settings** — borders, gaps, rounding, blur, shadows,
//!   cursor. An `hl.config` overlay, handled by
//!   [`hyprforge_core::hlconfig`]; this crate supplies only the
//!   [`catalog`].
//! - **[`animations`]** — speed and curve per animation. Looks
//!   append-shaped but is last-wins per leaf, so it's the same overlay in
//!   the same generated file.
//! - **[`desktop`]** — the GTK/icon/cursor/font/dark-mode settings that
//!   live in gsettings, which no file here generates and which owns
//!   itself differently. See that module for why.

pub mod animations;
pub mod apply;
pub mod catalog;
pub mod desktop;
pub mod look;
pub mod setup;
pub mod storage;
pub mod themes;

pub use catalog::CATALOG;
pub use storage::Appearance;
