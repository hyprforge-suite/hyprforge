use crate::model::{Rule, WorkspaceRule};
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
    /// Workspace→monitor pins. One file for both kinds: they're one
    /// user-facing concept ("my window rules"), and a file that predates this
    /// field simply has none.
    #[serde(rename = "workspace_rule", default)]
    workspace_rules: Vec<WorkspaceRule>,
}

/// Everything the module persists. Returned as a struct rather than a tuple
/// so adding a third rule kind doesn't churn every call site.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Rules {
    pub rules: Vec<Rule>,
    pub workspace_rules: Vec<WorkspaceRule>,
}

/// Loads the ordered rule lists from `path`. A missing file is treated as an
/// empty rule set (first run), not an error.
pub fn load(path: &Path) -> Result<Rules, StorageError> {
    if !path.exists() {
        return Ok(Rules::default());
    }
    let contents = std::fs::read_to_string(path).map_err(|source| StorageError::Read {
        path: path.display().to_string(),
        source,
    })?;
    let file: RuleFile = toml::from_str(&contents).map_err(|source| StorageError::Parse {
        path: path.display().to_string(),
        source,
    })?;
    Ok(Rules {
        rules: file.rules,
        workspace_rules: file.workspace_rules,
    })
}

/// Writes the ordered rule lists to `path`, atomically (temp file + rename),
/// so a crash mid-write never corrupts the canonical TOML.
pub fn save(path: &Path, rules: &Rules) -> Result<(), StorageError> {
    let file = RuleFile {
        rules: rules.rules.clone(),
        workspace_rules: rules.workspace_rules.clone(),
    };
    let contents = toml::to_string_pretty(&file)?;
    hyprforge_core::paths::write_atomic(path, &contents).map_err(|source| StorageError::Write {
        path: path.display().to_string(),
        source,
    })
}
