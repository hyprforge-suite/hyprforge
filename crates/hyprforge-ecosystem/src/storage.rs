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
    // Read first and ask what went wrong, rather than `path.exists()`
    // first: that is false for anything it cannot stat (no permission on
    // a directory above it, an I/O error), and each of those read as a
    // first run whose next save replaced the file. Only `NotFound` is.
    let contents = match std::fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(T::default()),
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
        let result = load::<Sample>(&path);
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o700)).unwrap();
        if reachable {
            eprintln!("HYPRFORGE-SKIP: running with permission to read anything (root?)");
            return;
        }
        assert!(result.is_err(), "an unreachable file read as a first run");
    }
}
