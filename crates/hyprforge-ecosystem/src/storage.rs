//! The canonical TOML behind each ecosystem daemon's settings.
//!
//! One generic pair rather than four near-identical copies: every module
//! in this crate stores a single serialisable struct, and the rules that
//! matter — a missing file is first-run, an unreadable one is an error,
//! writes are atomic — are the same for all of them.

use serde::{de::DeserializeOwned, Serialize};
use std::path::Path;

pub use hyprforge_core::hlconfig::storage::StorageError;

/// A missing file means "nothing configured yet" (first run), not an
/// error. A file that exists but can't be read **is** an error and the
/// caller must treat it as one: collapsing that into an empty set is what
/// cost a real user 37 hand-written binds.
pub fn load<T: DeserializeOwned + Default>(path: &Path) -> Result<T, StorageError> {
    if !path.exists() {
        return Ok(T::default());
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
pub fn save<T: Serialize>(path: &Path, value: &T) -> Result<(), StorageError> {
    let contents = toml::to_string_pretty(value)?;
    hyprforge_core::paths::write_atomic(path, &contents).map_err(|source| StorageError::Write {
        path: path.display().to_string(),
        source,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;

    #[derive(Debug, Default, PartialEq, Serialize, Deserialize)]
    struct Sample {
        name: String,
        count: u32,
    }

    #[test]
    fn round_trips_through_toml() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("s.toml");
        let value = Sample { name: "x".into(), count: 3 };
        save(&path, &value).unwrap();
        assert_eq!(load::<Sample>(&path).unwrap(), value);
    }

    #[test]
    fn missing_file_loads_as_nothing_configured() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(load::<Sample>(&dir.path().join("no.toml")).unwrap(), Sample::default());
    }

    /// The rule that keeps the data-loss bug from coming back.
    #[test]
    fn an_unparseable_file_is_an_error_not_an_empty_value() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("s.toml");
        std::fs::write(&path, "not = = toml").unwrap();
        assert!(load::<Sample>(&path).is_err());
    }
}
