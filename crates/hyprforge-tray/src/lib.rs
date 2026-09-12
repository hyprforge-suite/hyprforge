//! Tray icons for the Hyprforge suite, over `org.kde.StatusNotifierItem`.
//!
//! A tray icon is a D-Bus object, not a toolkit widget. That is what makes
//! this possible inside a suite that forbids GTK and Qt everywhere: the
//! icon is a *name*, resolved by whichever bar is running against the
//! user's own icon theme, and this crate owns no pixels at all.
//!
//! The library half knows nothing about Wi-Fi or Bluetooth — it is the
//! protocol and the item model. `src/bin/trayd.rs` is what joins it to
//! `hyprforge-network` and `hyprforge-bluetooth`.
//!
//! # What this does not do yet
//!
//! **Menus.** A right-click menu is `com.canonical.dbusmenu`, a second
//! protocol with its own object tree. Until then every click opens the
//! Settings page the icon is about, which is why `hyprforge-settings
//! --screen <name>` exists.

pub mod item;
pub mod prefs;
pub mod sni;

pub use prefs::Prefs;
pub use item::{Category, Status, TrayItem};
pub use sni::{watcher_present, TrayError, TrayIcon};
