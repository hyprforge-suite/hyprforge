//! `photos.toml` — the state this app rewrites.
//!
//! The written half of the two-file split every app in this suite uses.
//! This file is rewritten whole on every save, which drops comments and
//! reorders tables, so nothing a person writes by hand belongs in it —
//! that is [`crate::config`]'s job.
//!
//! Three states, kept distinct, which is the rule
//! `hyprforge_core::hlconfig::storage` records and the one that once cost
//! a user 37 hand-written binds:
//!
//! - **Missing** is first run: the defaults, nothing to report.
//! - **Present and unparseable** is an error the user hears about, and is
//!   *never* silently replaced. [`update_at`]'s `?` on load is the line
//!   that enforces it — a parse failure aborts before any write.
//! - **Parseable** is used.
//!
//! `#[serde(default)]` per field, so a file written before a setting
//! existed still loads and gains that setting's default.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// What the viewer remembers between launches.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Prefs {
    /// Whether the filmstrip shows. A real off state a user chose, not
    /// "on unless we forgot to ask" — which is why the type's own
    /// `Default` is not relied on and [`Default`] below sets it
    /// explicitly.
    pub filmstrip: bool,
    /// Whether the inspector shows, docked right — mockup `2c`.
    pub info_panel: bool,
    /// Whether the Places sidebar shows — `Ctrl+B`.
    pub sidebar: bool,
    /// Seconds per picture in a slideshow; see `slideshow::Interval` for
    /// the values that exist.
    pub slideshow_seconds: u64,
    /// Whether a slideshow starts again after the last picture.
    pub slideshow_loop: bool,
    pub window_width: u32,
    pub window_height: u32,
}

impl Default for Prefs {
    fn default() -> Self {
        Prefs {
            // On, since the window became the file manager's shell
            // (mockup `2a`). It used to default off so a bare photograph
            // filled the window; now there is a sidebar and a status bar
            // around it anyway, and the strip is how the shell shows
            // that there is a folder behind the picture.
            filmstrip: true,
            info_panel: false,
            sidebar: true,
            slideshow_seconds: 5,
            slideshow_loop: false,
            window_width: 1180,
            window_height: 760,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum PrefsError {
    /// The file exists and will not parse. **Not** the same as absent.
    #[error("{path} could not be read as Photos settings: {source}")]
    Unreadable {
        path: PathBuf,
        #[source]
        source: toml::de::Error,
    },
    #[error("{path} could not be opened: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("{path} could not be written: {source}")]
    Write {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

/// Where it lives. `hyprforge-paths` owns this, the way it owns every
/// other single-file Hyprforge setting — one crate knows the layout of
/// the config directory and no other joins path segments to guess it.
pub fn path() -> PathBuf {
    hyprforge_paths::photos_toml_path()
}

pub fn load() -> Result<Prefs, PrefsError> {
    load_from(&path())
}

/// Reads the preferences, or says why it could not. A **missing** file
/// is first run and yields the defaults; a file that **exists and will
/// not parse** is an error the user has to hear about.
pub fn load_from(path: &Path) -> Result<Prefs, PrefsError> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Prefs::default()),
        Err(source) => return Err(PrefsError::Io { path: path.to_path_buf(), source }),
    };
    toml::from_str(&text)
        .map_err(|source| PrefsError::Unreadable { path: path.to_path_buf(), source })
}

pub fn save_to(path: &Path, prefs: &Prefs) -> Result<(), PrefsError> {
    let text = toml::to_string_pretty(prefs).expect("Prefs is plain data and always serialises");
    hyprforge_paths::write_atomic(path, &text)
        .map_err(|source| PrefsError::Write { path: path.to_path_buf(), source })
}

/// Read-modify-write, and what every writer should call rather than
/// saving a copy it loaded earlier.
pub fn update(f: impl FnOnce(&mut Prefs)) -> Result<Prefs, PrefsError> {
    update_at(&path(), f)
}

/// [`update`] against an arbitrary path — the seam a test uses to point
/// this at a throwaway file instead of the real one.
pub fn update_at(path: &Path, f: impl FnOnce(&mut Prefs)) -> Result<Prefs, PrefsError> {
    // The `?` here is the whole three-state rule in one character: a
    // file that will not parse fails the load, so nothing below runs and
    // the broken file is never written over.
    let mut prefs = load_from(path)?;
    f(&mut prefs);
    save_to(path, &prefs)?;
    Ok(prefs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_file_is_first_run_and_not_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let prefs = load_from(&dir.path().join("photos.toml")).unwrap();
        assert_eq!(prefs, Prefs::default());
    }

    #[test]
    fn everything_written_reads_back_as_itself() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("photos.toml");
        let prefs = Prefs {
            filmstrip: false,
            info_panel: true,
            sidebar: false,
            slideshow_seconds: 10,
            slideshow_loop: true,
            window_width: 640,
            window_height: 480,
        };
        save_to(&path, &prefs).unwrap();
        assert_eq!(load_from(&path).unwrap(), prefs);
    }

    /// The rule this file exists to obey. A corrupt file must be
    /// reported, and must still be there afterwards for the user to fix.
    #[test]
    fn a_photos_config_that_will_not_parse_is_reported_and_never_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("photos.toml");
        std::fs::write(&path, "filmstrip = yes please\n").unwrap();

        assert!(matches!(load_from(&path), Err(PrefsError::Unreadable { .. })));

        let before = std::fs::read_to_string(&path).unwrap();
        assert!(update_at(&path, |p| p.filmstrip = true).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), before, "the file was written over");
    }

    /// A file written before a setting existed still loads, and gains
    /// that setting's default rather than failing.
    #[test]
    fn a_file_written_before_a_setting_existed_still_parses() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("photos.toml");
        std::fs::write(&path, "window_width = 800\n").unwrap();
        let prefs = load_from(&path).unwrap();
        assert_eq!(prefs.window_width, 800);
        assert_eq!(prefs.info_panel, Prefs::default().info_panel);
    }

    /// Read-modify-write, not save-what-I-loaded: a change made by
    /// another window is not undone by this one saving.
    #[test]
    fn updating_keeps_a_change_another_writer_made() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("photos.toml");
        save_to(&path, &Prefs::default()).unwrap();

        // Somebody else resizes their window...
        update_at(&path, |p| p.window_width = 1234).unwrap();
        // ...and this writer, holding a stale copy, changes something else.
        let updated = update_at(&path, |p| p.info_panel = true).unwrap();

        assert_eq!(updated.window_width, 1234);
        assert!(updated.info_panel);
    }
}
