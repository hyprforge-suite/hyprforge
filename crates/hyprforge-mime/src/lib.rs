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
pub mod cli;
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
// `PartialEq` because a host's message type derives it — the file
// manager hands a freshly-read database back to its update loop.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
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
    /// Applications a person added for a type themselves.
    added: std::collections::BTreeMap<String, Vec<String>>,
    /// Applications a person does not want offered for a type.
    removed: std::collections::BTreeMap<String, Vec<String>>,
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
            for (section, into) in [
                (defaults::parse(&text), &mut db.chosen),
                (defaults::parse_section(&text, defaults::ADDED_SECTION), &mut db.added),
                (defaults::parse_section(&text, defaults::REMOVED_SECTION), &mut db.removed),
            ] {
                for (mime, ids) in section {
                    let list = into.entry(mime).or_default();
                    for id in ids {
                        if !list.contains(&id) {
                            list.push(id);
                        }
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

    /// Makes `app` the default for `mime`, resolving an alias first.
    ///
    /// A method rather than the free [`set_default`] because the read
    /// half of this database canonicalizes (`default_for`, `apps_for`)
    /// and the write half did not: a caller naming a type by an alias
    /// wrote a line nothing would ever read back, and the next reload
    /// showed the old default — which reads as "the setting didn't
    /// take" with nothing anywhere reporting a failure.
    pub fn set_default(&self, mime: &str, app: &str) -> std::io::Result<()> {
        set_default(self.canonical(mime), app)
    }

    /// Forgets the default for `mime`, resolving an alias first — the
    /// counterpart to [`MimeDb::set_default`], for a person undoing a
    /// choice rather than making a different one.
    ///
    /// Worth telling apart from setting a different application: with
    /// no line at all, the desktop falls back to whatever registered
    /// for the type, which is where a fresh machine starts. Setting
    /// "none" is not expressible, so "put it back how it was" has no
    /// other spelling.
    ///
    /// This edits the one file this crate writes. A default that came
    /// from a system-wide `mimeapps.list` is not this suite's to remove,
    /// and clearing will leave it standing — see [`MimeDb::chosen_types`]
    /// for the read side of the same distinction.
    pub fn clear_default(&self, mime: &str) -> std::io::Result<()> {
        clear_default(self.canonical(mime))
    }

    /// Every type this machine has a name rule for, or that something
    /// registered for, or that a default was recorded for. Sorted, and
    /// each type once.
    ///
    /// Aliases are left out: each resolves to a name already in the
    /// list, and offering both makes one row of a search look like two
    /// settings that could disagree.
    pub fn known_types(&self) -> Vec<&str> {
        let mut types: Vec<&str> = self
            .lookup
            .globs
            .mimes()
            .chain(self.registered.keys().map(String::as_str))
            .chain(self.chosen.keys().map(String::as_str))
            .collect();
        types.sort_unstable();
        types.dedup();
        types
    }

    /// The types somebody has actually chosen an application for, from
    /// every `mimeapps.list` that applies.
    ///
    /// The list nothing on this desktop shows. A `mimeapps.list`
    /// accumulates over years, and an entry that has gone stale — the
    /// fstl case in [`MimeDb::default_for`] — stays invisible until a
    /// file quietly fails to open. Sorted, since it comes out of a
    /// `BTreeMap`.
    pub fn chosen_types(&self) -> Vec<&str> {
        self.chosen.keys().map(String::as_str).collect()
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
        let mime = self.canonical(mime);
        let mut ids: Vec<&String> = Vec::new();
        for source in [self.chosen.get(mime), self.added.get(mime), self.registered.get(mime)] {
            for id in source.into_iter().flatten() {
                if !ids.contains(&id) {
                    ids.push(id);
                }
            }
        }
        // A removal says "never offer this for that type", and a
        // chooser that ignores it keeps putting back something somebody
        // deliberately took away.
        let removed = self.removed.get(mime);
        ids.into_iter()
            .filter(|id| !removed.is_some_and(|list| list.contains(id)))
            .filter_map(|id| self.entries.get(id))
            .filter(|app| app.installed)
            .collect()
    }

    /// [`MimeDb::apps_for`], then the applications for every type this
    /// one is a *kind of* — an archive manager can open a 3MF, and a
    /// text editor can open a shell script.
    ///
    /// The reference implementation's `mime_applications_all`. Most
    /// callers want [`MimeDb::candidates`] instead, which says which of
    /// the two each application is.
    pub fn apps_for_including_parents(&self, mime: &str) -> Vec<&App> {
        self.candidates(mime).into_iter().map(|candidate| candidate.app).collect()
    }

    /// Everything that could open this type, ranked, and labelled with
    /// *why* it is offered.
    ///
    /// One query rather than a ranking each caller works out for
    /// itself. The file manager's chooser and `mimeopen` both ask "what
    /// can open this", and each used to diff `apps_for` against
    /// `apps_for_including_parents` to recover the distinction — with
    /// the result that the two already disagreed about the order they
    /// offered. A third consumer (a viewer, a portal) would have made
    /// it three.
    pub fn candidates(&self, mime: &str) -> Vec<Candidate<'_>> {
        let default = self.default_for(mime).map(|app| app.id.as_str());
        let own = self.apps_for(mime);
        let mut candidates: Vec<Candidate<'_>> = own
            .iter()
            .map(|app| Candidate { app, made_for: true, is_default: default == Some(app.id.as_str()) })
            .collect();
        for parent in self.lookup.types.ancestors(mime) {
            for app in self.apps_for(&parent) {
                if !candidates.iter().any(|seen| seen.app.id == app.id) {
                    candidates.push(Candidate {
                        app,
                        made_for: false,
                        is_default: default == Some(app.id.as_str()),
                    });
                }
            }
        }
        candidates
    }

    /// The application that opens `mime` today, whether or not it is
    /// installed — so a caller can say "your default is fstl, which
    /// isn't installed any more" instead of quietly offering something
    /// else.
    pub fn default_for(&self, mime: &str) -> Option<&App> {
        let id = self.chosen.get(self.canonical(mime))?.first()?;
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

/// One application that could open a type, and why it is offered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate<'a> {
    pub app: &'a App,
    /// Registered for this type itself, rather than for something it is
    /// a kind of. "Can open it" and "is meant for it" are different
    /// claims, and a chooser should not put an archive manager above a
    /// model viewer.
    pub made_for: bool,
    /// The one that opens this type today.
    pub is_default: bool,
}

/// Makes `app` the default for `mime`, in the user's own
/// `mimeapps.list`.
///
/// Prefer [`MimeDb::set_default`], which resolves an alias first. This
/// one writes the name it is given, so a caller passing an alias
/// records a line nothing will ever read back.
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

/// Removes `mime`'s default from the user's own `mimeapps.list`.
///
/// Prefer [`MimeDb::clear_default`], which resolves an alias first.
///
/// Nothing recorded is already the state being asked for, so a missing
/// file and a file with no line for this type are both `Ok(())` and
/// neither writes anything. That is the one case where doing nothing is
/// the whole job.
pub fn clear_default(mime: &str) -> std::io::Result<()> {
    let path = user_mimeapps_path();
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e),
    };
    let Some(without) = defaults::without_default(&text, mime) else { return Ok(()) };
    hyprforge_paths::write_atomic(&path, &without)
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

/// Every `mimeapps.list` that applies, most specific first. Public
/// because the `mimeopen` command loads the database the same way.
///
/// The desktop-prefixed one (`hyprland-mimeapps.list`) comes before the
/// plain one in each directory, which is the spec's way of letting a
/// desktop differ from the machine's own choices.
pub fn mimeapps_paths() -> Vec<PathBuf> {
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

    /// The two lists a manager screen is built on: everything that
    /// could be searched for, and the much shorter list of what
    /// somebody actually chose.
    #[test]
    fn the_types_known_and_the_types_chosen_are_different_questions() {
        let (_dir, db) = fixture();
        let known = db.known_types();
        assert!(known.contains(&"model/stl"), "a glob rule names it: {known:?}");
        assert!(known.contains(&"model/3mf"));
        assert!(known.windows(2).all(|pair| pair[0] < pair[1]), "sorted, each once: {known:?}");

        assert_eq!(db.chosen_types(), vec!["model/3mf"], "the only line in the fixture's mimeapps");
        assert!(
            !db.chosen_types().contains(&"model/stl"),
            "registering for a type is not choosing one for it"
        );
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

    /// One ranked answer, with "meant for this type" told apart from
    /// "could open it" — rather than each caller diffing two lists.
    #[test]
    fn candidates_say_why_each_application_is_offered() {
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path().join("data");
        std::fs::create_dir_all(data.join("mime")).unwrap();
        std::fs::create_dir_all(data.join("applications")).unwrap();
        std::fs::write(data.join("mime/globs2"), "50:model/3mf:*.3mf\n").unwrap();
        std::fs::write(data.join("mime/subclasses"), "model/3mf application/zip\n").unwrap();
        for (file, name) in [("viewer.desktop", "Viewer"), ("archiver.desktop", "Archiver")] {
            std::fs::write(
                data.join("applications").join(file),
                format!("[Desktop Entry]\nName={name}\nExec=sh %f\n"),
            )
            .unwrap();
        }
        std::fs::write(
            data.join("applications/mimeinfo.cache"),
            "[MIME Cache]\nmodel/3mf=viewer.desktop;\napplication/zip=archiver.desktop;\n",
        )
        .unwrap();
        let mimeapps = dir.path().join("mimeapps.list");
        std::fs::write(&mimeapps, "[Default Applications]\nmodel/3mf=viewer.desktop\n").unwrap();
        let db = MimeDb::load_from(&[data], &[mimeapps]);

        let candidates = db.candidates("model/3mf");
        assert_eq!(candidates.len(), 2);
        assert_eq!(candidates[0].app.name, "Viewer");
        assert!(candidates[0].made_for && candidates[0].is_default);
        assert_eq!(candidates[1].app.name, "Archiver");
        assert!(!candidates[1].made_for, "a 3MF is a zip, but an archiver is not for 3MFs");
        assert!(!candidates[1].is_default);
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
