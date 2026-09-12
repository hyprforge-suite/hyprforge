//! The Hypr ecosystem daemons' settings: wallpaper, colour temperature,
//! and idle behaviour.
//!
//! These are not Hyprland itself. Each is a separate daemon with its own
//! hyprlang config file, so none of them goes through `hl.config` or
//! `require()` — the shared plumbing is [`hyprforge_core::hyprlang`],
//! which owns one generated file per daemon and joins it to the user's
//! own config with a single `source =` line.
//!
//! How a change reaches a running daemon differs, and the difference is
//! visible to the user:
//!
//! - **hyprpaper** takes `hyprctl hyprpaper wallpaper` live.
//! - **hyprsunset** takes `hyprctl hyprsunset temperature` live.
//! - **hypridle** has no IPC at all (`hyprctl hypridle` → "unknown
//!   request") and has to be restarted for a change to take effect.

pub mod apply;
pub mod import;
pub mod idle;
pub mod portal;
pub mod storage;
pub mod sunset;
pub mod sunset_control;
pub mod wallpaper;
