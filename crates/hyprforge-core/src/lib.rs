pub mod module;
pub mod paths;
pub mod theme;
pub mod widgets;

#[cfg(feature = "dbus")]
pub mod displayd_proxy;

pub use module::SettingsModule;
