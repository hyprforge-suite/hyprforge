use crate::profile::Profile;
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
    #[error("failed to serialize profiles: {0}")]
    Serialize(#[from] toml::ser::Error),
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct ProfileFile {
    #[serde(rename = "profile", default)]
    profiles: Vec<Profile>,
}

/// A missing file is treated as "no profiles yet" (first run), not an
/// error.
pub fn load(path: &Path) -> Result<Vec<Profile>, StorageError> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let contents = std::fs::read_to_string(path).map_err(|source| StorageError::Read {
        path: path.display().to_string(),
        source,
    })?;
    let file: ProfileFile = toml::from_str(&contents).map_err(|source| StorageError::Parse {
        path: path.display().to_string(),
        source,
    })?;
    Ok(file.profiles)
}

/// Atomic (temp file + rename) so a crash mid-write never corrupts
/// storage. TOML is hand-editable by design — this is the sole
/// human-inspectable source of truth for profiles.
pub fn save(path: &Path, profiles: &[Profile]) -> Result<(), StorageError> {
    let file = ProfileFile {
        profiles: profiles.to_vec(),
    };
    let contents = toml::to_string_pretty(&file)?;
    hyprforge_core::paths::write_atomic(path, &contents).map_err(|source| StorageError::Write {
        path: path.display().to_string(),
        source,
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profile::{ExtraOutputPolicy, HeadRecord};
    use crate::types::Transform;

    fn sample_profile() -> Profile {
        Profile {
            id: "abc123".to_string(),
            name: "2 displays incl. Dell U2720Q".to_string(),
            last_used: "2026-08-04T15:49:00Z".to_string(),
            extra_output_policy: ExtraOutputPolicy::ExtendRight,
            head_swaps: vec![("eDP-1".to_string(), "eDP-2".to_string())],
            heads: vec![HeadRecord {
                make: "BOE".to_string(),
                model: "0x0BC9".to_string(),
                serial: String::new(),
                connector_hint: "eDP-2".to_string(),
                x: 0,
                y: 0,
                width: 2560,
                height: 1600,
                refresh_mhz: 165000,
                scale: 1.6,
                transform: Transform::Normal,
                enabled: true,
            }],
        }
    }

    #[test]
    fn round_trips_through_toml() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("display-profiles.toml");
        let profiles = vec![sample_profile()];
        save(&path, &profiles).unwrap();
        let loaded = load(&path).unwrap();
        assert_eq!(loaded, profiles);
    }

    #[test]
    fn missing_file_loads_as_empty() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nope.toml");
        assert!(load(&path).unwrap().is_empty());
    }

    #[test]
    fn identity_is_human_readable_not_hash_only() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("display-profiles.toml");
        save(&path, &[sample_profile()]).unwrap();
        let contents = std::fs::read_to_string(&path).unwrap();
        assert!(contents.contains("make = \"BOE\""));
        assert!(contents.contains("model = \"0x0BC9\""));
    }
}
