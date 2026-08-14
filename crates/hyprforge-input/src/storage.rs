//! The canonical `input.toml`, and the only source of truth for what this
//! module owns.

use crate::model::Settings;
use std::path::Path;

#[derive(Debug, thiserror::Error)]
pub enum StorageError {
    #[error("failed to read {path}: {source}")]
    Read {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to write {path}: {source}")]
    Write {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to parse {path}: {source}")]
    Parse {
        path: String,
        #[source]
        source: toml::de::Error,
    },
    #[error("failed to serialize settings: {0}")]
    Serialize(#[from] toml::ser::Error),
}

/// Loads the owned settings from `path`. A missing file means "nothing
/// configured yet" (first run), not an error.
///
/// A file that exists but can't be read or parsed **is** an error, and the
/// caller must treat it as one. Collapsing that into an empty set is what
/// cost a real user 37 hand-written binds: the app showed nothing
/// configured and the next save wrote that emptiness over the file.
pub fn load(path: &Path) -> Result<Settings, StorageError> {
    if !path.exists() {
        return Ok(Settings::default());
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
pub fn save(path: &Path, settings: &Settings) -> Result<(), StorageError> {
    let contents = toml::to_string_pretty(settings)?;
    hyprforge_core::paths::write_atomic(path, &contents).map_err(|source| StorageError::Write {
        path: path.display().to_string(),
        source,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Value;

    fn sample() -> Settings {
        let mut s = Settings::default();
        s.set("input:kb_layout", Value::Text("us,cz".into()));
        s.set("input:repeat_rate", Value::Int(30));
        s.set("input:sensitivity", Value::Float(-0.25));
        s.set("input:touchpad:tap_to_click", Value::Bool(true));
        s
    }

    #[test]
    fn round_trips_through_toml() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("input.toml");
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
        let path = dir.path().join("input.toml");
        std::fs::write(&path, "this is not = = toml").unwrap();
        assert!(load(&path).is_err());
    }

    /// Keys carry colons, which TOML needs quoted. A file that saves but
    /// won't load again is the worst possible outcome here, so this pins
    /// the round trip through real bytes rather than trusting serde.
    #[test]
    fn colon_separated_keys_survive_the_file_format() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("input.toml");
        save(&path, &sample()).unwrap();
        let raw = std::fs::read_to_string(&path).unwrap();
        assert!(raw.contains("\"input:touchpad:tap_to_click\""), "{raw}");
        assert_eq!(load(&path).unwrap(), sample());
    }

    /// [`Value`] is an untagged enum whose `Int` arm is tried before its
    /// `Float` arm, so a whole float is exactly where a silent type change
    /// would hide. `1.0` coming back as `Int(1)` would render as `1` in the
    /// generated Lua — an integer reaching a setting Hyprland types as a
    /// float.
    #[test]
    fn a_whole_float_does_not_come_back_as_an_integer() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("input.toml");
        let mut s = Settings::default();
        s.set("input:touchpad:scroll_factor", Value::Float(1.0));
        save(&path, &s).unwrap();

        assert_eq!(
            load(&path).unwrap().get("input:touchpad:scroll_factor"),
            Some(&Value::Float(1.0))
        );
        let lua = crate::codegen::generate(&load(&path).unwrap());
        assert!(lua.contains("scroll_factor = 1.0,"), "{lua}");
    }

    /// The file is meant to be hand-editable, so a hand-written spelling
    /// has to load.
    #[test]
    fn a_hand_written_file_loads() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("input.toml");
        std::fs::write(
            &path,
            "\"input:kb_layout\" = \"de\"\n\"input:repeat_rate\" = 40\n",
        )
        .unwrap();
        let s = load(&path).unwrap();
        assert_eq!(s.get("input:kb_layout"), Some(&Value::Text("de".into())));
        assert_eq!(s.get("input:repeat_rate"), Some(&Value::Int(40)));
    }
}
