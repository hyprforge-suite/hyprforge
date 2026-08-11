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
    if !path.exists() {
        return Ok(Vec::new());
    }
    let contents = std::fs::read_to_string(path).map_err(|source| StorageError::Read {
        path: path.display().to_string(),
        source,
    })?;
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
