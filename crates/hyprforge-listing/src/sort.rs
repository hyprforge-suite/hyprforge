//! How a listing gets ordered, kept separate from how it is read.
//!
//! Pure functions over `&[Entry]` in, `Vec<Entry>`/ordering out — no
//! [`crate::backend::FsBackend`] in sight, so these are exercised with
//! hand-built entries and never a real directory.

use crate::types::{Entry, EntryKind};
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;

// These two derive (De)Serialize directly. `crate::prefs` used to carry
// a mirror of each — `SortColumnPref`/`SortDirectionPref` — plus four
// `From` impls, so that this module "did not need to know about TOML".
// It still does not: a `#[derive]` on a fieldless enum says how it
// spells itself, not where it gets written, and the mirrors' real cost
// was six variants to keep in step by hand across two files for a
// separation nothing was enforcing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SortColumn {
    Name,
    Size,
    Modified,
    Kind,
    Owner,
    Permissions,
    /// A trashed item's original folder.
    Origin,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SortDirection {
    Ascending,
    Descending,
}

/// Sorts `entries` in place by `column`/`direction`, optionally with
/// every directory ahead of every file regardless of what the column
/// says about them individually.
///
/// `directories_first` is a separate toggle rather than folded into
/// `SortColumn::Kind`, because a person sorting by size or name still
/// overwhelmingly wants folders grouped at the top — that is the
/// behaviour of every mainstream file manager, and conflating "group
/// folders" with "the *only* way to see kind-based order" would make
/// picking, say, Size-descending silently undo it.
pub fn sort(entries: &mut [Entry], column: SortColumn, direction: SortDirection, directories_first: bool) {
    entries.sort_by(|a, b| row_order(a, b, column, direction, directories_first));
}

/// [`sort`] over a list of *indices* into `entries`, which is how a
/// browser holds its current view: one owned listing, and an ordering of
/// the subset being shown.
///
/// Same comparator, so "sorted" cannot mean two different things
/// depending on which entry point a caller reached for.
pub fn sort_indices(
    view: &mut [usize],
    entries: &[Entry],
    column: SortColumn,
    direction: SortDirection,
    directories_first: bool,
) {
    view.sort_by(|&a, &b| {
        row_order(&entries[a], &entries[b], column, direction, directories_first)
    });
}

fn row_order(
    a: &Entry,
    b: &Entry,
    column: SortColumn,
    direction: SortDirection,
    directories_first: bool,
) -> Ordering {
    {
        if directories_first {
            // `is_dir` true sorts first regardless of `direction` — a
            // reversed size sort should put the *biggest file* first
            // among files, not send folders to the bottom, so this
            // comparison is deliberately made before `direction` is
            // applied to anything else.
            let by_dir = b.is_dir.cmp(&a.is_dir);
            if by_dir != Ordering::Equal {
                return by_dir;
            }
        }
        let ordering = compare(a, b, column);
        match direction {
            SortDirection::Ascending => ordering,
            SortDirection::Descending => ordering.reverse(),
        }
    }
}

fn compare(a: &Entry, b: &Entry, column: SortColumn) -> Ordering {
    match column {
        SortColumn::Name => natural_compare(&a.name, &b.name),
        // `EntrySize`'s own ordering — so the column sorts by exactly
        // what it displays. It used to sort by a `u64` that was always
        // `0` for a directory while the cell showed an item count, which
        // meant clicking the Size header ordered folders by name.
        SortColumn::Size => a.size.cmp(&b.size).then_with(|| natural_compare(&a.name, &b.name)),
        SortColumn::Modified => a
            .modified
            .cmp(&b.modified)
            .then_with(|| natural_compare(&a.name, &b.name)),
        SortColumn::Kind => kind_rank(a.kind)
            .cmp(&kind_rank(b.kind))
            .then_with(|| natural_compare(&a.name, &b.name)),
        // By the name where there is one, and by the uid where there is
        // not — so files owned by users this system cannot name group
        // together rather than scattering through the alphabet. The
        // same order `format_owner` displays, which is the rule every
        // sortable column here follows.
        SortColumn::Owner => owner_key(a).cmp(&owner_key(b)).then_with(|| natural_compare(&a.name, &b.name)),
        // By the bits, not by the rendered string: `drwxr-xr-x` sorts
        // `d` before `-`, which would order by file type rather than by
        // permission and put every directory in one lump whichever way
        // the arrow points.
        SortColumn::Permissions => a.mode.cmp(&b.mode).then_with(|| natural_compare(&a.name, &b.name)),
        SortColumn::Origin => a.origin.cmp(&b.origin).then_with(|| natural_compare(&a.name, &b.name)),
    }
}

/// Sort key for the Owner column: named owners first, alphabetically,
/// then unnamed ones by uid.
fn owner_key(entry: &Entry) -> (u8, &str, u32) {
    match entry.owner.as_deref() {
        Some(name) => (0, name, entry.uid),
        None => (1, "", entry.uid),
    }
}

/// An arbitrary but fixed order for grouping by kind — folders first
/// (though `directories_first` normally handles that on its own),
/// otherwise alphabetical-ish by the category name so the order is at
/// least predictable and stable across runs.
fn kind_rank(kind: EntryKind) -> u8 {
    match kind {
        EntryKind::Folder => 0,
        EntryKind::Image => 1,
        EntryKind::Document => 2,
        EntryKind::Code => 3,
        EntryKind::Archive => 4,
        EntryKind::Audio => 5,
        EntryKind::Video => 6,
        EntryKind::Other => 7,
    }
}

/// Case-insensitive natural-order comparison: `"file2"` before
/// `"file10"`, the way every mainstream file manager already sorts,
/// because a plain byte/codepoint compare puts `"file10"` before
/// `"file2"` (`'1' < '2'`) and that reads as broken to anyone who has
/// used a file manager before.
///
/// Walks both strings once, comparing either a run of ASCII digits
/// (as a number, so `"10"` > `"2"`) or a single non-digit character
/// (case-folded) at each step, and falls back to a plain
/// case-insensitive compare only if the two strings are otherwise
/// identical under this rule — see the fallback's own comment for why
/// that step exists at all.
pub fn natural_compare(a: &str, b: &str) -> Ordering {
    let (raw_a, raw_b) = (a, b);
    let mut a = a.chars().peekable();
    let mut b = b.chars().peekable();

    // Walked case-insensitively first, because that is the order a
    // person reads. `raw` decides only when that walk ties — see the
    // bottom of this function.
    loop {
        match (a.peek(), b.peek()) {
            (None, None) => return raw_tiebreak(raw_a, raw_b),
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(ca), Some(cb)) => {
                if ca.is_ascii_digit() && cb.is_ascii_digit() {
                    let na = take_number(&mut a);
                    let nb = take_number(&mut b);
                    // A run of digits can be longer than fits a u64 in
                    // principle (a filename of a thousand digits), so
                    // compare digit-run *lengths* first — after
                    // dropping leading zeros, more digits always means
                    // a bigger number in base 10 — before falling back
                    // to a lexical compare of equal-length runs. This
                    // avoids ever parsing into an integer type that
                    // could overflow.
                    let cmp = compare_digit_runs(&na, &nb);
                    if cmp != Ordering::Equal {
                        return cmp;
                    }
                    // Equal numeric value and equal digit-run length
                    // (so, character-for-character identical runs) —
                    // keep walking rather than treating the whole
                    // strings as equal yet, so a later differing
                    // character still decides it.
                } else {
                    let fold = |c: char| c.to_lowercase().next().unwrap_or(c);
                    let ordering = fold(*ca).cmp(&fold(*cb));
                    if ordering != Ordering::Equal {
                        return ordering;
                    }
                    a.next();
                    b.next();
                }
            }
        }
    }
}

/// Consumes one run of ASCII digits from `iter` and returns it as a
/// string — kept as text rather than parsed to a number so a run longer
/// than any integer type can still be compared (see the caller).
fn take_number(iter: &mut std::iter::Peekable<std::str::Chars>) -> String {
    let mut digits = String::new();
    while let Some(c) = iter.peek() {
        if c.is_ascii_digit() {
            digits.push(*c);
            iter.next();
        } else {
            break;
        }
    }
    digits
}

/// The last word when two names are equal to a reader: `"APPLE"` and
/// `"apple"` fold to the same thing, and so do `"a"` and `"A"`.
///
/// Without this the comparator returns `Equal` for them, and a stable
/// sort then leaves whichever the *filesystem* listed first at the top —
/// `read_dir` order, which is neither alphabetical nor stable across a
/// directory being modified. Two files differing only in case would
/// swap places for no reason a user could see, on a refresh they did
/// not ask for. Falling back to the raw bytes costs nothing when the
/// names differ (the common case returns before reaching here) and makes
/// the order total, so a listing is the same every time it is drawn.
fn raw_tiebreak(a: &str, b: &str) -> Ordering {
    a.cmp(b)
}

/// Compares two runs of ASCII digits numerically (as if parsed, but
/// without risking an overflow on an implausibly long run — see the
/// caller), with a tiebreak for numerically-equal runs of different
/// length, e.g. `"7"` vs `"007"`: the run with fewer leading zeros
/// sorts first, which reads as "the plainer spelling comes first" and
/// keeps this a strict order (two digit runs that are not
/// character-for-character identical never compare `Equal`).
fn compare_digit_runs(a: &str, b: &str) -> Ordering {
    let a_trimmed = a.trim_start_matches('0');
    let b_trimmed = b.trim_start_matches('0');
    a_trimmed
        .len()
        .cmp(&b_trimmed.len())
        .then_with(|| a_trimmed.cmp(b_trimmed))
        .then_with(|| a.len().cmp(&b.len()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{EntryKind, EntrySize, ItemCount};
    use std::path::PathBuf;
    use std::time::{Duration, SystemTime};

    fn entry(name: &str, is_dir: bool, size: u64, modified_secs: u64) -> Entry {
        Entry {
            name: name.to_string(),
            path: PathBuf::from("/").join(name),
            is_dir,
            // A directory's "size" is a child count, so a test that
            // hands one a number means that many items — see
            // `EntrySize`.
            size: if is_dir {
                EntrySize::Items(ItemCount::Known(size as usize))
            } else {
                EntrySize::Bytes(size)
            },
            modified: Some(SystemTime::UNIX_EPOCH + Duration::from_secs(modified_secs)),
            is_symlink: false,
            link_broken: false,
            hidden: name.starts_with('.'),
            kind: EntryKind::classify(is_dir, name),
            // Ownership and permissions are fixtures here: these
            // helpers build entries for tests about names, sizes and
            // ordering, none of which read them.
            mode: 0o644,
            uid: 1000,
            owner: Some("alex".to_string()),
            origin: None,
        }
    }

    fn names(entries: &[Entry]) -> Vec<&str> {
        entries.iter().map(|e| e.name.as_str()).collect()
    }

    // --- natural_compare ---------------------------------------------------

    #[test]
    fn file2_sorts_before_file10() {
        assert_eq!(natural_compare("file2.txt", "file10.txt"), Ordering::Less);
    }

    #[test]
    fn natural_compare_is_case_insensitive() {
        assert_eq!(natural_compare("Banana", "apple"), Ordering::Greater);
        // Folding decides the *reading* order; when that ties, the raw
        // bytes decide, so the result is deterministic rather than
        // whatever the filesystem happened to list first. `A` (0x41)
        // sorts before `a` (0x61).
        assert_eq!(natural_compare("APPLE", "apple"), Ordering::Less);
    }

    #[test]
    fn two_names_differing_only_in_case_have_one_definite_order() {
        // The property that matters is not which one wins, but that the
        // answer does not depend on input order — a stable sort over a
        // comparator returning `Equal` would hand back `read_dir`'s
        // order, which changes as a directory is written to.
        let forwards = natural_compare("README", "readme");
        let backwards = natural_compare("readme", "README");
        assert_ne!(forwards, Ordering::Equal);
        assert_eq!(forwards, backwards.reverse(), "the comparator must be antisymmetric");
    }

    #[test]
    fn a_column_full_of_ties_falls_back_to_name_rather_than_to_disk_order() {
        // Size and time tie constantly — a directory of empty files, or
        // a dozen written in the same second. Every comparator here
        // tie-breaks on the name, so the answer never depends on the
        // order `read_dir` happened to return, which is neither
        // alphabetical nor stable across the directory being written to.
        // Sort stability is therefore not load-bearing anywhere in this
        // module: every comparator is a total order on its own. That is
        // a stronger guarantee than stability and worth keeping.
        let mut entries = vec![
            entry("charlie.txt", false, 0, 5),
            entry("alpha.txt", false, 0, 5),
            entry("bravo.txt", false, 0, 5),
        ];
        sort(&mut entries, SortColumn::Size, SortDirection::Ascending, false);
        assert_eq!(
            names(&entries),
            vec!["alpha.txt", "bravo.txt", "charlie.txt"],
            "every size is 0, so the name decides"
        );

        // And the same input in a different order lands the same way,
        // which is the property that actually matters.
        let mut shuffled = vec![
            entry("bravo.txt", false, 0, 5),
            entry("charlie.txt", false, 0, 5),
            entry("alpha.txt", false, 0, 5),
        ];
        sort(&mut shuffled, SortColumn::Size, SortDirection::Ascending, false);
        assert_eq!(names(&shuffled), names(&entries), "input order must not survive into the result");
    }

    #[test]
    fn leading_zeros_break_a_numeric_tie_rather_than_being_ignored_outright() {
        // "7" and "007" are numerically equal; "7" is the shorter digit
        // run and sorts first once the numeric value ties, which is what
        // a person expects from "the one with more leading zeros sorts
        // after". Together with `raw_tiebreak` this keeps the whole
        // comparison a total order: two strings that are not identical
        // never compare `Equal`, so a listing never depends on the order
        // the filesystem happened to hand entries back in.
        assert_eq!(natural_compare("7", "007"), Ordering::Less);
        assert_eq!(natural_compare("file7.txt", "file007.txt"), Ordering::Less);
    }

    #[test]
    fn a_long_digit_run_does_not_panic_or_misorder() {
        let a = format!("file{}", "9".repeat(40));
        let b = format!("file{}", "1".repeat(41));
        // 41 nines-minus-one-digit vs a 41-digit run: more digits wins,
        // regardless of what a u64/u128 parse of either would do.
        assert_eq!(natural_compare(&a, &b), Ordering::Less);
    }

    #[test]
    fn multiple_numeric_runs_are_each_compared_numerically() {
        assert_eq!(natural_compare("v2.9.txt", "v2.10.txt"), Ordering::Less);
    }

    // --- directories-first, across every column -----------------------

    #[test]
    fn directories_first_holds_when_sorting_by_name() {
        let mut entries = vec![entry("zzz_file", false, 1, 1), entry("aaa_dir", true, 0, 1)];
        sort(&mut entries, SortColumn::Name, SortDirection::Ascending, true);
        assert_eq!(names(&entries), vec!["aaa_dir", "zzz_file"]);
    }

    #[test]
    fn directories_first_holds_when_sorting_by_size_descending() {
        let mut entries = vec![
            entry("big_file", false, 1000, 1),
            entry("small_dir", true, 0, 1),
        ];
        sort(&mut entries, SortColumn::Size, SortDirection::Descending, true);
        assert_eq!(
            names(&entries),
            vec!["small_dir", "big_file"],
            "a folder outranks even the largest file when directories-first is on"
        );
    }

    #[test]
    fn directories_first_holds_when_sorting_by_modified_time() {
        let mut entries = vec![entry("old_file", false, 1, 1), entry("new_dir", true, 0, 999)];
        sort(&mut entries, SortColumn::Modified, SortDirection::Ascending, true);
        assert_eq!(names(&entries), vec!["new_dir", "old_file"]);
    }

    #[test]
    fn directories_first_holds_when_sorting_by_kind() {
        let mut entries = vec![entry("photo.png", false, 1, 1), entry("a_dir", true, 0, 1)];
        sort(&mut entries, SortColumn::Kind, SortDirection::Ascending, true);
        assert_eq!(names(&entries), vec!["a_dir", "photo.png"]);
    }

    #[test]
    fn without_directories_first_size_order_ignores_kind_entirely() {
        let mut entries = vec![entry("tiny_dir", true, 0, 1), entry("big_file", false, 500, 1)];
        sort(&mut entries, SortColumn::Size, SortDirection::Descending, false);
        assert_eq!(
            names(&entries),
            vec!["big_file", "tiny_dir"],
            "with the toggle off, a directory's synthetic size (0) sorts like any other size"
        );
    }

    #[test]
    fn reversing_direction_does_not_disturb_directories_first() {
        let mut entries = vec![
            entry("b_file", false, 2, 1),
            entry("a_file", false, 1, 1),
            entry("z_dir", true, 0, 1),
        ];
        sort(&mut entries, SortColumn::Name, SortDirection::Descending, true);
        assert_eq!(
            names(&entries),
            vec!["z_dir", "b_file", "a_file"],
            "descending must reverse the within-group order, not send directories to the bottom"
        );
    }
}
