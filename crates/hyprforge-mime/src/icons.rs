//! Which icon *name* a type has — the half of "draw this file's icon"
//! that is the MIME database's. Which *file* that name is belongs to the
//! icon theme, and to `hyprforge-icons`.
//!
//! The shared MIME database specification gives three sources, most
//! specific first, and two more plain files beside `globs2`:
//!
//! - `icons` — `type:icon`, a type's own icon when it is not the
//!   obvious one (`application/x-godot-scene:x-godot-scene`).
//! - the type itself with `/` turned into `-` — `image/png` is
//!   `image-png`. This is the one almost every type uses.
//! - `generic-icons` — `type:icon`, the family it belongs to
//!   (`application/xhtml+xml:text-html`); and failing that,
//!   `<media>-x-generic`.
//!
//! Between the second and the third, this also offers each *parent*
//! type's own name — `text/markdown` is a kind of `text/plain`, so
//! `text-plain` is a better answer than a generic page. Not in the
//! specification's list, and harmless beside it: a theme that has none
//! of these names still reaches the generic ones after.

use std::collections::BTreeMap;
use std::path::PathBuf;

/// The two icon files, read once.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Icons {
    own: BTreeMap<String, String>,
    generic: BTreeMap<String, String>,
}

impl Icons {
    /// Reads `mime/icons` and `mime/generic-icons` from every data
    /// directory given. The first to name a type keeps it, so a user's
    /// own database beats the system's.
    pub fn load_from(data_dirs: &[PathBuf]) -> Icons {
        let mut icons = Icons::default();
        for dir in data_dirs {
            let mime = dir.join("mime");
            if let Ok(text) = std::fs::read_to_string(mime.join("icons")) {
                add(&mut icons.own, &text);
            }
            if let Ok(text) = std::fs::read_to_string(mime.join("generic-icons")) {
                add(&mut icons.generic, &text);
            }
        }
        icons
    }

    /// Every icon name worth trying for `mime`, best first. `parents` is
    /// what the type is a kind of, nearest first.
    pub fn names_for(&self, mime: &str, parents: &[String]) -> Vec<String> {
        let mut names: Vec<String> = Vec::new();
        let mut push = |name: String| {
            if !names.contains(&name) {
                names.push(name);
            }
        };
        if let Some(own) = self.own.get(mime) {
            push(own.clone());
        }
        push(mime.replace('/', "-"));
        // `application/octet-stream` is every type's last ancestor and
        // its icon means "unknown", which is no better than the generic
        // family a more specific type still has.
        for parent in parents.iter().filter(|p| *p != "application/octet-stream") {
            push(parent.replace('/', "-"));
        }
        if let Some(generic) = self.generic.get(mime) {
            push(generic.clone());
        }
        if let Some((media, _)) = mime.split_once('/') {
            push(format!("{media}-x-generic"));
        }
        names
    }
}

fn add(into: &mut BTreeMap<String, String>, text: &str) {
    for line in text.lines().map(str::trim) {
        if line.starts_with('#') {
            continue;
        }
        if let Some((mime, icon)) = line.split_once(':') {
            into.entry(mime.trim().to_string()).or_insert_with(|| icon.trim().to_string());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn icons(own: &str, generic: &str) -> Icons {
        let mut icons = Icons::default();
        add(&mut icons.own, own);
        add(&mut icons.generic, generic);
        icons
    }

    /// The ordinary case: the type's own dashed name, then its family.
    #[test]
    fn a_type_is_named_by_itself_first_and_its_media_last() {
        let names = icons("", "").names_for("image/png", &[]);
        assert_eq!(names, ["image-png", "image-x-generic"]);
    }

    /// An `icons` entry overrides the obvious name, and `generic-icons`
    /// overrides the `<media>-x-generic` guess without removing it.
    #[test]
    fn the_databases_own_answers_come_before_the_guesses() {
        let db = icons("application/x-godot-scene:x-godot-scene", "application/xhtml+xml:text-html");
        assert_eq!(db.names_for("application/x-godot-scene", &[])[0], "x-godot-scene");
        assert_eq!(
            db.names_for("application/xhtml+xml", &[]),
            ["application-xhtml+xml", "text-html", "application-x-generic"]
        );
    }

    /// A parent's name sits between the type's own and the generic ones —
    /// and the universal ancestor is not offered, since its icon means
    /// "unknown".
    #[test]
    fn a_parent_type_is_tried_before_the_generic_family() {
        let parents = ["text/plain".to_string(), "application/octet-stream".to_string()];
        let names = icons("", "").names_for("text/markdown", &parents);
        assert_eq!(names, ["text-markdown", "text-plain", "text-x-generic"]);
    }
}
