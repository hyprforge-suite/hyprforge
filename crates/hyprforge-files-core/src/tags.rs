//! Tags: words a person puts on files, and the lists they make.
//!
//! **Where a tag lives.** On the file, in the `user.xdg.tags` extended
//! attribute — a comma-separated list — which is what Dolphin and Baloo
//! read and write. So a tag travels with its file through a rename or a
//! move, a copy keeps it (`hyprforge_fileops::xattr`), and a file tagged
//! in Dolphin shows its tags here and the other way round. A filesystem
//! with no extended attributes (FAT, most phones and shares) cannot hold
//! one, and the window says so rather than pretend.
//!
//! **What lists them.** The attribute answers "what tags does this file
//! have", never "which files have this tag" — that would mean reading
//! every file on the disk. So Files keeps an index of what it tagged:
//! [`Index`], in `files.toml` beside the stars. It follows moves made in
//! Files the way stars do ([`crate::starred::follow`]), and it is only an
//! index — the attribute is the truth. A tag view checks each file's
//! attribute as it reads it, and one whose tag was taken off elsewhere
//! drops out of the list ([`TagChange::Forget`]) rather than being shown
//! with a tag it no longer has.
//!
//! This module is the format and the index: pure, no I/O. Reading and
//! writing the attribute is the host's.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// The extended attribute tags are kept in.
pub const ATTRIBUTE: &str = "user.xdg.tags";

/// The tags in an attribute's value, in order, each once.
///
/// Spaces around a tag are not part of it; an empty tag (two commas
/// together) is no tag. A value that is not UTF-8 is read as far as it
/// is, losslessly where it can be — a tag is a word someone typed.
pub fn parse(value: &[u8]) -> Vec<String> {
    let text = String::from_utf8_lossy(value);
    let mut tags: Vec<String> = Vec::new();
    for tag in text.split(',').map(str::trim).filter(|t| !t.is_empty()) {
        if !tags.iter().any(|t| t == tag) {
            tags.push(tag.to_string());
        }
    }
    tags
}

/// The attribute's value for `tags`: comma-separated, as Dolphin writes
/// it. `None` for no tags at all — the attribute is removed rather than
/// left empty.
pub fn format(tags: &[String]) -> Option<Vec<u8>> {
    (!tags.is_empty()).then(|| tags.join(",").into_bytes())
}

/// A tag as someone typed it, made fit to keep — or why it cannot be.
///
/// A comma would split it in two in the attribute, so it is refused
/// rather than quietly turned into two tags.
pub fn clean(typed: &str) -> Result<String, &'static str> {
    let tag = typed.trim();
    if tag.is_empty() {
        return Err("A tag needs a name.");
    }
    if tag.contains(',') {
        return Err("A tag can't contain a comma.");
    }
    Ok(tag.to_string())
}

/// `tags` with `tag` added, if it was not there.
pub fn with(tags: &[String], tag: &str) -> Vec<String> {
    let mut out = tags.to_vec();
    if !out.iter().any(|t| t == tag) {
        out.push(tag.to_string());
    }
    out
}

/// `tags` with `tag` taken off.
pub fn without(tags: &[String], tag: &str) -> Vec<String> {
    tags.iter().filter(|t| *t != tag).cloned().collect()
}

/// Which files Files has tagged, by tag — see the module doc. Kept
/// sorted by tag, which is the order the sidebar lists them in.
pub type Index = BTreeMap<String, Vec<PathBuf>>;

/// A change to the [`Index`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TagChange {
    /// These now have `tag`.
    Add { tag: String, paths: Vec<PathBuf> },
    /// These no longer have `tag`.
    Remove { tag: String, paths: Vec<PathBuf> },
    /// These were found without `tag` — taken off somewhere else, or the
    /// file is gone. The same as `Remove`, kept apart so the window can
    /// tell what the person did from what it found.
    Forget { tag: String, paths: Vec<PathBuf> },
    /// These moved, `(from, to)`; tags on `from`, and on anything inside
    /// it, move with them.
    Follow(Vec<(PathBuf, PathBuf)>),
}

/// The index with `change` applied. Pure and idempotent, as
/// [`crate::starred::apply_star_change`] is. A tag left with no files
/// is gone from the index — the sidebar shows no empty tags.
pub fn apply(index: &Index, change: &TagChange) -> Index {
    let mut out = index.clone();
    match change {
        TagChange::Add { tag, paths } => {
            let list = out.entry(tag.clone()).or_default();
            for path in paths {
                if !list.contains(path) {
                    list.push(path.clone());
                }
            }
        }
        TagChange::Remove { tag, paths } | TagChange::Forget { tag, paths } => {
            if let Some(list) = out.get_mut(tag) {
                list.retain(|p| !paths.contains(p));
            }
        }
        TagChange::Follow(moves) => {
            for list in out.values_mut() {
                *list = crate::starred::follow(list, moves);
            }
        }
    }
    out.retain(|_, list| !list.is_empty());
    out
}

/// Every tag the index has on `path`.
pub fn tags_of<'a>(index: &'a Index, path: &'a Path) -> impl Iterator<Item = &'a str> + 'a {
    index.iter().filter(move |(_, paths)| paths.iter().any(|p| p == path)).map(|(tag, _)| tag.as_str())
}

/// For a selection, each tag any of them has, and whether all of them
/// have it — what the Tags sheet draws as a tick or a dash.
pub fn shared(per_file: &[Vec<String>]) -> Vec<(String, bool)> {
    let mut seen: Vec<(String, bool)> = Vec::new();
    for tags in per_file {
        for tag in tags {
            if !seen.iter().any(|(t, _)| t == tag) {
                let all = per_file.iter().all(|other| other.contains(tag));
                seen.push((tag.clone(), all));
            }
        }
    }
    seen
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths(list: &[&str]) -> Vec<PathBuf> {
        list.iter().map(PathBuf::from).collect()
    }

    #[test]
    fn the_attribute_reads_the_way_dolphin_writes_it() {
        assert_eq!(parse(b"work,draft"), ["work", "draft"]);
        assert_eq!(parse(b" work , ,draft,work"), ["work", "draft"], "spaces, empties and repeats are not tags");
        assert_eq!(parse(b""), Vec::<String>::new());
        assert_eq!(format(&["work".into(), "draft".into()]).as_deref(), Some(&b"work,draft"[..]));
        assert_eq!(format(&[]), None, "no tags removes the attribute rather than leaving it empty");
    }

    #[test]
    fn a_tag_with_a_comma_is_refused_rather_than_split() {
        assert!(clean("a,b").is_err());
        assert!(clean("   ").is_err());
        assert_eq!(clean("  holiday 2026 "), Ok("holiday 2026".to_string()));
    }

    #[test]
    fn tagging_twice_is_tagging_once_and_an_emptied_tag_is_gone() {
        let once = apply(&Index::new(), &TagChange::Add { tag: "work".into(), paths: paths(&["/a", "/b"]) });
        let twice = apply(&once, &TagChange::Add { tag: "work".into(), paths: paths(&["/b"]) });
        assert_eq!(twice["work"], paths(&["/a", "/b"]));
        let gone = apply(&twice, &TagChange::Remove { tag: "work".into(), paths: paths(&["/a", "/b"]) });
        assert!(gone.is_empty(), "the sidebar shows no tag with nothing under it");
    }

    #[test]
    fn a_tag_follows_a_move_made_in_files() {
        let index = apply(&Index::new(), &TagChange::Add { tag: "work".into(), paths: paths(&["/docs/a.txt"]) });
        let moved = apply(&index, &TagChange::Follow(vec![("/docs".into(), "/archive/docs".into())]));
        assert_eq!(moved["work"], paths(&["/archive/docs/a.txt"]));
    }

    #[test]
    fn a_tag_found_missing_is_forgotten() {
        let index = apply(&Index::new(), &TagChange::Add { tag: "work".into(), paths: paths(&["/a", "/b"]) });
        let after = apply(&index, &TagChange::Forget { tag: "work".into(), paths: paths(&["/a"]) });
        assert_eq!(after["work"], paths(&["/b"]));
    }

    #[test]
    fn a_selection_says_which_tags_all_of_it_has() {
        let per_file = vec![vec!["work".to_string(), "draft".to_string()], vec!["work".to_string()]];
        assert_eq!(shared(&per_file), [("work".to_string(), true), ("draft".to_string(), false)]);
    }

    #[test]
    fn a_files_tags_come_from_the_index() {
        let index = apply(&Index::new(), &TagChange::Add { tag: "b".into(), paths: paths(&["/x"]) });
        let index = apply(&index, &TagChange::Add { tag: "a".into(), paths: paths(&["/x", "/y"]) });
        assert_eq!(tags_of(&index, Path::new("/x")).collect::<Vec<_>>(), ["a", "b"]);
    }
}
