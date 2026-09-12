//! Which entry wins when two of them configure the same thing.
//!
//! Several generated formats here are last-one-wins: `hl.env` for a given
//! name, a hyprpaper `wallpaper` block for a given monitor, a hyprsunset
//! profile at a given time. An editor that lets the user keep both has to
//! say which one is dead — and it has to pick the right one.
//!
//! That sounds cosmetic and is not. Every one of those generators skips
//! whatever its `invalid()` reported, so flagging the *winner* does not
//! merely mislabel a row: it drops the value that applies and writes the
//! superseded one instead. Two crates had that backwards in the same way,
//! which is why the rule lives here rather than being written out a third
//! time.

/// The indices of every entry that a later entry supersedes.
///
/// `key` returns `None` for an entry that takes no part — a disabled row
/// neither wins nor loses, and must not knock out the row above it.
///
/// The result is ascending, so it can be reported against rows in the
/// order they are displayed.
pub fn superseded<T, K: PartialEq>(items: &[T], key: impl Fn(&T) -> Option<K>) -> Vec<usize> {
    let keyed: Vec<(usize, K)> =
        items.iter().enumerate().filter_map(|(i, item)| Some((i, key(item)?))).collect();
    keyed
        .iter()
        .filter(|(i, k)| keyed.iter().any(|(j, other)| j > i && other == k))
        .map(|(i, _)| *i)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys(items: &[&str]) -> Vec<usize> {
        superseded(items, |s| Some(*s))
    }

    #[test]
    fn the_loser_is_the_earlier_entry_not_the_one_in_effect() {
        assert_eq!(keys(&["A", "A"]), vec![0]);
    }

    #[test]
    fn every_superseded_entry_is_reported_not_only_the_first() {
        assert_eq!(keys(&["A", "A", "A"]), vec![0, 1]);
    }

    #[test]
    fn entries_that_do_not_collide_are_left_alone() {
        assert_eq!(keys(&["A", "B", "C"]), Vec::<usize>::new());
        assert_eq!(keys(&[]), Vec::<usize>::new());
    }

    #[test]
    fn interleaved_keys_each_keep_their_own_last_one() {
        assert_eq!(keys(&["A", "B", "A", "B"]), vec![0, 1]);
    }

    /// A row that takes no part — switched off, or otherwise not written —
    /// must neither be flagged nor knock out the row above it, or
    /// disabling a duplicate would silently retire the wrong value.
    #[test]
    fn an_entry_with_no_key_neither_wins_nor_loses() {
        let rows = [("A", true), ("A", false), ("A", true)];
        let out = superseded(&rows, |(k, on)| on.then_some(*k));
        assert_eq!(out, vec![0], "only the first enabled row is superseded");
    }
}
