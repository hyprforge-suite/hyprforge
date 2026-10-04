//! Every path setup reads or writes, from one place.
//!
//! Explicit rather than read from `$XDG_CONFIG_HOME` at each call, so a
//! test can point the whole of setup at a temp directory without touching
//! the process environment — and so no test can reach the real
//! `~/.config` by forgetting to set a variable. [`Env::from_environment`]
//! is the one constructor that reads the environment, and a test pins it
//! to the same answers the modules that own each file give.

use std::path::{Path, PathBuf};

/// The directories setup works in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Env {
    /// `$XDG_CONFIG_HOME`.
    pub config_home: PathBuf,
    /// `$XDG_DATA_HOME`.
    pub data_home: PathBuf,
    /// Where desktop entries and the MIME database are read from: the
    /// data home first, then `$XDG_DATA_DIRS`.
    pub data_dirs: Vec<PathBuf>,
    /// Every `mimeapps.list` that applies, most specific first.
    pub mimeapps_paths: Vec<PathBuf>,
}

impl Env {
    /// The directories of the session this process runs in.
    pub fn from_environment() -> Env {
        Env {
            config_home: hyprforge_paths::config_home(),
            data_home: hyprforge_paths::data_home(),
            data_dirs: hyprforge_mime::data_dirs(),
            mimeapps_paths: hyprforge_mime::mimeapps_paths(),
        }
    }

    /// Everything under one directory: `<root>/config` and
    /// `<root>/data`, with only those two consulted for applications and
    /// defaults. For tests, so nothing on the machine leaks in.
    pub fn rooted_at(root: &Path) -> Env {
        let config_home = root.join("config");
        let data_home = root.join("data");
        Env {
            mimeapps_paths: vec![config_home.join("mimeapps.list")],
            data_dirs: vec![data_home.clone()],
            config_home,
            data_home,
        }
    }

    /// `hypr/` — the user's Hyprland config directory.
    pub fn hypr_dir(&self) -> PathBuf {
        self.config_home.join("hypr")
    }

    pub fn hyprland_lua(&self) -> PathBuf {
        self.hypr_dir().join("hyprland.lua")
    }

    /// `hypr/hyprforge/`, where each module's generated Lua lives.
    pub fn generated_dir(&self) -> PathBuf {
        self.hypr_dir().join("hyprforge")
    }

    /// `hyprforge/`, where the canonical TOML lives.
    pub fn hyprforge_dir(&self) -> PathBuf {
        self.config_home.join("hyprforge")
    }

    /// What setup changed, so it can be undone — see [`crate::record`].
    pub fn setup_toml(&self) -> PathBuf {
        self.hyprforge_dir().join("setup.toml")
    }

    pub fn shortcuts_toml(&self) -> PathBuf {
        self.hyprforge_dir().join("shortcuts.toml")
    }

    pub fn keybinds_lua(&self) -> PathBuf {
        self.generated_dir().join("keybinds.lua")
    }

    pub fn window_rules_toml(&self) -> PathBuf {
        self.hyprforge_dir().join("window-rules.toml")
    }

    pub fn window_rules_lua(&self) -> PathBuf {
        self.generated_dir().join("window-rules.lua")
    }

    pub fn input_lua(&self) -> PathBuf {
        self.generated_dir().join("input.lua")
    }

    pub fn system_lua(&self) -> PathBuf {
        self.generated_dir().join("system.lua")
    }

    pub fn appearance_lua(&self) -> PathBuf {
        self.generated_dir().join("appearance.lua")
    }

    pub fn session_toml(&self) -> PathBuf {
        self.hyprforge_dir().join("session.toml")
    }

    pub fn session_lua(&self) -> PathBuf {
        self.generated_dir().join("session.lua")
    }

    pub fn idle_toml(&self) -> PathBuf {
        self.hyprforge_dir().join("idle.toml")
    }

    /// The generated hypridle file.
    pub fn idle_conf(&self) -> PathBuf {
        self.generated_dir().join("idle.conf")
    }

    /// hypridle's own config, which sources [`Env::idle_conf`].
    pub fn hypridle_conf(&self) -> PathBuf {
        self.hypr_dir().join("hypridle.conf")
    }

    /// The one `mimeapps.list` this suite writes.
    pub fn mimeapps_list(&self) -> PathBuf {
        self.config_home.join("mimeapps.list")
    }

    /// The portal's per-desktop preferences. The portal reads only the
    /// *first* `hyprland-portals.conf` it finds, so a user file here
    /// replaces `/usr/share`'s outright — which is why the item keeps a
    /// `default=` line in it.
    pub fn portals_conf(&self) -> PathBuf {
        self.config_home.join("xdg-desktop-portal").join("hyprland-portals.conf")
    }

    /// A user's D-Bus service file for `org.freedesktop.FileManager1`,
    /// read before `/usr/share`'s — which is how it wins over Nemo's.
    pub fn file_manager1_service(&self) -> PathBuf {
        self.data_home
            .join("dbus-1")
            .join("services")
            .join("org.freedesktop.FileManager1.service")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The real constructor must land on exactly the files the modules
    /// that own them use, or setup and the Settings page would be editing
    /// two different copies. Both sides read the same environment here,
    /// so nothing is set or unset.
    #[test]
    fn the_real_paths_are_the_ones_each_module_owns() {
        let env = Env::from_environment();
        assert_eq!(env.hyprland_lua(), hyprforge_core::paths::hyprland_lua_path());
        assert_eq!(env.shortcuts_toml(), hyprforge_core::paths::shortcuts_toml_path());
        assert_eq!(env.keybinds_lua(), hyprforge_core::paths::keybinds_lua_path());
        assert_eq!(env.window_rules_toml(), hyprforge_core::paths::window_rules_toml_path());
        assert_eq!(env.window_rules_lua(), hyprforge_core::paths::window_rules_lua_path());
        assert_eq!(env.input_lua(), hyprforge_core::paths::input_lua_path());
        assert_eq!(env.idle_toml(), hyprforge_ecosystem::idle::settings_path());
        assert_eq!(env.idle_conf(), hyprforge_ecosystem::idle::generated_path());
        assert_eq!(env.hypridle_conf(), hyprforge_ecosystem::idle::hypridle_conf_path());
        assert_eq!(env.mimeapps_list(), hyprforge_mime::user_mimeapps_path());
        assert_eq!(env.generated_dir(), hyprforge_core::paths::hypr_hyprforge_dir());
        assert_eq!(env.hyprforge_dir(), hyprforge_core::paths::hyprforge_config_dir());
    }

    #[test]
    fn a_rooted_env_reaches_nothing_outside_its_root() {
        let root = Path::new("/tmp/somewhere");
        let env = Env::rooted_at(root);
        for path in [
            env.setup_toml(),
            env.portals_conf(),
            env.file_manager1_service(),
            env.mimeapps_list(),
            env.hypridle_conf(),
        ] {
            assert!(path.starts_with(root), "{}", path.display());
        }
        assert!(env.data_dirs.iter().chain(&env.mimeapps_paths).all(|p| p.starts_with(root)));
    }
}
