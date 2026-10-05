//! Type-to-jump: typing a name's first letters moves to it, the way
//! Explorer, Finder and Dolphin do — the other answer to "what does a
//! letter typed at a folder mean", and a preference here
//! (`[behaviour] typing = "jump"`) because the default answer is
//! type-to-search.
//!
//! Pure: the host stamps each key with the time it arrived, the way
//! [`crate::click`] is handed its clock, so the buffer's timeout is
//! testable and the browser still owns no timers.
//!
//! Three rules, each copied from what the others agree on:
//!
//! - **Letters typed together are one prefix.** "ma" lands on
//!   "makefile", not on the first "a…" after the first "m…".
//! - **A pause starts again.** [`RESET`] after the last key, the next one
//!   begins a new prefix.
//! - **The same letter again moves on.** Pressing "d" three times visits
//!   the first three names starting with "d" — a prefix of one repeated
//!   letter is a request for the next match, since no name was being
//!   spelled.

use std::time::{Duration, Instant};

/// How long after a key the next one still extends the prefix.
pub const RESET: Duration = Duration::from_millis(1000);

/// What has been typed so far, and when the last of it arrived.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Typeahead {
    typed: String,
    last: Option<Instant>,
}

impl Typeahead {
    /// Takes a key and says which row to move to, if any matches.
    ///
    /// `names` are the rows in the order they are shown, `focused` the
    /// row the keyboard is on now. A prefix nothing matches keeps the
    /// focus where it is — and keeps the typed letters, so a mistyped
    /// letter is corrected by waiting, not by a jump somewhere random.
    pub fn press<'a>(
        &mut self,
        c: char,
        at: Instant,
        names: impl IntoIterator<Item = &'a str>,
        focused: Option<usize>,
    ) -> Option<usize> {
        let fresh = self.last.is_none_or(|last| at.saturating_duration_since(last) >= RESET);
        if fresh {
            self.typed.clear();
        }
        self.last = Some(at);
        self.typed.extend(c.to_lowercase());

        let names: Vec<String> = names.into_iter().map(str::to_lowercase).collect();
        if names.is_empty() {
            return None;
        }
        let mut letters = self.typed.chars();
        let first = letters.next()?;
        let repeated = self.typed.chars().count() > 1 && letters.all(|l| l == first);

        // A repeat of one letter cycles from the row after the focus; a
        // fresh prefix — or one still being spelled — starts at the focus
        // itself, so the "m" of "ma" does not move off a row "a" keeps.
        let (prefix, start) = if repeated {
            (first.to_string(), focused.map_or(0, |f| f + 1))
        } else if self.typed.chars().count() == 1 {
            (self.typed.clone(), focused.map_or(0, |f| f + 1))
        } else {
            (self.typed.clone(), focused.unwrap_or(0))
        };
        let n = names.len();
        (0..n).map(|k| (start + k) % n).find(|&i| names.get(i).is_some_and(|name| name.starts_with(&prefix)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NAMES: [&str; 6] = ["Documents", "Downloads", "desktop.ini", "Makefile", "main.rs", "Music"];

    #[test]
    fn letters_typed_together_spell_one_prefix() {
        let mut t = Typeahead::default();
        let now = Instant::now();
        assert_eq!(t.press('m', now, NAMES, None), Some(3), "the first m…");
        assert_eq!(t.press('a', now + Duration::from_millis(200), NAMES, Some(3)), Some(3), "ma: still Makefile");
        assert_eq!(t.press('i', now + Duration::from_millis(400), NAMES, Some(3)), Some(4), "mai: main.rs");
    }

    #[test]
    fn matching_ignores_case() {
        let mut t = Typeahead::default();
        assert_eq!(t.press('D', Instant::now(), NAMES, None), Some(0));
    }

    #[test]
    fn the_same_letter_again_moves_to_the_next_match_and_wraps() {
        let mut t = Typeahead::default();
        let now = Instant::now();
        let ms = |n| now + Duration::from_millis(n);
        let mut at = t.press('d', ms(0), NAMES, None);
        assert_eq!(at, Some(0));
        at = t.press('d', ms(100), NAMES, at);
        assert_eq!(at, Some(1));
        at = t.press('d', ms(200), NAMES, at);
        assert_eq!(at, Some(2));
        at = t.press('d', ms(300), NAMES, at);
        assert_eq!(at, Some(0), "past the last d…, back to the first");
    }

    #[test]
    fn a_pause_starts_a_new_prefix() {
        let mut t = Typeahead::default();
        let now = Instant::now();
        assert_eq!(t.press('m', now, NAMES, None), Some(3));
        // "mu" would be Music; after a pause the "u" stands alone and
        // matches nothing.
        assert_eq!(t.press('u', now + RESET, NAMES, Some(3)), None);
    }

    #[test]
    fn a_prefix_nothing_matches_leaves_the_focus_alone() {
        let mut t = Typeahead::default();
        assert_eq!(t.press('z', Instant::now(), NAMES, Some(2)), None);
    }

    #[test]
    fn a_single_letter_moves_off_the_row_already_on_it() {
        let mut t = Typeahead::default();
        // On Documents, "d" goes to the next d…, as Explorer does.
        assert_eq!(t.press('d', Instant::now(), NAMES, Some(0)), Some(1));
    }
}
