//! What every Hyprforge app looks like.
//!
//! One crate rather than one per app, because the alternative already
//! happened: the Settings app and the lock screen each grew their own
//! palette and drifted to different accents, different backgrounds and
//! different text colours — two products from one project, which is the
//! exact failure this suite exists to avoid.
//!
//! Deliberately free of any GUI toolkit. Some of its users have no window
//! at all — `hyprforge-core` reads colours out of a Hyprland config and
//! `hyprforge-appearance` resolves and publishes the theme — and a toolkit
//! here would be carried by each of them for nothing. The rule began with
//! the lock screen painting into a raw Wayland buffer with no toolkit;
//! the lock screen and greeter both draw through iced's software renderer
//! now, and like every other iced host they convert at their own boundary
//! (`hyprforge_ui::color`).

pub mod color;
pub mod theme;


pub use color::{Color, ColorError};
pub use theme::{Surfaces, Theme, ThemeError};
