//! Which tray icons the user wants.
//!
//! Its own file rather than a section of something larger, because two
//! processes read it: the Settings app writes it, and `hyprforge-trayd`
//! re-reads it on every poll so a toggle takes effect without restarting
//! anything.
//!
//! Defaults are "show both". A tray icon the user never asked to hide is
//! the reason they installed a tray daemon.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Prefs {
    pub network: bool,
    pub bluetooth: bool,
}

impl Default for Prefs {
    fn default() -> Self {
        Prefs {
            network: true,
            bluetooth: true,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum PrefsError {
    /// The file exists and will not parse. **Not** the same as absent —
    /// see [`load_from`].
    #[error("{path} could not be read as tray settings: {source}")]
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

/// Where it lives. `hyprforge-paths` owns this, the way it owns
/// `lock.toml` and `appearance.toml` — one crate knows the layout of the
/// config directory, and no other crate joins path segments to guess it.
pub fn path() -> PathBuf {
    hyprforge_paths::tray_toml_path()
}

pub fn load() -> Result<Prefs, PrefsError> {
    load_from(&path())
}

/// Reads the preferences, or says why it could not.
///
/// A **missing** file is first run and yields the defaults. A file that
/// **exists and will not parse** is an error the user has to hear about,
/// and must never be silently replaced with defaults — doing that would
/// turn a typo into "you have configured nothing" and then overwrite what
/// they wrote on the next save. This suite has paid for that mistake
/// once already, in `hlconfig::storage`, and the rule is in CLAUDE.md.
pub fn load_from(path: &Path) -> Result<Prefs, PrefsError> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Prefs::default()),
        Err(source) => {
            return Err(PrefsError::Io {
                path: path.to_path_buf(),
                source,
            })
        }
    };
    toml::from_str(&text).map_err(|source| PrefsError::Unreadable {
        path: path.to_path_buf(),
        source,
    })
}

pub fn save(prefs: &Prefs) -> Result<(), PrefsError> {
    save_to(&path(), prefs)
}

pub fn save_to(path: &Path, prefs: &Prefs) -> Result<(), PrefsError> {
    let text = toml::to_string_pretty(prefs).expect("Prefs is two bools and always serialises");
    hyprforge_paths::write_atomic(path, &text).map_err(|source| PrefsError::Write {
        path: path.to_path_buf(),
        source,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn both_icons_are_shown_until_someone_says_otherwise() {
        let prefs = Prefs::default();
        assert!(prefs.network);
        assert!(prefs.bluetooth);
    }

    #[test]
    fn a_missing_file_is_first_run_not_a_failure() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("tray.toml");
        assert_eq!(load_from(&missing).unwrap(), Prefs::default());
    }

    /// The rule this file is most likely to break. A tray.toml with a
    /// typo in it must be reported, not silently treated as "nothing
    /// configured" — because the next save would then overwrite what the
    /// user actually wrote.
    #[test]
    fn a_file_that_exists_and_will_not_parse_is_reported_rather_than_defaulted() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tray.toml");
        std::fs::write(&path, "network = yes please\n").unwrap();

        let err = load_from(&path).expect_err("a malformed file is an error, not defaults");
        assert!(matches!(err, PrefsError::Unreadable { .. }));
        assert!(err.to_string().contains("tray.toml"));
    }

    /// A file naming only one icon leaves the other at its default,
    /// rather than switching it off — `#[serde(default)]` per field is
    /// what makes adding a third icon later not break existing files.
    #[test]
    fn a_partial_file_leaves_the_icons_it_does_not_mention_alone() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tray.toml");
        std::fs::write(&path, "network = false\n").unwrap();

        let prefs = load_from(&path).unwrap();
        assert!(!prefs.network);
        assert!(prefs.bluetooth, "an unmentioned icon keeps its default");
    }

    #[test]
    fn what_is_saved_is_what_comes_back() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tray.toml");
        let prefs = Prefs {
            network: false,
            bluetooth: true,
        };
        save_to(&path, &prefs).unwrap();
        assert_eq!(load_from(&path).unwrap(), prefs);
    }
}
