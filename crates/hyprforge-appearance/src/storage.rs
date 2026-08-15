//! The canonical `appearance.toml`.
//!
//! Holds both halves the module writes: the `hl.config` settings and the
//! per-animation overrides. One file because they are one screen and one
//! generated Lua file; separate tables because they have different shapes.
//!
//! The gsettings half is deliberately absent. Those values live in
//! gsettings itself, which is the only copy — storing a second one here
//! would create two sources of truth that drift the moment anything else
//! on the desktop writes a theme.

use crate::animations::Animations;
use hyprforge_core::hlconfig::Settings;
use serde::{Deserialize, Serialize};
use std::path::Path;

pub use hyprforge_core::hlconfig::storage::StorageError;

#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
pub struct Appearance {
    #[serde(default)]
    pub settings: Settings,
    #[serde(default)]
    pub animations: Animations,
}

impl Appearance {
    pub fn is_empty(&self) -> bool {
        self.settings.is_empty() && self.animations.is_empty()
    }
}

/// A missing file means "nothing configured yet" (first run), not an
/// error. A file that exists but can't be read **is** an error and the
/// caller must treat it as one: collapsing that into an empty set is what
/// cost a real user 37 hand-written binds.
pub fn load(path: &Path) -> Result<Appearance, StorageError> {
    if !path.exists() {
        return Ok(Appearance::default());
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

/// Writes atomically (temp file + rename), so a crash mid-write never
/// leaves a half-written canonical file.
pub fn save(path: &Path, appearance: &Appearance) -> Result<(), StorageError> {
    let contents = toml::to_string_pretty(appearance)?;
    hyprforge_core::paths::write_atomic(path, &contents).map_err(|source| StorageError::Write {
        path: path.display().to_string(),
        source,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::animations::Animation;
    use hyprforge_core::hlconfig::Value;

    fn sample() -> Appearance {
        let mut settings = Settings::default();
        settings.set("decoration:rounding", Value::Int(10));
        settings.set("general:col:active_border", Value::Text("rgba(bd93f9ff)".into()));
        settings.set("decoration:blur:vibrancy", Value::Float(0.1696));
        let mut animations = Animations::default();
        animations.set(
            "windows",
            Animation {
                enabled: true,
                speed: 4.79,
                bezier: "easeOutQuint".to_string(),
                style: String::new(),
            },
        );
        Appearance { settings, animations }
    }

    #[test]
    fn round_trips_through_toml() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("appearance.toml");
        save(&path, &sample()).unwrap();
        assert_eq!(load(&path).unwrap(), sample());
    }

    #[test]
    fn missing_file_loads_as_nothing_configured() {
        let dir = tempfile::tempdir().unwrap();
        assert!(load(&dir.path().join("nope.toml")).unwrap().is_empty());
    }

    /// The rule that keeps the data-loss bug from coming back: unreadable
    /// is not empty.
    #[test]
    fn an_unparseable_file_is_an_error_not_an_empty_set() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("appearance.toml");
        std::fs::write(&path, "this is not = = toml").unwrap();
        assert!(load(&path).is_err());
    }

    /// A file holding only settings predates animations being editable,
    /// and must still load.
    #[test]
    fn a_file_without_animations_still_loads() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("appearance.toml");
        std::fs::write(&path, "[settings]\n\"decoration:rounding\" = 10\n").unwrap();
        let loaded = load(&path).unwrap();
        assert_eq!(loaded.settings.get("decoration:rounding"), Some(&Value::Int(10)));
        assert!(loaded.animations.is_empty());
    }

    /// Colon keys need quoting in TOML, and a file that saves but won't
    /// load again is the worst possible outcome.
    #[test]
    fn colon_separated_keys_survive_the_file_format() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("appearance.toml");
        save(&path, &sample()).unwrap();
        let raw = std::fs::read_to_string(&path).unwrap();
        assert!(raw.contains("\"general:col:active_border\""), "{raw}");
        assert_eq!(load(&path).unwrap(), sample());
    }
}
