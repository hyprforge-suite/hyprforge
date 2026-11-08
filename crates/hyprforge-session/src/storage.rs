//! The canonical `session.toml`.

use crate::{autostart, environment, gestures, permissions};
use serde::{Deserialize, Serialize};
use std::path::Path;

pub use hyprforge_core::hlconfig::storage::StorageError;

/// Everything the Session screen owns, in one file — four small lists
/// that are one screen and one generated Lua file.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Session {
    #[serde(default)]
    pub autostart: autostart::Settings,
    #[serde(default)]
    pub environment: environment::Settings,
    #[serde(default)]
    pub gestures: gestures::Settings,
    #[serde(default)]
    pub permissions: permissions::Settings,
}

impl Session {
    pub fn is_empty(&self) -> bool {
        self.autostart.is_empty()
            && self.environment.is_empty()
            && self.gestures.is_empty()
            && self.permissions.is_empty()
    }
}

/// A missing file means "nothing configured yet". A file that exists but
/// can't be read **is** an error — collapsing that into an empty set is
/// what cost a real user 37 hand-written binds.
pub fn load(path: &Path) -> Result<Session, StorageError> {
    // Read first and ask what went wrong, rather than `path.exists()`
    // first: that is false for anything it cannot stat (no permission on
    // a directory above it, an I/O error), and each of those read as a
    // first run whose next save replaced the file. Only `NotFound` is.
    let contents = match std::fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Session::default()),
        Err(source) => {
            return Err(StorageError::Read { path: path.display().to_string(), source });
        }
    };
    toml::from_str(&contents).map_err(|source| StorageError::Parse {
        path: path.display().to_string(),
        source,
    })
}

pub fn save(path: &Path, session: &Session) -> Result<(), StorageError> {
    let contents = toml::to_string_pretty(session)?;
    hyprforge_core::paths::write_atomic(path, &contents).map_err(|source| StorageError::Write {
        path: path.display().to_string(),
        source,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Session {
        let mut session = Session::default();
        session.autostart.programs.push(autostart::Program {
            command: "waybar".into(),
            enabled: true,
            when: autostart::When::Start,
            note: "status bar".into(),
        });
        session.environment.variables.push(environment::Variable {
            name: "GTK_THEME".into(),
            value: "Dracula".into(),
            enabled: true,
        });
        session.gestures.gestures.push(gestures::Gesture::default());
        session.permissions.rules.push(permissions::Rule {
            binary: "/usr/bin/grim".into(),
            r#type: "screencopy".into(),
            mode: permissions::Mode::Allow,
            enabled: true,
        });
        session
    }

    #[test]
    fn round_trips_through_toml() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.toml");
        save(&path, &sample()).unwrap();
        assert_eq!(load(&path).unwrap(), sample());
    }

    #[test]
    fn missing_file_loads_as_nothing_configured() {
        let dir = tempfile::tempdir().unwrap();
        assert!(load(&dir.path().join("no.toml")).unwrap().is_empty());
    }

    /// The rule that keeps the data-loss bug from returning.
    #[test]
    fn an_unparseable_file_is_an_error_not_an_empty_session() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.toml");
        std::fs::write(&path, "not = = toml").unwrap();
        assert!(load(&path).is_err());
    }

    /// A file written before a section existed must still load.
    #[test]
    fn a_partial_file_loads_with_the_rest_empty() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.toml");
        std::fs::write(&path, "[[autostart.program]]\ncommand = \"waybar\"\n").unwrap();
        let session = load(&path).unwrap();
        assert_eq!(session.autostart.programs.len(), 1);
        assert!(session.environment.is_empty());
        assert!(
            session.autostart.programs[0].enabled,
            "a program with no `enabled` defaults to on, not off"
        );
    }
    /// Only a file that is *not there* is a first run. `path.exists()` is
    /// false for anything it cannot stat — a directory without search
    /// permission, here — so gating on it read an unreachable file as
    /// "nothing configured", and the next save wrote that over it.
    #[test]
    fn a_file_that_cannot_be_reached_is_an_error_not_a_first_run() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let locked = dir.path().join("locked");
        std::fs::create_dir(&locked).unwrap();
        let path = locked.join("settings.toml");
        std::fs::write(&path, "").unwrap();
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();
        let reachable = std::fs::metadata(&path).is_ok();
        let result = load(&path);
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o700)).unwrap();
        if reachable {
            eprintln!("HYPRFORGE-SKIP: running with permission to read anything (root?)");
            return;
        }
        assert!(result.is_err(), "an unreachable file read as a first run");
    }
}
