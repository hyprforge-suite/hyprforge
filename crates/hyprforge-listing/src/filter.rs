//! What stays out of a listing, kept separate from how it was read or
//! ordered. Pure functions over `&[Entry]`/`&Entry`, same reasoning as
//! [`crate::sort`].

use crate::types::Entry;

/// Whether `entry` should be hidden — currently just the usual Unix
/// dotfile convention ([`Entry::hidden`], set when the entry was built).
///
/// A `.hidden` file naming further entries (honoured by Nautilus, Thunar
/// and Dolphin) is a real convention, deliberately **not** implemented
/// here yet: no file manager is installed on this machine to
/// interoperate with, and there is no `.hidden` file anywhere under this
/// user's home directory to check a reading of the format against —
/// building it now would be speculative in exactly the way CLAUDE.md
/// argues against, not a convention this crate has an actual case for
/// today. If a later need for it turns up (importing a directory tree
/// someone curated with Nautilus, say), this is the function to extend:
/// take the parsed contents of the directory's own `.hidden` file as a
/// second argument alongside `entry`.
pub fn is_hidden(entry: &Entry) -> bool {
    entry.hidden
}

/// Keeps only the entries that should show given `show_hidden`.
pub fn filter_hidden(entries: Vec<Entry>, show_hidden: bool) -> Vec<Entry> {
    if show_hidden {
        return entries;
    }
    entries.into_iter().filter(|e| !is_hidden(e)).collect()
}

/// Case-insensitive substring match against an entry's name only.
///
/// Deliberately **not** doing yet: matching on path (so typing part of a
/// parent directory's name would not find a file inside it), fuzzy /
/// subsequence matching (so "rdme" would not find "readme.txt"), or
/// searching file *contents*. All three are reasonable things to want
/// from a file browser's search box; none of them belongs in a predicate
/// this cheap to call on every entry in a large directory on every
/// keystroke. Content search in particular is far more expensive, and
/// the file manager answers it in its bounded search walk
/// (`hyprforge-files-core`'s `content` module), off the UI thread —
/// never here.
/// `query` must already be lowercased — see [`filter_query`], which is
/// what does it. Taking it pre-folded rather than folding it here is the
/// difference between lowercasing the search box's contents once and
/// lowercasing it again for every entry in the directory, on every
/// keystroke.
pub fn matches_query(entry: &Entry, query: &str) -> bool {
    if query.is_empty() {
        return true;
    }
    entry.name.to_lowercase().contains(query)
}

pub fn filter_query(entries: Vec<Entry>, query: &str) -> Vec<Entry> {
    if query.is_empty() {
        return entries;
    }
    let query = query.to_lowercase();
    entries.into_iter().filter(|e| matches_query(e, &query)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::EntryKind;
    use std::path::PathBuf;

    fn entry(name: &str) -> Entry {
        Entry {
            name: name.to_string(),
            path: PathBuf::from("/").join(name),
            is_dir: false,
            size: crate::types::EntrySize::Bytes(0),
            modified: None,
            is_symlink: false,
            link_broken: false,
            hidden: name.starts_with('.') && name != "." && name != "..",
            kind: EntryKind::classify(false, name),
            // Ownership and permissions are fixtures here: these
            // helpers build entries for tests about names, sizes and
            // ordering, none of which read them.
            mode: 0o644,
            uid: 1000,
            owner: Some("alex".to_string()),
            origin: None,
            packed: None,
        }
    }

    #[test]
    fn a_dotfile_is_hidden_by_default() {
        assert!(is_hidden(&entry(".bashrc")));
    }

    /// The distinction the brief calls out: a name that merely *contains*
    /// a dot is not the same as one that *starts* with one.
    #[test]
    fn a_name_that_only_contains_a_dot_is_not_hidden() {
        assert!(!is_hidden(&entry("archive.tar.gz")));
    }

    #[test]
    fn filter_hidden_with_show_hidden_true_keeps_everything() {
        let entries = vec![entry(".git"), entry("readme.md")];
        let kept = filter_hidden(entries, true);
        assert_eq!(kept.len(), 2);
    }

    #[test]
    fn filter_hidden_with_show_hidden_false_drops_only_dotfiles() {
        let entries = vec![entry(".git"), entry("readme.md")];
        let kept = filter_hidden(entries, false);
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].name, "readme.md");
    }

    /// Through `filter_query` and not `matches_query`: case folding is
    /// `filter_query`'s job (it does it once for the whole listing
    /// rather than once per entry), so that is the level at which
    /// "searching is case-insensitive" is actually a true statement.
    #[test]
    fn query_matching_is_case_insensitive_substring_on_name() {
        let names = |query| {
            filter_query(vec![entry("ReadMe.TXT")], query)
                .into_iter()
                .map(|e| e.name)
                .collect::<Vec<_>>()
        };
        assert_eq!(names("readme"), ["ReadMe.TXT"]);
        assert_eq!(names("ME.tx"), ["ReadMe.TXT"], "an upper-case query still matches");
        assert!(names("license").is_empty());
    }

    #[test]
    fn an_empty_query_matches_everything() {
        assert!(matches_query(&entry("anything.txt"), ""));
        assert_eq!(filter_query(vec![entry("anything.txt")], "").len(), 1);
    }
}
