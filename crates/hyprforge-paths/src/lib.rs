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

/// Which tray icons the user wants, written by Settings and re-read by
/// `hyprforge-trayd` on every poll so a toggle takes effect without
/// restarting anything.
pub fn tray_toml_path() -> PathBuf {
    hyprforge_config_dir().join("tray.toml")
}

/// Where the clipboard manager keeps its history. Its own subdirectory,
/// not a file directly under `hyprforge_config_dir()`, because the index
/// is one file but image content is one file *per entry* — see
/// [`clipboard_images_dir`] — and both need a directory to live in.
pub fn clipboard_dir() -> PathBuf {
    hyprforge_config_dir().join("clipboard")
}

/// The clipboard history's index: ids, timestamps, pins, text content and
/// image *references*. Never image bytes themselves — see
/// [`clipboard_images_dir`] for why those live elsewhere.
pub fn clipboard_index_path() -> PathBuf {
    clipboard_dir().join("history.toml")
}

/// Where clipboard image bytes are kept, one file per entry, named by its
/// content hash. Inlining a screenshot's bytes into the index as base64
/// would make that file unreadable by eye, unparseable at any real size,
/// and would mean rewriting the whole index — text entries included —
/// every time a single image is added or evicted.
pub fn clipboard_images_dir() -> PathBuf {
    clipboard_dir().join("images")
}

/// The path for one clipboard entry's image bytes, keyed by its
/// `EntryId` (as returned by `EntryId::as_str`) so the index and the
/// file agree on which entry a file belongs to without either side
/// having to store the other's full path.
pub fn clipboard_image_path(id: &str) -> PathBuf {
    clipboard_images_dir().join(format!("{id}.bin"))
}

/// Write `contents` to `path` atomically and durably: write a sibling temp
/// file, flush it to disk, rename over the target, then flush the
/// directory.
///
/// The rename alone makes the change atomic *for a concurrent reader* —
/// nobody ever sees half a file. It does not make it survive a power
/// loss, and those are two different guarantees that are easy to
/// conflate. Without the first fsync the rename can land while the new
/// file's contents are still only in page cache, so the file comes back
/// empty; without the second the rename itself can be lost, so the file
/// comes back missing or with its previous contents.
///
/// The cost is one flush per config write. These files are written when
/// a user changes a setting or a layout settles, not in a loop, and what
/// is being protected is the only record of something the user cannot
/// reconstruct — their profiles, their keybinds.
pub fn write_atomic(path: &std::path::Path, contents: &str) -> std::io::Result<()> {
    write_atomic_bytes(path, contents.as_bytes())
}

/// The same guarantee as [`write_atomic`], for content that is not text —
/// clipboard image bytes, specifically. Kept as a second entry point
/// rather than making [`write_atomic`] generic over `AsRef<[u8]>`. so
/// every existing caller (all of which pass `&str`) keeps its exact
/// signature.
pub fn write_atomic_bytes(path: &std::path::Path, contents: &[u8]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    // Unique per call, not a fixed `<name>.tmp`. Two processes write some
    // of these files — `monitors.lua` has both the display daemon and the
    // Settings app as writers — and with one shared temp name the second
    // `create` truncates the first writer's half-written file, so the
    // bytes that get renamed into place can be an interleaving of both.
    // The fsync below then commits that corruption to disk.
    //
    // Process id and a counter rather than randomness: no dependency, and
    // the only collision that matters is between concurrent writers,
    // which these distinguish. A leftover from a previous boot with a
    // recycled pid is cleaned up below rather than reused.
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let unique = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let stem = path.file_name().and_then(|n| n.to_str()).unwrap_or("hyprforge");
    let tmp = path.with_file_name(format!(".{stem}.{}.{unique}.tmp", std::process::id()));

    // Removes the temp file unless the rename claimed it. Without this a
    // full disk leaves one behind on every failed save, accumulating in
    // the user's config directory — and a leftover is not inert, because
    // it is exactly what the next writer would have reused.
    struct Cleanup<'a>(Option<&'a std::path::Path>);
    impl Drop for Cleanup<'_> {
        fn drop(&mut self) {
            if let Some(path) = self.0 {
                let _ = std::fs::remove_file(path);
            }
        }
    }
    let mut cleanup = Cleanup(Some(&tmp));

    {
        let file = std::fs::File::create(&tmp)?;
        {
            use std::io::Write as _;
            let mut writer = std::io::BufWriter::new(&file);
            writer.write_all(contents)?;
            writer.flush()?;
        }
        file.sync_all()?;
    }

    std::fs::rename(&tmp, path)?;
    // The rename consumed it; there is nothing left to remove, and the
    // name now belongs to whatever a later writer creates.
    cleanup.0 = None;

    // Flushing the directory is what makes the rename itself durable.
    // Best-effort: some filesystems refuse to open a directory for this,
    // and failing the whole write because the *extra* guarantee could not
    // be had would be worse than the guarantee being missing — the data
    // is already on disk and already renamed into place.
    if let Some(parent) = path.parent() {
        if let Ok(dir) = std::fs::File::open(parent) {
            let _ = dir.sync_all();
        }
    }
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

    /// The failure path, which the success-only test above never
    /// reaches. A full disk or a quota'd home leaves the error to the
    /// caller — and must not also leave debris in the user's config
    /// directory, one file per failed save.
    #[test]
    fn a_failed_write_leaves_no_temp_file_behind() {
        let dir = tempfile::tempdir().unwrap();
        // A directory where the file should go: `File::create` fails, so
        // the error arrives after the temp path has been chosen.
        let target = dir.path().join("occupied.toml");
        std::fs::create_dir(&target).unwrap();

        assert!(write_atomic(&target, "anything").is_err());

        let leftovers: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|name| name.ends_with(".tmp"))
            .collect();
        assert!(leftovers.is_empty(), "left behind: {leftovers:?}");
    }

    /// Two writers to one path must not share a temp file. `monitors.lua`
    /// really does have two: the display daemon and the Settings app.
    /// With one fixed `<name>.tmp` the second `create` truncates the
    /// first's in-progress file, and what gets renamed into place can be
    /// an interleaving of both.
    #[test]
    fn concurrent_writers_to_one_path_never_share_a_temp_file() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("monitors.lua");

        let a = std::thread::spawn({
            let target = target.clone();
            move || {
                for _ in 0..50 {
                    write_atomic(&target, &"a".repeat(4096)).unwrap();
                }
            }
        });
        let b = std::thread::spawn({
            let target = target.clone();
            move || {
                for _ in 0..50 {
                    write_atomic(&target, &"b".repeat(4096)).unwrap();
                }
            }
        });
        a.join().unwrap();
        b.join().unwrap();

        // Whichever writer landed last, the file must be entirely one of
        // them — never a mixture, and never short.
        let got = std::fs::read_to_string(&target).unwrap();
        assert!(
            got == "a".repeat(4096) || got == "b".repeat(4096),
            "torn write: {} bytes, starts {:?}",
            got.len(),
            &got[..got.len().min(8)]
        );
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

    #[test]
    fn clipboard_paths_hang_off_their_own_subdirectory() {
        let _g = EnvGuard::set(Some("/custom/config"), Some("/home/someone"));
        assert_eq!(
            clipboard_dir(),
            PathBuf::from("/custom/config/hyprforge/clipboard")
        );
        assert_eq!(
            clipboard_index_path(),
            PathBuf::from("/custom/config/hyprforge/clipboard/history.toml")
        );
        assert_eq!(
            clipboard_images_dir(),
            PathBuf::from("/custom/config/hyprforge/clipboard/images")
        );
        assert_eq!(
            clipboard_image_path("abc123"),
            PathBuf::from("/custom/config/hyprforge/clipboard/images/abc123.bin")
        );
    }

    #[test]
    fn write_atomic_bytes_round_trips_binary_content() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("image.bin");
        let bytes: Vec<u8> = (0..=255).collect();
        write_atomic_bytes(&path, &bytes).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
    }
}
