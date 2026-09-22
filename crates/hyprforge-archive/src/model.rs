//! What is inside an archive, as a tree, independent of which format it
//! came out of.
//!
//! Every reader in this crate produces a flat `Vec<Member>` and hands it
//! to [`Index::build`]. Nothing above this module knows whether the tree
//! it is walking came from a zip's central directory or from a scan of a
//! tar, which is the point: a browser asks for the children of a
//! directory and gets them.
//!
//! # Three things the formats do not agree about
//!
//! **Directories may not be listed.** A zip written by some tools has an
//! entry for `docs/guide.txt` and none for `docs/`. A tar usually has
//! both; a tar built with `--no-recursion` may have neither. So the
//! directories in an [`Index`] are *synthesised* from the paths of the
//! things in them, and any real directory entry found merges into the
//! synthesised one rather than competing with it.
//!
//! **Names are not normalised.** `./docs/guide.txt`, `docs/guide.txt`
//! and `docs//guide.txt` are the same member written three ways, and an
//! index that kept them apart would show three rows and open one file.
//!
//! **A name can repeat.** Appending to a tar does not replace anything:
//! `tar rf` writes a second `notes.txt` after the first, and every tool
//! that unpacks it extracts both in order, so the *last* one is the file
//! that ends up on disk. That is the same last-one-wins rule CLAUDE.md
//! records for `hl.env` and hyprpaper, arriving through a different
//! door, and getting it backwards here would show and extract a
//! superseded version of the file while the real one sat below it in the
//! archive.

use std::collections::{BTreeMap, HashMap};
use std::time::SystemTime;

/// One thing inside an archive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Member {
    /// Normalised: `/`-separated, no leading `/` or `./`, no trailing
    /// `/`, never empty. The root of the archive is the empty string and
    /// is not itself a member.
    pub path: String,
    pub is_dir: bool,
    /// Uncompressed size in bytes. `0` for a directory, and for a member
    /// whose format declines to say — see [`Member::size_known`].
    pub size: u64,
    /// Whether `size` is a number the archive actually stated.
    ///
    /// Separate from `size` for the reason `ItemCount` is separate from
    /// a count in `hyprforge-files-core`: a size nobody knows is not the
    /// number zero, and showing "0 bytes" for a file that has contents
    /// is a claim this crate has no basis for. A streamed zip entry
    /// (bit 3 of the general purpose flag set, sizes in a trailing
    /// descriptor the central directory may not carry) is the ordinary
    /// case.
    pub size_known: bool,
    /// Compressed size, where the format tracks one per member. `None`
    /// for a tar, where compression is applied to the whole stream and
    /// no individual member has a compressed size at all.
    pub compressed: Option<u64>,
    pub modified: Option<SystemTime>,
    /// Unix permission bits, where the format carries them. `None` for
    /// an archive written on a system that had none to record, which is
    /// the usual case for a zip made on Windows.
    pub mode: Option<u32>,
    /// Where a symlink member points, verbatim from the archive.
    pub link_target: Option<String>,
    /// Whether reading this member's contents needs a password.
    ///
    /// A state, not an error, and it has to survive into the listing:
    /// an encrypted archive still lists its names, and someone looking
    /// at a padlock beside a row understands why the preview is empty.
    /// Collapsing it into a read failure would report "could not read"
    /// for a file that is perfectly readable by someone who knows the
    /// password.
    pub encrypted: bool,
}

impl Member {
    /// A plain file member with nothing but a path and a size — the
    /// shape most readers fill in and then adjust.
    pub fn file(path: impl Into<String>, size: u64) -> Member {
        Member {
            path: path.into(),
            is_dir: false,
            size,
            size_known: true,
            compressed: None,
            modified: None,
            mode: None,
            link_target: None,
            encrypted: false,
        }
    }

    pub fn dir(path: impl Into<String>) -> Member {
        Member {
            is_dir: true,
            size: 0,
            ..Member::file(path, 0)
        }
    }

    /// The final path component — what a listing shows.
    pub fn name(&self) -> &str {
        self.path.rsplit('/').next().unwrap_or(&self.path)
    }

    /// The path of the directory holding this member; `""` for one at
    /// the archive's root.
    pub fn parent(&self) -> &str {
        match self.path.rfind('/') {
            Some(cut) => &self.path[..cut],
            None => "",
        }
    }
}

/// A member path as this crate spells it, or `None` if the path leaves
/// the archive.
///
/// The `None` case is the whole reason this returns an `Option`. A
/// member named `../../etc/passwd` is the classic archive attack, and it
/// is *also* not a thing that can appear in a tree rooted at the
/// archive — there is nowhere to draw it. Dropping it here means no
/// caller can navigate to one, and [`crate::extract`] has its own
/// independent guard rather than trusting that this ran.
pub fn normalise(raw: &str) -> Option<String> {
    let mut parts: Vec<&str> = Vec::new();
    for part in raw.split(['/', '\\']) {
        match part {
            // Empty covers a leading `/`, a trailing one, and `a//b`.
            "" | "." => continue,
            ".." => {
                // A `..` that still has somewhere to climb inside the
                // archive is ordinary — `docs/../notes.txt` is
                // `notes.txt`. One with nothing left above it is the
                // escape.
                parts.pop()?;
            }
            part => parts.push(part),
        }
    }
    if parts.is_empty() {
        return None;
    }
    Some(parts.join("/"))
}

/// Everything inside one archive, as a navigable tree.
#[derive(Debug, Clone, Default)]
pub struct Index {
    members: Vec<Member>,
    /// Directory path (`""` for the root) to the indices of what is
    /// directly inside it. `BTreeMap` only so a debug dump of an index
    /// is stable to read; nothing here depends on the ordering, and
    /// listings are sorted by whoever is showing them.
    children: BTreeMap<String, Vec<usize>>,
    by_path: HashMap<String, usize>,
}

impl Index {
    /// Builds the tree: normalises, resolves repeats last-one-wins, and
    /// synthesises the directories no reader reported.
    pub fn build(raw: Vec<Member>) -> Index {
        let mut index = Index::default();

        for mut member in raw {
            let Some(path) = normalise(&member.path) else {
                tracing::warn!(
                    path = %member.path,
                    "skipping an archive member whose path leaves the archive"
                );
                continue;
            };
            // A trailing `/` is how both zip and tar spell "directory",
            // and `normalise` has just removed it — so a directory entry
            // that said so only by its slash needs that remembered here.
            member.is_dir = member.is_dir || member.path.ends_with('/');
            member.path = path;
            index.insert(member);
        }

        // After every real member, so a synthesised directory never
        // displaces a real one carrying a mode and a timestamp.
        index.synthesise_directories();
        index.rebuild_children();
        index
    }

    /// Adds one member, replacing any earlier member with the same path.
    ///
    /// Replacing and not skipping: see the module doc on repeats. The
    /// entry that wins has to be the *last* one, because that is the one
    /// an extraction leaves on disk.
    fn insert(&mut self, member: Member) {
        match self.by_path.get(&member.path) {
            Some(&existing) => self.members[existing] = member,
            None => {
                self.by_path.insert(member.path.clone(), self.members.len());
                self.members.push(member);
            }
        }
    }

    /// Adds a directory member for every ancestor of every member that
    /// does not already have one.
    fn synthesise_directories(&mut self) {
        // Collected first: `insert` borrows `self` mutably, and the
        // ancestors being walked come out of `self.members`.
        let mut wanted: Vec<String> = Vec::new();
        for member in &self.members {
            let mut parent = member.parent();
            while !parent.is_empty() {
                if !self.by_path.contains_key(parent) {
                    wanted.push(parent.to_string());
                }
                parent = match parent.rfind('/') {
                    Some(cut) => &parent[..cut],
                    None => "",
                };
            }
        }
        for path in wanted {
            // Re-checked: two members under the same missing directory
            // each asked for it.
            if !self.by_path.contains_key(&path) {
                self.insert(Member::dir(path));
            }
        }
    }

    fn rebuild_children(&mut self) {
        self.children.clear();
        // Every directory gets an entry even when nothing is in it, so
        // "this directory is empty" and "there is no such directory"
        // stay distinguishable — the rule CLAUDE.md states for a config
        // file that will not read, and `ItemCount` states for a folder
        // nobody could count.
        self.children.insert(String::new(), Vec::new());
        for (i, member) in self.members.iter().enumerate() {
            if member.is_dir {
                self.children.entry(member.path.clone()).or_default();
            }
            self.children.entry(member.parent().to_string()).or_default().push(i);
        }
    }

    /// What is directly inside `dir` — `""` for the archive's root.
    ///
    /// `None` when there is no such directory, which a caller must not
    /// confuse with an empty one.
    pub fn children(&self, dir: &str) -> Option<Vec<&Member>> {
        let dir = if dir.is_empty() {
            String::new()
        } else {
            normalise(dir)?
        };
        let indices = self.children.get(&dir)?;
        Some(indices.iter().map(|&i| &self.members[i]).collect())
    }

    /// One member by path, or `None`.
    pub fn get(&self, path: &str) -> Option<&Member> {
        let path = normalise(path)?;
        self.by_path.get(&path).map(|&i| &self.members[i])
    }

    /// Every member, directories included, in no particular order.
    pub fn members(&self) -> &[Member] {
        &self.members
    }

    /// How many members there are, and how many bytes they hold
    /// uncompressed — what a progress bar needs before it starts, and
    /// what an "extract this?" prompt quotes.
    pub fn totals(&self) -> Totals {
        let files = self.members.iter().filter(|m| !m.is_dir);
        Totals {
            files: files.clone().count(),
            dirs: self.members.iter().filter(|m| m.is_dir).count(),
            bytes: files.map(|m| m.size).sum(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Totals {
    pub files: usize,
    pub dirs: usize,
    /// Uncompressed, and a lower bound rather than a promise: a member
    /// whose size the archive did not state contributes nothing. See
    /// [`Member::size_known`].
    pub bytes: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_member_path_that_climbs_out_of_the_archive_is_dropped() {
        assert_eq!(normalise("../../etc/passwd"), None);
        assert_eq!(normalise("/etc/passwd"), Some("etc/passwd".to_string()));
        assert_eq!(normalise("docs/../notes.txt"), Some("notes.txt".to_string()));
        assert_eq!(normalise("./docs//guide.txt"), Some("docs/guide.txt".to_string()));
        assert_eq!(normalise(""), None);
        assert_eq!(normalise("/"), None);
    }

    #[test]
    fn the_same_name_written_three_ways_is_one_row() {
        let index = Index::build(vec![
            Member::file("./docs/guide.txt", 1),
            Member::file("docs//guide.txt", 2),
            Member::file("docs/guide.txt", 3),
        ]);
        let docs = index.children("docs").expect("docs was synthesised");
        assert_eq!(docs.len(), 1, "three spellings of one path listed {} rows", docs.len());
    }

    /// The rule CLAUDE.md records for `hl.env` and hyprpaper: in a
    /// last-one-wins format, the entry that applies is the last one, and
    /// keeping the first would show a file the archive supersedes.
    #[test]
    fn a_repeated_member_resolves_to_the_last_one_because_that_is_what_unpacks() {
        let index = Index::build(vec![
            Member::file("notes.txt", 10),
            Member::file("notes.txt", 99),
        ]);
        assert_eq!(
            index.get("notes.txt").unwrap().size,
            99,
            "the appended copy is the one that ends up on disk"
        );
    }

    #[test]
    fn directories_nobody_listed_are_synthesised_from_the_paths_under_them() {
        let index = Index::build(vec![Member::file("a/b/c/deep.txt", 1)]);

        let root = index.children("").unwrap();
        assert_eq!(root.len(), 1);
        assert_eq!(root[0].path, "a");
        assert!(root[0].is_dir);

        assert!(index.get("a/b").is_some_and(|m| m.is_dir));
        assert_eq!(index.children("a/b/c").unwrap().len(), 1);
    }

    #[test]
    fn a_real_directory_entry_is_not_replaced_by_a_synthesised_one() {
        let mut real = Member::dir("docs");
        real.mode = Some(0o750);
        let index = Index::build(vec![real, Member::file("docs/guide.txt", 1)]);

        assert_eq!(
            index.get("docs").unwrap().mode,
            Some(0o750),
            "synthesising over a real entry would lose its recorded mode"
        );
    }

    #[test]
    fn a_directory_spelled_only_by_its_trailing_slash_is_still_a_directory() {
        let index = Index::build(vec![Member::file("empty/", 0)]);
        assert!(index.get("empty").is_some_and(|m| m.is_dir));
        assert_eq!(
            index.children("empty").map(|c| c.len()),
            Some(0),
            "an empty directory in the archive must list as empty, not as missing"
        );
    }

    #[test]
    fn a_directory_that_is_not_there_is_not_an_empty_one() {
        let index = Index::build(vec![Member::file("a.txt", 1)]);
        assert!(index.children("nowhere").is_none());
        assert_eq!(index.children("").map(|c| c.len()), Some(1));
    }

    #[test]
    fn totals_count_files_and_directories_apart() {
        let index = Index::build(vec![
            Member::file("a/one.txt", 100),
            Member::file("a/two.txt", 200),
        ]);
        let totals = index.totals();
        assert_eq!(totals.files, 2);
        assert_eq!(totals.dirs, 1, "the synthesised `a` is a directory");
        assert_eq!(totals.bytes, 300);
    }
}
