//! `setup.toml`: what setup changed, and what was there before.
//!
//! Undo restores exactly what is recorded here and nothing else — an old
//! default application, an old `lock_cmd`, the notification daemon that
//! was stopped. Something setup found already done is never recorded, so
//! undo never "turns off" what setup did not turn on: a service a package
//! enabled for every user stays enabled after an undo, and saying
//! otherwise would be a lie about systemd (see `mock::MockSystem`, which models it).
//!
//! A file that exists and will not parse is an error, never an empty
//! record. Read as empty, an undo would report "nothing to undo" while
//! every change it should have reverted stayed in place — and an apply
//! would overwrite the only copy of the previous values. Both refuse.

use hyprforge_windowrules::LayerRule;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

#[derive(Debug, thiserror::Error)]
pub enum RecordError {
    #[error("couldn't read {path}: {source}")]
    Read {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error(
        "{path} could not be read ({source}). It is the record of what setup changed, so \
         undoing — or changing anything more — would lose track of it. Fix or remove the \
         file first."
    )]
    Parse {
        path: String,
        #[source]
        source: toml::de::Error,
    },
    #[error("couldn't write {path}: {source}")]
    Write {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("couldn't serialize the setup record: {0}")]
    Serialize(#[from] toml::ser::Error),
}

/// Everything setup has changed and not yet undone.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Record {
    /// By item id.
    #[serde(default)]
    pub items: BTreeMap<String, Entry>,
    /// Units setup masked on request ("Turn off for me") — the per-user
    /// off switch for a unit a package enabled for every user. Undoing the
    /// unit's item unmasks it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub masked: Vec<String>,
}

/// One applied item.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Entry {
    /// What was done, in the words the item used when it was offered.
    pub applied: String,
    pub change: Change,
}

/// What an item changed, with whatever it needs to put things back.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Change {
    /// Require lines inserted into `hyprland.lua`.
    Wiring {
        inserted: Vec<String>,
        /// `hyprland.lua` did not exist and setup created it. Undo still
        /// only removes lines: a config file is the user's from the moment
        /// it exists.
        #[serde(default)]
        created_config: bool,
    },
    /// A unit setup enabled for this user.
    Service { unit: String },
    /// A system unit setup enabled for everyone, through pkexec.
    SystemService { unit: String },
    /// notifd, and whichever daemons were turned off to make room for it.
    Notifd {
        /// False when notifd was already enabled and only a competitor had
        /// to go.
        enabled_notifd: bool,
        #[serde(default)]
        replaced: Vec<Replaced>,
    },
    /// A shortcut added to `shortcuts.toml`, by its stable name.
    Bind { name: String },
    /// `idle.toml`'s `lock_cmd` before setup set it.
    IdleLock { previous: String },
    /// `misc:allow_session_lock_restore` set in `system.toml`, and what
    /// `system.toml` held for it before — absent when it did not own the
    /// key, so undo gives it back to Hyprland and the user's own config
    /// rather than writing a value nobody chose.
    LockRestore {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        previous: Option<hyprforge_core::hlconfig::Value>,
    },
    /// Layer rules added to `window-rules.toml`, and any rule of the same
    /// name they replaced.
    LayerRules {
        names: Vec<String>,
        #[serde(default)]
        previous: Vec<LayerRule>,
    },
    /// Defaults written to the user's `mimeapps.list`.
    Defaults { app: String, previous: Vec<PreviousDefault> },
    /// The FileChooser line in `hyprland-portals.conf`.
    Portal {
        created_file: bool,
        added_section: bool,
        added_default: bool,
        /// The FileChooser value that was there before, if one was.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        previous: Option<String>,
    },
    /// The user's FileManager1 service file, and what it said before.
    FileManager1 {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        previous: Option<String>,
    },
    /// `GTK_USE_PORTAL=1` appended to `session.toml`.
    GtkPortal {},
    /// snapper's `home` config, and what its two settings said before.
    Snapper { allow_users: String, sync_acl: String },
    /// `/etc/pam.d/polkit-1` with the fingerprint reader added, and what
    /// it said before — `None` when there was no local copy, so undo
    /// removes it and the vendor's stack applies again.
    FingerprintPam {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        previous: Option<String>,
    },
    /// hyprforge-polkit enabled, and the agents masked to make room.
    PolkitAgent {
        /// False when it was already enabled and only a competitor had
        /// to go.
        enabled_ours: bool,
        #[serde(default)]
        replaced: Vec<ReplacedAgent>,
    },
}

/// A competing notification daemon that was turned off.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Replaced {
    pub unit: String,
    /// Masked rather than disabled, because a package had enabled it for
    /// every user and `disable` would have left it starting at login.
    #[serde(default)]
    pub masked: bool,
}

/// A competing polkit agent that was masked — see `items/polkit.rs` for
/// why a mask and not a disable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReplacedAgent {
    pub unit: String,
    /// Running when it was masked, so undo starts it again.
    #[serde(default)]
    pub was_active: bool,
}

/// One type's default before setup changed it. `app` absent means the
/// user's `mimeapps.list` had no line for it — undo removes ours rather
/// than writing one, so whatever the system file says applies again.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PreviousDefault {
    pub mime: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app: Option<String>,
}

const HEADER: &str = "# What Hyprforge's Set up changed, and what was there before, so it\n\
                      # can be undone: `hyprforge-settings --setup --undo`. Written by\n\
                      # setup; editing it changes what an undo puts back.\n\n";

/// Whether setup has ever recorded anything — the Settings page opens on
/// Set up at first launch only while this is false.
pub fn exists(path: &Path) -> bool {
    path.exists()
}

/// Reads the record. Missing is "nothing applied yet"; unreadable is an
/// error — see the module doc.
pub fn load(path: &Path) -> Result<Record, RecordError> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Record::default()),
        Err(source) => {
            return Err(RecordError::Read { path: path.display().to_string(), source })
        }
    };
    toml::from_str(&text)
        .map_err(|source| RecordError::Parse { path: path.display().to_string(), source })
}

/// Writes the record atomically, so an interrupted save cannot leave half
/// a record of what to undo.
pub fn save(path: &Path, record: &Record) -> Result<(), RecordError> {
    let body = toml::to_string_pretty(record)?;
    hyprforge_paths::write_atomic(path, &format!("{HEADER}{body}"))
        .map_err(|source| RecordError::Write { path: path.display().to_string(), source })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn every_kind() -> Record {
        let mut record = Record::default();
        let changes = [
            Change::Wiring { inserted: vec!["require(\"hyprforge/keybinds\")".into()], created_config: true },
            Change::Service { unit: "hyprforge-trayd.service".into() },
            Change::Notifd {
                enabled_notifd: true,
                replaced: vec![Replaced { unit: "dunst.service".into(), masked: false }],
            },
            Change::Bind { name: "hyprforge-files-1".into() },
            Change::IdleLock { previous: "hyprlock".into() },
            Change::LockRestore { previous: None },
            Change::LockRestore { previous: Some(hyprforge_core::hlconfig::Value::Bool(false)) },
            Change::LayerRules { names: vec!["hyprforge-notif-blur".into()], previous: vec![] },
            Change::Defaults {
                app: "hyprforge-files.desktop".into(),
                previous: vec![
                    PreviousDefault { mime: "inode/directory".into(), app: Some("nemo.desktop".into()) },
                    PreviousDefault { mime: "x-scheme-handler/trash".into(), app: None },
                ],
            },
            Change::Portal { created_file: false, added_section: true, added_default: true, previous: None },
            Change::FileManager1 { previous: Some("[D-BUS Service]\n".into()) },
            Change::GtkPortal {},
            Change::Snapper { allow_users: String::new(), sync_acl: "no".into() },
            Change::FingerprintPam { previous: None },
            Change::FingerprintPam { previous: Some("#%PAM-1.0\nauth include system-auth\n".into()) },
            Change::PolkitAgent {
                enabled_ours: true,
                replaced: vec![ReplacedAgent { unit: "hyprpolkitagent.service".into(), was_active: true }],
            },
        ];
        for (i, change) in changes.into_iter().enumerate() {
            record.items.insert(format!("item-{i}"), Entry { applied: format!("did {i}"), change });
        }
        record.masked.push("hyprforge-clipd.service".into());
        record
    }

    /// Every kind of change survives the file, `None`s included — TOML has
    /// no null, so an `Option` that serialised badly would come back as
    /// something else or not at all.
    #[test]
    fn every_change_round_trips_through_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("setup.toml");
        let record = every_kind();
        save(&path, &record).unwrap();
        assert_eq!(load(&path).unwrap(), record);
    }

    #[test]
    fn a_missing_record_is_nothing_applied_yet() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(load(&dir.path().join("setup.toml")).unwrap(), Record::default());
    }

    /// The rule CLAUDE.md keeps relearning: could not read is not empty.
    #[test]
    fn an_unparseable_record_is_an_error_not_an_empty_one() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("setup.toml");
        std::fs::write(&path, "items = = nonsense").unwrap();
        let err = load(&path).unwrap_err();
        assert!(matches!(err, RecordError::Parse { .. }));
        assert!(err.to_string().contains("Fix or remove"), "{err}");
    }
}
