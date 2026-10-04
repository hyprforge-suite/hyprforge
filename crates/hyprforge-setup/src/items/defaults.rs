//! Files for folders and Media for pictures, in the user's
//! `mimeapps.list`.
//!
//! One line per type, through `hyprforge_mime::defaults` — the same edit
//! the Default apps page makes, which copies every other line through.
//! Types are canonicalised first: a line written under an alias is a line
//! nothing reads back.

use super::{Applied, Cx};
use crate::record::{Change, PreviousDefault};
use crate::state::State;
use hyprforge_mime::{defaults, MimeDb};

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct DefaultsSpec {
    /// The desktop entry's file name.
    pub app: &'static str,
    /// The application's name, for messages.
    pub name: &'static str,
    pub mimes: &'static [&'static str],
    pub requires: &'static [&'static str],
}

pub(crate) const FOLDERS: DefaultsSpec = DefaultsSpec {
    app: "hyprforge-files.desktop",
    name: "Files",
    mimes: &["inode/directory"],
    requires: &["hyprforge-files"],
};

/// The Default apps page's "Images" row, which is also exactly what
/// Media's desktop entry claims of these. Not SVG: Media does not open it.
pub(crate) const IMAGES: DefaultsSpec = DefaultsSpec {
    app: "hyprforge-media.desktop",
    name: "Media",
    mimes: &["image/png", "image/jpeg", "image/gif", "image/webp", "image/tiff", "image/bmp"],
    requires: &["hyprforge-media"],
};

fn database(cx: &Cx<'_>) -> MimeDb {
    MimeDb::load_from(&cx.env.data_dirs, &cx.env.mimeapps_paths)
}

/// The user's own `mimeapps.list`, or empty if there is none.
fn read_user(cx: &Cx<'_>) -> Result<String, String> {
    let path = cx.env.mimeapps_list();
    match std::fs::read_to_string(&path) {
        Ok(text) => Ok(text),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(e) => Err(format!("couldn't read {}: {e}", path.display())),
    }
}

/// The types not yet opened by `spec.app`, canonical.
fn not_yet<'d>(db: &'d MimeDb, spec: &DefaultsSpec) -> Vec<&'d str> {
    spec.mimes
        .iter()
        .map(|m| db.canonical(m))
        .filter(|m| db.default_for(m).is_none_or(|app| app.id != spec.app))
        .collect()
}

pub(super) fn check(cx: &Cx<'_>, spec: &DefaultsSpec) -> State {
    // Reading it here, not only on apply: a list that exists and can't be
    // read must not look like one with no defaults in it.
    if let Err(e) = read_user(cx) {
        return State::Unknown { why: e };
    }
    let db = database(cx);
    if db.app(spec.app).is_none_or(|app| !app.installed) {
        return State::Unavailable { why: format!("{} has no desktop entry installed", spec.name) };
    }
    let todo = not_yet(&db, spec);
    if todo.is_empty() {
        return State::Done;
    }
    // Name what it replaces, when it is one thing: "instead of Nemo".
    let current: Vec<String> = todo
        .iter()
        .filter_map(|m| db.default_for(m).map(|a| a.name.clone()))
        .collect();
    let instead = match current.first() {
        Some(first) if current.iter().all(|c| c == first) => format!(" instead of {first}"),
        _ => String::new(),
    };
    State::Todo { what: format!("Open {} with {}{instead}", todo.join(", "), spec.name) }
}

pub(super) fn apply(cx: &Cx<'_>, spec: &DefaultsSpec) -> Result<Applied, String> {
    let db = database(cx);
    let mut text = read_user(cx)?;
    let before = defaults::parse(&text);
    let mut previous = Vec::new();
    for mime in not_yet(&db, spec) {
        previous.push(PreviousDefault {
            mime: mime.to_string(),
            app: before.get(mime).and_then(|apps| apps.first()).cloned(),
        });
        text = defaults::with_default(&text, mime, spec.app);
    }
    write_user(cx, &text)?;
    Ok(Applied { change: Change::Defaults { app: spec.app.to_string(), previous }, note: None })
}

fn write_user(cx: &Cx<'_>, text: &str) -> Result<(), String> {
    let path = cx.env.mimeapps_list();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("couldn't create {}: {e}", dir.display()))?;
    }
    hyprforge_paths::write_atomic(&path, text)
        .map_err(|e| format!("couldn't write {}: {e}", path.display()))
}

/// Puts back each type setup changed — unless the user has chosen
/// something else for it since, which is theirs.
pub(super) fn undo(
    cx: &Cx<'_>,
    app: &str,
    previous: &[PreviousDefault],
) -> Result<Option<String>, String> {
    let original = read_user(cx)?;
    let mut text = original.clone();
    let mut kept = Vec::new();
    for p in previous {
        let now = defaults::parse(&text).get(&p.mime).and_then(|a| a.first()).cloned();
        if now.as_deref() != Some(app) {
            kept.push(p.mime.as_str());
            continue;
        }
        text = match &p.app {
            Some(old) => defaults::with_default(&text, &p.mime, old),
            None => defaults::without_default(&text, &p.mime).unwrap_or(text),
        };
    }
    if text != original {
        write_user(cx, &text)?;
    }
    Ok((!kept.is_empty())
        .then(|| format!("left {} alone: changed since setup set it", kept.join(", "))))
}
