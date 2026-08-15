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
    if !path.exists() {
        return Ok(Session::default());
    }
    let contents = std::fs::read_to_string(path).map_err(|source| StorageError::Read {
        path: path.display().to_string(),
        source,
    })?;
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
}
