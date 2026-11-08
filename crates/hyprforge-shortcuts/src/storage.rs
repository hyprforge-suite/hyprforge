use crate::model::Shortcut;
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, thiserror::Error)]
pub enum StorageError {
    #[error("failed to read {path}: {source}")]
    Read { path: String, #[source] source: std::io::Error },
    #[error("failed to write {path}: {source}")]
    Write { path: String, #[source] source: std::io::Error },
    #[error("failed to parse {path}: {source}")]
    Parse { path: String, #[source] source: toml::de::Error },
    #[error("failed to serialize shortcuts: {0}")]
    Serialize(#[from] toml::ser::Error),
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct ShortcutFile {
    #[serde(rename = "shortcut", default)]
    shortcuts: Vec<Shortcut>,
}

/// Loads the shortcut list. A missing file is an empty set (first run), not
/// an error.
pub fn load(path: &Path) -> Result<Vec<Shortcut>, StorageError> {
    // Read first and ask what went wrong, rather than `path.exists()`
    // first: that is false for anything it cannot stat (no permission on
    // a directory above it, an I/O error), and each of those read as a
    // first run whose next save replaced the file. Only `NotFound` is.
    let contents = match std::fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(source) => {
            return Err(StorageError::Read { path: path.display().to_string(), source });
        }
    };
    let file: ShortcutFile = toml::from_str(&contents).map_err(|source| StorageError::Parse {
        path: path.display().to_string(),
        source,
    })?;
    Ok(file.shortcuts)
}

/// Writes atomically, so a crash mid-write never corrupts the canonical TOML.
pub fn save(path: &Path, shortcuts: &[Shortcut]) -> Result<(), StorageError> {
    let file = ShortcutFile { shortcuts: shortcuts.to_vec() };
    let contents = toml::to_string_pretty(&file)?;
    hyprforge_core::paths::write_atomic(path, &contents).map_err(|source| StorageError::Write {
        path: path.display().to_string(),
        source,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

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
