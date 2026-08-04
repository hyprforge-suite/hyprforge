use crate::model::Rule;
use serde::{Deserialize, Serialize};
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
    #[error("failed to serialize rules: {0}")]
    Serialize(#[from] toml::ser::Error),
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct RuleFile {
    #[serde(rename = "rule", default)]
    rules: Vec<Rule>,
}

/// Loads the ordered rule list from `path`. A missing file is treated as an
/// empty rule set (first run), not an error.
pub fn load(path: &Path) -> Result<Vec<Rule>, StorageError> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let contents = std::fs::read_to_string(path).map_err(|source| StorageError::Read {
        path: path.display().to_string(),
        source,
    })?;
    let file: RuleFile = toml::from_str(&contents).map_err(|source| StorageError::Parse {
        path: path.display().to_string(),
        source,
    })?;
    Ok(file.rules)
}

/// Writes the ordered rule list to `path`, atomically (temp file + rename),
/// so a crash mid-write never corrupts the canonical TOML.
pub fn save(path: &Path, rules: &[Rule]) -> Result<(), StorageError> {
    let file = RuleFile {
        rules: rules.to_vec(),
    };
    let contents = toml::to_string_pretty(&file)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|source| StorageError::Write {
            path: path.display().to_string(),
            source,
        })?;
    }
    let tmp = path.with_extension("toml.tmp");
    std::fs::write(&tmp, &contents).map_err(|source| StorageError::Write {
        path: tmp.display().to_string(),
        source,
    })?;
    std::fs::rename(&tmp, path).map_err(|source| StorageError::Write {
        path: path.display().to_string(),
        source,
    })?;
    Ok(())
}
