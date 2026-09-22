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

/// Whether the sidebar is showing.
///
/// Three states rather than a `bool`, because two different things want
/// to decide this and only one of them is the user. `Auto` lets the
/// window width decide — a sidebar is 216 logical pixels, which is most
/// of a narrow window and worth reclaiming automatically. `Shown` and
/// `Hidden` are what the toggle sets, and they win everywhere, including
/// below the breakpoint: a small window is a reason to *default* to
/// hiding the sidebar, never a reason to refuse to show it. Somebody
/// narrowing a window to navigate to Downloads still has to be able to
/// reach Downloads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SidebarPref {
    #[default]
    Auto,
    Shown,
    Hidden,
}

impl SidebarPref {
    /// Whether the sidebar is collapsed, given how wide the window is
    /// and the width `Auto` collapses below (`[sidebar] collapse-below`;
    /// `0` never collapses on its own).
    pub fn collapsed(self, viewport_width: f32, collapse_below: f32) -> bool {
        match self {
            SidebarPref::Shown => false,
            SidebarPref::Hidden => true,
            SidebarPref::Auto => viewport_width < collapse_below,
        }
    }
}

/// One optional column in the list view.
///
/// Name is not here: it is the row's identity, and a listing with the
/// names switched off is not a listing. Everything else is the user's
/// choice and is remembered — see [`Columns`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Column {
    Kind,
    Size,
    Owner,
    Permissions,
    Modified,
    /// Where a trashed item came from. Not one of the switchable columns
    /// — it means nothing outside the Trash, and inside it is the column
    /// that matters most — so the Trash listing always shows it and no
    /// other listing ever does. Not in [`Column::ALL`].
    Origin,
    /// How much room a member takes up inside its archive. The same
    /// arrangement as [`Column::Origin`] and for the same reason: it
    /// means nothing outside an archive, so an archive listing always
    /// shows it and no other listing ever does. Not in [`Column::ALL`].
    Packed,
}

impl Column {
    /// Left to right, the order they appear in the header. One list, so
    /// the header, the rows and the picker cannot disagree about it.
    pub const ALL: [Column; 5] =
        [Column::Kind, Column::Size, Column::Owner, Column::Permissions, Column::Modified];

    pub fn label(self) -> &'static str {
        match self {
            Column::Kind => "Kind",
            Column::Size => "Size",
            Column::Owner => "Owner",
            Column::Permissions => "Permissions",
            Column::Modified => "Modified",
            Column::Packed => "Packed",
            Column::Origin => "Original Location",
        }
    }
}

/// Which optional columns the list view shows.
///
/// A struct of named `bool`s rather than a `HashSet<Column>`, because
/// this is what lands in `files.toml` and `columns.owner = false` reads
/// as a setting where a list of strings reads as data. `#[serde(default)]`
/// per field, so a file written before a column existed still parses and
/// simply gets that column's default.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Columns {
    pub kind: bool,
    pub size: bool,
    pub owner: bool,
    pub permissions: bool,
    pub modified: bool,
}

impl Default for Columns {
    fn default() -> Self {
        // All five on by default. The alternative — shipping the new
        // ones off — means nobody discovers them, and the picker that
        // turns them off is right there in the header.
        Columns { kind: true, size: true, owner: true, permissions: true, modified: true }
    }
}

impl Columns {
    pub fn shows(&self, column: Column) -> bool {
        match column {
            Column::Kind => self.kind,
            Column::Size => self.size,
            Column::Owner => self.owner,
            Column::Permissions => self.permissions,
            Column::Modified => self.modified,
            Column::Origin | Column::Packed => false,
        }
    }

    pub fn set(&mut self, column: Column, shown: bool) {
        let slot = match column {
            Column::Kind => &mut self.kind,
            Column::Size => &mut self.size,
            Column::Owner => &mut self.owner,
            Column::Permissions => &mut self.permissions,
            Column::Modified => &mut self.modified,
            // Not switchable — see the variants' docs.
            Column::Origin | Column::Packed => return,
        };
        *slot = shown;
    }

    pub fn toggle(&mut self, column: Column) {
        let shown = self.shows(column);
        self.set(column, !shown);
    }

    /// The shown columns, in header order.
    pub fn shown(&self) -> impl Iterator<Item = Column> + '_ {
        Column::ALL.into_iter().filter(|&c| self.shows(c))
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Prefs {
    sort_column: SortColumn,
    sort_direction: SortDirection,
    pub directories_first: bool,
    pub show_hidden: bool,
    pub view_mode: ViewMode,
    /// Which optional list columns to show — see [`Columns`].
    pub columns: Columns,
    /// Whether the sidebar shows — see [`SidebarPref`].
    pub sidebar: SidebarPref,
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
            sort_column: SortColumn::Name,
            sort_direction: SortDirection::Ascending,
            directories_first: true,
            show_hidden: false,
            view_mode: ViewMode::List,
            columns: Columns::default(),
            sidebar: SidebarPref::default(),
            preview_pane: true,
            window_width: 900,
            window_height: 600,
            pinned: Vec::new(),
        }
    }
}

impl Prefs {
    // Accessors rather than `pub` fields, kept from when these wrapped a
    // mirror type: they are now the only thing stopping a caller writing
    // a sort column without going through `Browser`, which has to
    // re-sort when one changes.
    pub fn sort_column(&self) -> SortColumn {
        self.sort_column
    }

    pub fn set_sort_column(&mut self, column: SortColumn) {
        self.sort_column = column;
    }

    pub fn sort_direction(&self) -> SortDirection {
        self.sort_direction
    }

    pub fn set_sort_direction(&mut self, direction: SortDirection) {
        self.sort_direction = direction;
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

    /// A narrow window collapses the sidebar on its own — 216 pixels
    /// of it is most of a small window, and the listing it exists to
    /// help you navigate has no room left.
    #[test]
    fn a_narrow_window_collapses_the_sidebar_and_a_wide_one_does_not() {
        let below = crate::density::SIDEBAR_COLLAPSE_BELOW;
        assert!(SidebarPref::Auto.collapsed(500.0, below));
        assert!(!SidebarPref::Auto.collapsed(1200.0, below));
        assert!(!SidebarPref::Auto.collapsed(500.0, 0.0), "0 never collapses on its own");
    }

    /// And an explicit choice beats the width in both directions. A
    /// small window is a reason to *default* to hiding the sidebar,
    /// never a reason to refuse to show it: somebody who narrowed a
    /// window still has to be able to reach Downloads.
    #[test]
    fn an_explicit_choice_wins_over_the_width_in_both_directions() {
        assert!(!SidebarPref::Shown.collapsed(320.0, 760.0), "asked for it at any width");
        assert!(SidebarPref::Hidden.collapsed(3000.0, 760.0), "hidden at any width");
    }

    #[test]
    fn the_sidebar_choice_survives_a_round_trip_through_toml() {
        let mut prefs = Prefs::default();
        assert_eq!(prefs.sidebar, SidebarPref::Auto, "letting the width decide is the default");
        prefs.sidebar = SidebarPref::Hidden;
        let back: Prefs = toml::from_str(&toml::to_string_pretty(&prefs).unwrap()).unwrap();
        assert_eq!(back.sidebar, SidebarPref::Hidden);
    }

    /// Name is not a `Column`, and that is the point: a listing with the
    /// names switched off is not a listing.
    #[test]
    fn every_optional_column_can_be_switched_off() {
        let mut columns = Columns::default();
        for column in Column::ALL {
            assert!(columns.shows(column), "{column:?} is on by default");
            columns.toggle(column);
            assert!(!columns.shows(column), "{column:?} must switch off");
        }
        assert_eq!(columns.shown().count(), 0);
    }

    /// `shown()` is what both the header and the rows walk, so its order
    /// is what stops a cell landing under the wrong heading.
    #[test]
    fn shown_columns_come_back_in_header_order() {
        let mut columns = Columns::default();
        columns.set(Column::Size, false);
        let shown: Vec<Column> = columns.shown().collect();
        assert_eq!(shown, [Column::Kind, Column::Owner, Column::Permissions, Column::Modified]);
    }

    /// A `files.toml` written before these columns existed still parses,
    /// and simply gets their defaults — the `#[serde(default)]` rule
    /// this module's own doc sets out, checked rather than assumed.
    #[test]
    fn a_settings_file_written_before_columns_existed_still_parses() {
        let older = r#"
            sort_column = "name"
            sort_direction = "ascending"
            directories_first = true
            show_hidden = false
            view_mode = "list"
        "#;
        let prefs: Prefs = toml::from_str(older).expect("an older file still parses");
        assert_eq!(prefs.columns, Columns::default());
    }

    /// And a file that turns one off gets it back off, which is the
    /// whole point of remembering the choice.
    #[test]
    fn a_column_switched_off_survives_a_round_trip_through_toml() {
        let mut prefs = Prefs::default();
        prefs.columns.set(Column::Permissions, false);
        let text = toml::to_string_pretty(&prefs).unwrap();
        let back: Prefs = toml::from_str(&text).unwrap();
        assert!(!back.columns.shows(Column::Permissions));
        assert!(back.columns.shows(Column::Owner), "the others are untouched");
    }

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
