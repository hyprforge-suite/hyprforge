//! What the file browser remembers between launches.
//!
//! Follows `hyprforge-tray::prefs` exactly: `#[serde(default)]` per
//! field so a file written before a field existed still parses and
//! leaves that field at its default; a **missing** file is first run and
//! yields the defaults; a file that **exists and will not parse** is
//! reported rather than silently defaulted, because doing that once
//! turned a typo in `tray.toml` into "you have configured nothing" and
//! then overwrote what the user actually wrote on the next save — see
//! CLAUDE.md and `hlconfig::storage`. [`update`] is the read-modify-write
//! every writer of this file should call instead of saving a copy it
//! loaded earlier: the Files app window and the portal's own "open/save"
//! dialog are two separate processes that can genuinely be open at once,
//! each with its own idea of what `files.toml` last said, and a plain
//! load-mutate-save from either one would silently erase whatever the
//! other had just written.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use crate::sort::{SortColumn, SortDirection};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ViewMode {
    List,
    Grid,
}

// `SortColumn`/`SortDirection` live in `crate::sort` as the pure-function
// module's own vocabulary; they derive (De)Serialize here rather than
// there so `sort.rs` — which is otherwise plain data and functions with
// no notion of "on disk" — does not need to know about TOML at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
enum SortColumnPref {
    Name,
    Size,
    Modified,
    Kind,
}

impl From<SortColumn> for SortColumnPref {
    fn from(c: SortColumn) -> Self {
        match c {
            SortColumn::Name => SortColumnPref::Name,
            SortColumn::Size => SortColumnPref::Size,
            SortColumn::Modified => SortColumnPref::Modified,
            SortColumn::Kind => SortColumnPref::Kind,
        }
    }
}

impl From<SortColumnPref> for SortColumn {
    fn from(c: SortColumnPref) -> Self {
        match c {
            SortColumnPref::Name => SortColumn::Name,
            SortColumnPref::Size => SortColumn::Size,
            SortColumnPref::Modified => SortColumn::Modified,
            SortColumnPref::Kind => SortColumn::Kind,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
enum SortDirectionPref {
    Ascending,
    Descending,
}

impl From<SortDirection> for SortDirectionPref {
    fn from(d: SortDirection) -> Self {
        match d {
            SortDirection::Ascending => SortDirectionPref::Ascending,
            SortDirection::Descending => SortDirectionPref::Descending,
        }
    }
}

impl From<SortDirectionPref> for SortDirection {
    fn from(d: SortDirectionPref) -> Self {
        match d {
            SortDirectionPref::Ascending => SortDirection::Ascending,
            SortDirectionPref::Descending => SortDirection::Descending,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Prefs {
    sort_column: SortColumnPref,
    sort_direction: SortDirectionPref,
    pub directories_first: bool,
    pub show_hidden: bool,
    pub view_mode: ViewMode,
    /// Whether the preview pane is enabled. Defaults to on, but this is
    /// a real off state a user can choose and have honoured — not merely
    /// "on unless we forgot to ask", which is the distinction the brief
    /// is explicit about: a `bool` with `#[serde(default)]` gives
    /// exactly that, since the *type's* default is `false` but this
    /// struct's [`Default`] impl below sets it `true` — an explicit
    /// choice at the one place that matters, not relying on `bool`'s own
    /// zero value.
    pub preview_pane: bool,
    pub window_width: u32,
    pub window_height: u32,
    /// Directories the user pinned to the sidebar's Pinned section, in
    /// the order they were pinned — `crate::sidebar::build_pinned` turns
    /// this into what that section actually shows (label, item count).
    /// An empty `Vec` is a legitimate, common state (nobody has pinned
    /// anything yet) and needs no separate "has the user ever touched
    /// this" flag the way `preview_pane` does: unlike a `bool`, a `Vec`'s
    /// own default *is* the right first-run value, so `#[serde(default)]`
    /// alone is enough here.
    pub pinned: Vec<PathBuf>,
    // Per-directory overrides (vision pillar 6: "auto-remember beats
    // onboarding" — a directory sorted by size once should stay sorted
    // by size) are deliberately **not implemented** in this struct. The
    // shape they would need is a `HashMap<PathBuf, DirectoryPrefs>`
    // keyed by canonical path, where `DirectoryPrefs` is some subset of
    // the fields above (sort/view, not window size). Left out because
    // that map has no eviction: every directory ever visited would stay
    // in `files.toml` forever, growing the file — and the cost of
    // parsing and rewriting it on every save — without bound for
    // someone who browses a lot of one-off directories (a Downloads
    // folder's dated subfolders, an extracted archive). A real
    // implementation needs an eviction policy (LRU by count, or drop
    // entries whose path no longer exists) decided before the field is
    // added, not after the file is already large for someone.
}

impl Default for Prefs {
    fn default() -> Self {
        Prefs {
            sort_column: SortColumnPref::Name,
            sort_direction: SortDirectionPref::Ascending,
            directories_first: true,
            show_hidden: false,
            view_mode: ViewMode::List,
            preview_pane: true,
            window_width: 900,
            window_height: 600,
            pinned: Vec::new(),
        }
    }
}

impl Prefs {
    pub fn sort_column(&self) -> SortColumn {
        self.sort_column.into()
    }

    pub fn set_sort_column(&mut self, column: SortColumn) {
        self.sort_column = column.into();
    }

    pub fn sort_direction(&self) -> SortDirection {
        self.sort_direction.into()
    }

    pub fn set_sort_direction(&mut self, direction: SortDirection) {
        self.sort_direction = direction.into();
    }
}

#[derive(Debug, thiserror::Error)]
pub enum PrefsError {
    /// The file exists and will not parse. **Not** the same as absent —
    /// see [`load_from`].
    #[error("{path} could not be read as Files settings: {source}")]
    Unreadable {
        path: PathBuf,
        #[source]
        source: toml::de::Error,
    },
    #[error("{path} could not be opened: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("{path} could not be written: {source}")]
    Write {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

/// Where it lives. `hyprforge-paths` owns this, the way it owns every
/// other single-file Hyprforge setting — one crate knows the layout of
/// the config directory, and no other crate joins path segments to guess
/// it.
pub fn path() -> PathBuf {
    hyprforge_paths::files_toml_path()
}

pub fn load() -> Result<Prefs, PrefsError> {
    load_from(&path())
}

/// Reads the preferences, or says why it could not.
///
/// A **missing** file is first run and yields the defaults. A file that
/// **exists and will not parse** is an error the user has to hear about,
/// and must never be silently replaced with defaults — see this module's
/// own doc.
pub fn load_from(path: &Path) -> Result<Prefs, PrefsError> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Prefs::default()),
        Err(source) => {
            return Err(PrefsError::Io {
                path: path.to_path_buf(),
                source,
            })
        }
    };
    toml::from_str(&text).map_err(|source| PrefsError::Unreadable {
        path: path.to_path_buf(),
        source,
    })
}

pub fn save(prefs: &Prefs) -> Result<(), PrefsError> {
    save_to(&path(), prefs)
}

pub fn save_to(path: &Path, prefs: &Prefs) -> Result<(), PrefsError> {
    let text = toml::to_string_pretty(prefs).expect("Prefs is plain data and always serialises");
    hyprforge_paths::write_atomic(path, &text).map_err(|source| PrefsError::Write {
        path: path.to_path_buf(),
        source,
    })
}

/// Read-modify-write, and the one thing every writer of this file should
/// call instead of saving a copy it loaded earlier — see this module's
/// own doc for why.
pub fn update(f: impl FnOnce(&mut Prefs)) -> Result<Prefs, PrefsError> {
    update_at(&path(), f)
}

/// [`update`], against an arbitrary path — the seam a test uses to point
/// this at a throwaway `files.toml` instead of the real one.
pub fn update_at(path: &Path, f: impl FnOnce(&mut Prefs)) -> Result<Prefs, PrefsError> {
    let mut prefs = load_from(path)?;
    f(&mut prefs);
    save_to(path, &prefs)?;
    Ok(prefs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_preview_pane_is_on_by_default() {
        assert!(Prefs::default().preview_pane);
    }

    #[test]
    fn a_missing_file_is_first_run_not_a_failure() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("files.toml");
        assert_eq!(load_from(&missing).unwrap(), Prefs::default());
    }

    /// The rule this file is most likely to break. A `files.toml` with a
    /// typo in it must be reported, not silently treated as "nothing
    /// configured" — because the next save would then overwrite what the
    /// user actually wrote.
    #[test]
    fn a_file_that_exists_and_will_not_parse_is_reported_rather_than_defaulted() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("files.toml");
        std::fs::write(&path, "show_hidden = yes please\n").unwrap();

        let err = load_from(&path).expect_err("a malformed file is an error, not defaults");
        assert!(matches!(err, PrefsError::Unreadable { .. }));
        assert!(err.to_string().contains("files.toml"));
    }

    #[test]
    fn what_is_saved_is_what_comes_back() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("files.toml");
        let mut prefs = Prefs {
            directories_first: false,
            show_hidden: true,
            view_mode: ViewMode::Grid,
            preview_pane: false,
            window_width: 1200,
            window_height: 800,
            ..Prefs::default()
        };
        prefs.set_sort_column(SortColumn::Size);
        prefs.set_sort_direction(SortDirection::Descending);

        save_to(&path, &prefs).unwrap();
        let loaded = load_from(&path).unwrap();
        assert_eq!(loaded, prefs);
        assert_eq!(loaded.sort_column(), SortColumn::Size);
        assert_eq!(loaded.sort_direction(), SortDirection::Descending);
    }

    /// A `files.toml` written before a field existed — here, simulating
    /// `preview_pane` predating this file the way `menu_y_offset`
    /// predated some existing `tray.toml`s — must still parse and get
    /// that field's real default, not `bool`'s own zero value. This is
    /// the test the brief calls out by name.
    #[test]
    fn a_file_written_before_preview_pane_existed_still_parses_and_defaults_it_on() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("files.toml");
        std::fs::write(&path, "show_hidden = true\n").unwrap();

        let prefs = load_from(&path).unwrap();
        assert!(prefs.show_hidden);
        assert!(
            prefs.preview_pane,
            "a field this file predates must get its own default, not bool's zero value"
        );
    }

    /// A `files.toml` written before `pinned` existed — same shape as
    /// the `preview_pane` test above, for the field most recently added.
    #[test]
    fn a_file_written_before_pinned_existed_still_parses_and_defaults_to_empty() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("files.toml");
        std::fs::write(&path, "show_hidden = true\n").unwrap();

        let prefs = load_from(&path).unwrap();
        assert!(prefs.pinned.is_empty());
    }

    #[test]
    fn pinned_paths_round_trip_through_save_and_load() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("files.toml");
        let prefs = Prefs {
            pinned: vec![PathBuf::from("/home/alex/Projects"), PathBuf::from("/mnt/data")],
            ..Prefs::default()
        };
        save_to(&path, &prefs).unwrap();
        assert_eq!(load_from(&path).unwrap().pinned, prefs.pinned);
    }

    // --- `update`: the read-modify-write every writer shares -------------

    /// Two windows that can genuinely be open at once — the app and the
    /// portal dialog — each changing a different field, must not clobber
    /// each other. A plain load-mutate-save built on a copy loaded once
    /// would let the second write silently erase the first.
    #[test]
    fn two_writers_changing_different_fields_both_survive() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("files.toml");
        let window_a_initial = load_from(&path).unwrap();
        let window_b_initial = load_from(&path).unwrap();
        assert_eq!(window_a_initial, window_b_initial);

        update_at(&path, |p| p.show_hidden = true).unwrap();
        update_at(&path, |p| p.window_width = 1400).unwrap();

        let on_disk = load_from(&path).unwrap();
        assert!(on_disk.show_hidden, "window A's write must not be undone by window B's");
        assert_eq!(on_disk.window_width, 1400, "window B's own write must have landed");
    }

    #[test]
    fn update_refuses_to_write_over_a_file_it_cannot_reload() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("files.toml");
        std::fs::write(&path, "show_hidden = yes please\n").unwrap();
        let before = std::fs::read_to_string(&path).unwrap();

        let err = update_at(&path, |p| p.show_hidden = false)
            .expect_err("a file that won't parse must refuse the write, not overwrite it");
        assert!(matches!(err, PrefsError::Unreadable { .. }));
        let after = std::fs::read_to_string(&path).unwrap();
        assert_eq!(before, after, "the mutation must never have been applied or saved");
    }

    #[test]
    fn update_on_a_missing_file_starts_from_the_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("files.toml");
        let updated = update_at(&path, |p| p.preview_pane = false).unwrap();
        assert!(!updated.preview_pane);
        assert!(updated.directories_first, "everything else stays at its default");
        assert_eq!(load_from(&path).unwrap(), updated);
    }
}
