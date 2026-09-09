//! What every Hyprforge app looks like.
//!
//! One crate rather than one per app, because the alternative already
//! happened: the Settings app and the lock screen each grew their own
//! palette and drifted to different accents, different backgrounds and
//! different text colours — two products from one project, which is the
//! exact failure this suite exists to avoid.
//!
//! Deliberately free of any GUI toolkit. The lock screen and the greeter
//! paint into a raw Wayland buffer, so anything they cannot use is not
//! actually shared. The iced side converts at its own boundary.

pub mod color;
pub mod theme;


pub use color::{Color, ColorError};
pub use theme::{Surfaces, Theme, ThemeError};
