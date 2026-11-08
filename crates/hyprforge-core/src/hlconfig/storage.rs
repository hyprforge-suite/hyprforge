//! The canonical `input.toml`, and the only source of truth for what this
//! module owns.

use super::Settings;
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
    // Read first and ask what went wrong, rather than `path.exists()`
    // first: that is false for anything it cannot stat (no permission on
    // a directory above it, an I/O error), and each of those read as a
    // first run whose next save replaced the file. Only `NotFound` is.
    let contents = match std::fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Settings::default()),
        Err(source) => {
            return Err(StorageError::Read { path: path.display().to_string(), source });
        }
    };
    toml::from_str(&contents).map_err(|source| StorageError::Parse {
        path: path.display().to_string(),
        source,
    })
}

/// Writes atomically (temp file + rename), so a crash mid-write never
/// leaves a half-written canonical file.
pub fn save(path: &Path, settings: &Settings) -> Result<(), StorageError> {
    let contents = toml::to_string_pretty(settings)?;
    crate::paths::write_atomic(path, &contents).map_err(|source| StorageError::Write {
        path: path.display().to_string(),
        source,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hlconfig::model::test_catalog::CATALOG;
    use crate::hlconfig::Value;

    fn sample() -> Settings {
        let mut s = Settings::default();
        s.set("demo:name", Value::Text("us,cz".into()));
        s.set("demo:count", Value::Int(3));
        s.set("demo:ratio", Value::Float(-0.25));
        s.set("demo:nested:deep", Value::Bool(true));
        s.set("demo:offset", Value::Pair([2.0, -3.0]));
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
        assert!(raw.contains("\"demo:nested:deep\""), "{raw}");
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
        s.set("demo:ratio", Value::Float(1.0));
        save(&path, &s).unwrap();

        assert_eq!(
            load(&path).unwrap().get("demo:ratio"),
            Some(&Value::Float(1.0))
        );
        let lua = crate::hlconfig::codegen::generate(&load(&path).unwrap(), &CATALOG, "demo");
        assert!(lua.contains("ratio = 1.0,"), "{lua}");
    }

    /// The file is meant to be hand-editable, so a hand-written spelling
    /// has to load.
    #[test]
    fn a_hand_written_file_loads() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("input.toml");
        std::fs::write(
            &path,
            "\"demo:name\" = \"de\"\n\"demo:count\" = 40\n",
        )
        .unwrap();
        let s = load(&path).unwrap();
        assert_eq!(s.get("demo:name"), Some(&Value::Text("de".into())));
        assert_eq!(s.get("demo:count"), Some(&Value::Int(40)));
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
