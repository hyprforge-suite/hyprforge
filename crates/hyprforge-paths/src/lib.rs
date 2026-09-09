//! Where Hyprforge keeps things, and how it writes them.
//!
//! Every app in the suite needs these two answers and nothing else from
//! them: which directory is mine, and how do I replace a file without
//! ever leaving a half-written one on disk. Neither question has
//! anything to do with Hyprland, a GUI toolkit, or Lua, so neither does
//! this crate — it has no dependencies at all.
//!
//! Paths that *are* Hyprland-specific (`hypr/hyprforge/*.lua`, the
//! generated config artifacts) live in `hyprforge-core` instead.

use std::path::PathBuf;

/// `$XDG_CONFIG_HOME`, falling back to `~/.config`.
pub fn config_home() -> PathBuf {
    if let Some(dir) = std::env::var_os("XDG_CONFIG_HOME") {
        let dir = PathBuf::from(dir);
        if dir.is_absolute() {
            return dir;
        }
    }
    let home = std::env::var_os("HOME").expect("HOME must be set");
    PathBuf::from(home).join(".config")
}


/// `$XDG_CONFIG_HOME/hyprforge` — canonical storage for all Hyprforge-owned
/// config (display profiles, window-rules TOML).
pub fn hyprforge_config_dir() -> PathBuf {
    config_home().join("hyprforge")
}

/// The lock screen's theme, written by Settings and read by
/// `hyprforge-lock`. The greeter cannot use this one — it runs as its own
/// user and a home directory is `drwx------` — so it reads an exported
/// copy instead.
pub fn lock_toml_path() -> PathBuf {
    hyprforge_config_dir().join("lock.toml")
}

/// The Appearance module's stored settings, read when resolving the
/// shared theme.
pub fn appearance_toml_path() -> PathBuf {
    hyprforge_config_dir().join("appearance.toml")
}

/// Write `contents` to `path` atomically: write to a sibling temp file, then
/// rename over the target. Avoids ever leaving a half-written config file.
pub fn write_atomic(path: &std::path::Path, contents: &str) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension(format!(
        "{}.tmp",
        path.extension().and_then(|e| e.to_str()).unwrap_or("")
    ));
    std::fs::write(&tmp, contents)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Mutex, MutexGuard};

    /// `set_var`/`remove_var` are process-global and Rust runs tests on
    /// threads, so every test that touches the environment has to take this
    /// first or they corrupt each other's reads.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    /// Sets the two variables `config_home` reads, restoring whatever was
    /// there when the guard drops — otherwise a test would leak its fake
    /// `$HOME` into every test that runs after it.
    struct EnvGuard {
        _lock: MutexGuard<'static, ()>,
        xdg: Option<std::ffi::OsString>,
        home: Option<std::ffi::OsString>,
    }

    impl EnvGuard {
        fn set(xdg: Option<&str>, home: Option<&str>) -> Self {
            let guard = EnvGuard {
                _lock: ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner()),
                xdg: std::env::var_os("XDG_CONFIG_HOME"),
                home: std::env::var_os("HOME"),
            };
            // SAFETY: ENV_LOCK serializes every env mutation in this module.
            unsafe {
                match xdg {
                    Some(v) => std::env::set_var("XDG_CONFIG_HOME", v),
                    None => std::env::remove_var("XDG_CONFIG_HOME"),
                }
                match home {
                    Some(v) => std::env::set_var("HOME", v),
                    None => std::env::remove_var("HOME"),
                }
            }
            guard
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            unsafe {
                match &self.xdg {
                    Some(v) => std::env::set_var("XDG_CONFIG_HOME", v),
                    None => std::env::remove_var("XDG_CONFIG_HOME"),
                }
                match &self.home {
                    Some(v) => std::env::set_var("HOME", v),
                    None => std::env::remove_var("HOME"),
                }
            }
        }
    }

    #[test]
    fn absolute_xdg_config_home_is_used_as_is() {
        let _g = EnvGuard::set(Some("/custom/config"), Some("/home/someone"));
        assert_eq!(config_home(), PathBuf::from("/custom/config"));
    }

    #[test]
    fn unset_xdg_config_home_falls_back_to_home_dot_config() {
        let _g = EnvGuard::set(None, Some("/home/someone"));
        assert_eq!(config_home(), PathBuf::from("/home/someone/.config"));
    }

    #[test]
    fn relative_xdg_config_home_is_ignored_per_the_xdg_spec() {
        // The spec says a relative value is invalid and must be treated as
        // unset — not resolved against the cwd, which would scatter config
        // wherever the app happened to be launched from.
        let _g = EnvGuard::set(Some("relative/path"), Some("/home/someone"));
        assert_eq!(config_home(), PathBuf::from("/home/someone/.config"));
    }

    #[test]
    fn empty_xdg_config_home_falls_back_too() {
        let _g = EnvGuard::set(Some(""), Some("/home/someone"));
        assert_eq!(config_home(), PathBuf::from("/home/someone/.config"));
    }

    #[test]
    fn write_atomic_creates_parent_directories() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a").join("b").join("file.toml");
        write_atomic(&path, "hello").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "hello");
    }

    #[test]
    fn write_atomic_overwrites_and_leaves_no_temp_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("file.toml");
        write_atomic(&path, "first").unwrap();
        write_atomic(&path, "second").unwrap();

        assert_eq!(std::fs::read_to_string(&path).unwrap(), "second");
        let leftovers: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .filter(|n| n.to_string_lossy().contains(".tmp"))
            .collect();
        assert!(leftovers.is_empty(), "temp file left behind: {leftovers:?}");
    }

    #[test]
    fn write_atomic_handles_an_extensionless_path() {
        // `with_extension` on a name with no extension is the easy way to
        // accidentally produce a temp path that collides with the target.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("noext");
        write_atomic(&path, "body").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "body");
    }
    #[test]
    fn the_app_config_files_hang_off_the_hyprforge_dir() {
        let _g = EnvGuard::set(Some("/custom/config"), Some("/home/someone"));
        assert_eq!(
            lock_toml_path(),
            PathBuf::from("/custom/config/hyprforge/lock.toml")
        );
        assert_eq!(
            appearance_toml_path(),
            PathBuf::from("/custom/config/hyprforge/appearance.toml")
        );
    }
}
