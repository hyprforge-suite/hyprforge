pub mod geometry;
pub mod paths;

#[cfg(feature = "gui")]
pub mod module;
#[cfg(feature = "gui")]
pub mod theme;
#[cfg(feature = "gui")]
pub mod widgets;

#[cfg(feature = "dbus")]
pub mod displayd_proxy;

#[cfg(feature = "gui")]
pub use module::SettingsModule;
