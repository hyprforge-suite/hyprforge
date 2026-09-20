//! What type a file is, which applications can open it, and which one
//! does by default.
//!
//! Three questions, three data files, all of them freedesktop's and all
//! of them plain text:
//!
//! | Question | File | Module |
//! |---|---|---|
//! | What type is `part.3mf`? | `mime/globs2` | [`globs`] |
//! | What is this file, with no name to go on? | `mime/magic` | [`magic`] |
//! | Which of those two wins? | — | [`lookup`] |
//! | What is a `model/3mf` a kind of? | `mime/subclasses`, `mime/aliases` | [`types`] |
//! | What can open a `model/3mf`? | `applications/mimeinfo.cache` | [`apps`] |
//! | Which one does, normally? | `mimeapps.list` | [`defaults`] |
//!
//! # Why this crate exists
//!
//! A double-clicked STL opened in Firefox. Not because anything chose
//! that — because `xdg-open`, on a desktop it does not recognise
//! (Hyprland is one), asks `file --mime-type`, which reads bytes and not
//! names. An STL is `application/octet-stream` to it, a 3MF is a zip, a
//! `.blend` is a zstd stream; none of those has a default application,
//! and `xdg-open` answers a lookup that found nothing by walking a
//! built-in list of web browsers. Every `.stl`, `.3mf`, `.uf2`, `.blend`
//! and `.sh` in a real Downloads folder went that way.
//!
//! The database on the machine had the right answer the whole time. So
//! this reads it.
//!
//! # What this is not
//!
//! Not a launcher, and not a fourth opinion about which application
//! opens a file. It reads what the desktop already recorded and writes
//! only what a person explicitly chose; running the thing is still
//! `gio launch` with `xdg-open` behind it. Nothing here parses `Exec=`
//! field codes, `TryExec` or `Terminal=true` — see [`apps`] for why that
//! line is where it is.
//!
//! # Layering
//!
//! A leaf, like `hyprforge-look`: no iced, nothing Hyprland-shaped, one
//! dependency (`hyprforge-paths`, for the config directory and the
//! atomic write). The file manager needs it today; the Settings app
//! wants it for a Default Applications page, and any future viewer needs
//! the same three answers.

pub mod apps;
pub mod defaults;
pub mod globs;
pub mod lookup;
pub mod magic;
pub mod types;

pub use apps::App;
pub use globs::Globs;
pub use lookup::{Found, How, Lookup};
pub use magic::Magic;
pub use types::Types;

use std::path::{Path, PathBuf};

/// Everything the machine says about types and the applications that
/// open them, read once.
///
/// Reading it is a few small files; doing it per double-click would be a
/// few small files per double-click, so a host builds one of these and
/// keeps it. It is a snapshot: installing an application while a window
/// is open will not show up until [`MimeDb::load`] is called again.
#[derive(Debug, Clone, Default)]
pub struct MimeDb {
    /// Names, contents and the type graph — everything about types, as
    /// opposed to about applications.
    lookup: Lookup,
    /// Every readable desktop entry, by file name.
    entries: std::collections::BTreeMap<String, App>,
    /// Type to the entries that registered for it.
    registered: std::collections::BTreeMap<String, Vec<String>>,
    /// Type to the chosen application, user's file first.
    chosen: std::collections::BTreeMap<String, Vec<String>>,
}

impl MimeDb {
    /// Reads the database from the XDG directories.
    ///
    /// Never fails: a missing file is a machine without that piece,
    /// which is a smaller answer rather than an error — the same
    /// distinction the config loaders draw. What a caller can act on is
    /// [`MimeDb::knows_types`].
    pub fn load() -> MimeDb {
        MimeDb::load_from(&data_dirs(), &mimeapps_paths())
    }

    /// [`MimeDb::load`] from given directories — the seam the tests use,
    /// and what lets a caller point this at a fixture instead of the
    /// machine it is running on.
    pub fn load_from(data_dirs: &[PathBuf], mimeapps: &[PathBuf]) -> MimeDb {
        let mut db = MimeDb { lookup: Lookup::load_from(data_dirs), ..MimeDb::default() };
        for dir in data_dirs {
            let applications = dir.join("applications");
            if let Ok(text) = std::fs::read_to_string(applications.join("mimeinfo.cache")) {
                for (mime, ids) in apps::parse_cache(&text) {
                    let registered = db.registered.entry(mime).or_default();
                    for id in ids {
                        if !registered.contains(&id) {
                            registered.push(id);
                        }
                    }
                }
            }
            let Ok(read_dir) = std::fs::read_dir(&applications) else { continue };
            for entry in read_dir.flatten() {
                let path = entry.path();
                if path.extension().is_none_or(|e| e != "desktop") {
                    continue;
                }
                let Some(id) = path.file_name().map(|n| n.to_string_lossy().into_owned()) else {
                    continue;
                };
                // Earlier directories win: that is what makes a user's
                // own copy of an entry override the system's.
                if db.entries.contains_key(&id) {
                    continue;
                }
                if let Ok(text) = std::fs::read_to_string(&path) {
                    if let Some(app) = apps::parse_entry(&path, &text, &apps::on_path) {
                        db.entries.insert(id, app);
                    }
                }
            }
        }
        for path in mimeapps {
            let Ok(text) = std::fs::read_to_string(path) else { continue };
            for (mime, ids) in defaults::parse(&text) {
                let chosen = db.chosen.entry(mime).or_default();
                for id in ids {
                    if !chosen.contains(&id) {
                        chosen.push(id);
                    }
                }
            }
        }
        db
    }

    /// Whether any filename rules were found at all. `false` means the
    /// machine has no `shared-mime-info`, which a chooser should say
    /// rather than reporting every file as unknown.
    pub fn knows_types(&self) -> bool {
        !self.lookup.globs.is_empty()
    }

    /// The type of a file **by name alone**, doing no I/O. `None` when
    /// no rule matches — see [`globs::Globs::type_of`] for why that is
    /// not octet-stream.
    ///
    /// This is the one to call from a UI thread, and the one to call
    /// about a file that may not exist yet (a save dialog's filename
    /// box). [`MimeDb::sniff`] is the same question asked properly, at
    /// the cost of reading the file.
    pub fn type_of(&self, path: &Path) -> Option<&str> {
        self.lookup.globs.type_of(path)
    }

    /// What a file actually is: the name, its contents, and what kind
    /// of thing it is on disk, in the order [`lookup`] documents.
    ///
    /// Reads the file, so it belongs off the UI thread for anything
    /// that might be on a slow mount.
    pub fn sniff(&self, path: &Path) -> Found {
        self.lookup.of_file(path, true)
    }

    /// Every type a file matches, best first — the ambiguity, unresolved.
    pub fn all_types_of(&self, path: &Path) -> Vec<String> {
        self.lookup.all_of_file(path, true)
    }

    /// Whether an application registered for `parent` should be able to
    /// open a `mime`.
    pub fn is_subclass_of(&self, mime: &str, parent: &str) -> bool {
        self.lookup.is_subclass_of(mime, parent)
    }

    /// The name the database uses for a type, resolving an alias.
    pub fn canonical<'a>(&'a self, mime: &'a str) -> &'a str {
        self.lookup.types.canonical(mime)
    }

    /// Everything about types, for a caller that needs the parts
    /// directly — the `mimetype` command does.
    pub fn lookup(&self) -> &Lookup {
        &self.lookup
    }

    /// Every installed application registered for `mime`, the default
    /// first and the rest in the order the desktop lists them.
    ///
    /// Applications whose program is missing are left out: this is the
    /// list a person is about to pick from, and an entry that cannot
    /// run is a dead end rather than a choice. [`MimeDb::default_for`]
    /// still reports a missing default, because "your default is gone"
    /// is worth saying out loud.
    pub fn apps_for(&self, mime: &str) -> Vec<&App> {
        let mut ids: Vec<&String> = Vec::new();
        for id in self.chosen.get(mime).into_iter().flatten() {
            ids.push(id);
        }
        for id in self.registered.get(mime).into_iter().flatten() {
            if !ids.contains(&id) {
                ids.push(id);
            }
        }
        ids.into_iter()
            .filter_map(|id| self.entries.get(id))
            .filter(|app| app.installed)
            .collect()
    }

    /// The application that opens `mime` today, whether or not it is
    /// installed — so a caller can say "your default is fstl, which
    /// isn't installed any more" instead of quietly offering something
    /// else.
    pub fn default_for(&self, mime: &str) -> Option<&App> {
        let id = self.chosen.get(mime)?.first()?;
        self.entries.get(id)
    }

    /// Every installed application, for the "Other…" list — a person
    /// opening a file with something that never registered for its type
    /// is a normal thing to want.
    pub fn all_apps(&self) -> Vec<&App> {
        let mut apps: Vec<&App> = self.entries.values().filter(|app| app.installed).collect();
        apps.sort_by_key(|a| a.name.to_lowercase());
        apps
    }

    /// One application by its entry's file name.
    pub fn app(&self, id: &str) -> Option<&App> {
        self.entries.get(id)
    }
}

/// Makes `app` the default for `mime`, in the user's own
/// `mimeapps.list`.
///
/// Writes exactly one line of that file and copies the rest through —
/// see [`defaults`]'s own doc. The write is atomic, so an interrupted
/// save cannot leave a half-written associations file behind.
pub fn set_default(mime: &str, app: &str) -> std::io::Result<()> {
    let path = user_mimeapps_path();
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(e),
    };
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    hyprforge_paths::write_atomic(&path, &defaults::with_default(&text, mime, app))
}

/// The one file this crate writes.
pub fn user_mimeapps_path() -> PathBuf {
    hyprforge_paths::config_home().join("mimeapps.list")
}

/// `$XDG_DATA_HOME` then `$XDG_DATA_DIRS`, in the order the spec gives
/// them — public because the `mimetype` command needs the same list,
/// and `--database` replaces exactly this — earlier is more specific. Flatpak puts its exported entries in
/// here, which is how a flatpak application is offered at all.
pub fn data_dirs() -> Vec<PathBuf> {
    let mut dirs = vec![hyprforge_paths::data_home()];
    let system = std::env::var_os("XDG_DATA_DIRS")
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "/usr/local/share:/usr/share".into());
    dirs.extend(std::env::split_paths(&system));
    dirs
}

/// Every `mimeapps.list` that applies, most specific first.
///
/// The desktop-prefixed one (`hyprland-mimeapps.list`) comes before the
/// plain one in each directory, which is the spec's way of letting a
/// desktop differ from the machine's own choices.
fn mimeapps_paths() -> Vec<PathBuf> {
    let mut paths = Vec::new();
    let desktops: Vec<String> = std::env::var("XDG_CURRENT_DESKTOP")
        .unwrap_or_default()
        .split(':')
        .filter(|d| !d.is_empty())
        .map(|d| d.to_lowercase())
        .collect();
    let mut dirs = vec![hyprforge_paths::config_home()];
    let config_dirs = std::env::var_os("XDG_CONFIG_DIRS")
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "/etc/xdg".into());
    dirs.extend(std::env::split_paths(&config_dirs));
    // The data directories carry them too, after the config ones.
    dirs.extend(data_dirs().into_iter().map(|dir| dir.join("applications")));
    for dir in dirs {
        for desktop in &desktops {
            paths.push(dir.join(format!("{desktop}-mimeapps.list")));
        }
        paths.push(dir.join("mimeapps.list"));
    }
    paths
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// A whole miniature desktop on disk: the mime database, two
    /// applications, the registration cache and a user choice.
    fn fixture() -> (tempfile::TempDir, MimeDb) {
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path().join("data");
        fs::create_dir_all(data.join("mime")).unwrap();
        fs::create_dir_all(data.join("applications")).unwrap();
        fs::write(data.join("mime/globs2"), "50:model/stl:*.stl\n50:model/3mf:*.3mf\n").unwrap();
        fs::write(
            data.join("applications/view3d.desktop"),
            "[Desktop Entry]\nName=3D Viewer\nExec=sh %f\nIcon=view3d\n",
        )
        .unwrap();
        fs::write(
            data.join("applications/fstl.desktop"),
            "[Desktop Entry]\nName=fstl\nExec=fstl-not-installed-xyz %f\n",
        )
        .unwrap();
        fs::write(
            data.join("applications/mimeinfo.cache"),
            "[MIME Cache]\nmodel/stl=fstl.desktop;view3d.desktop;\nmodel/3mf=fstl.desktop;\n",
        )
        .unwrap();
        let mimeapps = dir.path().join("mimeapps.list");
        fs::write(&mimeapps, "[Default Applications]\nmodel/3mf=fstl.desktop\n").unwrap();
        let db = MimeDb::load_from(&[data], &[mimeapps]);
        (dir, db)
    }

    #[test]
    fn a_file_is_typed_by_name_and_its_applications_found() {
        let (_dir, db) = fixture();
        assert!(db.knows_types());
        assert_eq!(db.type_of(Path::new("/d/part.stl")), Some("model/stl"));
        let apps = db.apps_for("model/stl");
        assert_eq!(apps.len(), 1, "fstl is registered but not installed: {apps:?}");
        assert_eq!(apps[0].name, "3D Viewer");
    }

    /// The failure that started this: the user's default points at
    /// something uninstalled. It is still reported, because "your
    /// default is gone" is the useful sentence.
    #[test]
    fn a_default_that_is_no_longer_installed_is_still_reported() {
        let (_dir, db) = fixture();
        let default = db.default_for("model/3mf").expect("the choice is still recorded");
        assert_eq!(default.name, "fstl");
        assert!(!default.installed);
        assert!(db.apps_for("model/3mf").is_empty(), "and nothing is offered in its place");
    }

    #[test]
    fn a_type_nobody_registered_for_has_no_applications() {
        let (_dir, db) = fixture();
        assert!(db.apps_for("model/obj").is_empty());
        assert_eq!(db.default_for("model/obj"), None);
        assert_eq!(db.type_of(Path::new("/d/thing.qqq")), None);
    }

    #[test]
    fn a_machine_with_none_of_this_says_so_rather_than_guessing() {
        let db = MimeDb::load_from(&[PathBuf::from("/nonexistent-xyz")], &[]);
        assert!(!db.knows_types());
        assert!(db.all_apps().is_empty());
    }

    /// Every installed application is offerable, whatever it registered
    /// for — "open this with something else" is a normal request.
    #[test]
    fn the_other_list_is_every_installed_application_by_name() {
        let (_dir, db) = fixture();
        let names: Vec<&str> = db.all_apps().iter().map(|a| a.name.as_str()).collect();
        assert_eq!(names, ["3D Viewer"], "fstl is not installed");
    }
}
