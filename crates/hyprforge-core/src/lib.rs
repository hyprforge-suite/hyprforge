pub mod apply_lua;
pub mod geometry;
pub mod hlconfig;
pub mod hyprlang;
pub mod lua;
pub mod monitors;
pub mod lua_setup;
pub mod paths;
pub mod supersede;

#[cfg(feature = "dbus")]
pub mod displayd_proxy;

/// Re-exported for compatibility only: `command` used to live here, but
/// running a subprocess with a bound has nothing to do with Hyprland
/// config machinery, so it moved to `hyprforge-process` — a leaf crate
/// with no workspace dependencies, usable by clients of other daemons
/// (NetworkManager, BlueZ, systemd-logind) that have no business pulling
/// in this crate. Existing call sites spell it `hyprforge_core::command`,
/// and churning about twenty of them for no behavioural change is not
/// worth it, so it stays reachable from here. New code should depend on
/// `hyprforge-process` directly instead of routing through this crate.
pub use hyprforge_process as command;

