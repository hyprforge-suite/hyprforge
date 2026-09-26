//! The slideshow's order and pace — mockup `1e`.
//!
//! What the window does with it (fullscreen, a timer, controls that fade)
//! is in `main.rs`. What is here is the part with edge cases: where the
//! show goes after the last picture, what shuffling does to the one on
//! screen, and which intervals exist.

/// How long each picture stays up.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Interval {
    Three,
    #[default]
    Five,
    Ten,
    Thirty,
}

impl Interval {
    pub const ALL: [Interval; 4] = [Interval::Three, Interval::Five, Interval::Ten, Interval::Thirty];

    pub fn seconds(self) -> u64 {
        match self {
            Interval::Three => 3,
            Interval::Five => 5,
            Interval::Ten => 10,
            Interval::Thirty => 30,
        }
    }

    /// From a remembered number of seconds; anything else is the default,
    /// so a hand-edited `photos.toml` cannot produce a zero-second show.
    pub fn from_seconds(seconds: u64) -> Interval {
        Interval::ALL.into_iter().find(|i| i.seconds() == seconds).unwrap_or_default()
    }
}

impl std::fmt::Display for Interval {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Every {} s", self.seconds())
    }
}

/// A show in progress: the order it runs in and where it is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Show {
    /// Folder indices, in the order they will be shown.
    order: Vec<usize>,
    at: usize,
    pub looping: bool,
    pub paused: bool,
    shuffled: bool,
}

impl Show {
    /// A show over `total` items starting at `start`, in folder order.
    pub fn new(total: usize, start: usize, looping: bool) -> Show {
        Show {
            order: (0..total).collect(),
            at: start.min(total.saturating_sub(1)),
            looping,
            paused: false,
            shuffled: false,
        }
    }

    /// The folder index on screen.
    pub fn current(&self) -> Option<usize> {
        self.order.get(self.at).copied()
    }

    /// "12 / 48" — the position in the *show*, which after a shuffle is
    /// not the position in the folder.
    pub fn position(&self) -> (usize, usize) {
        (self.at + 1, self.order.len())
    }

    pub fn is_shuffled(&self) -> bool {
        self.shuffled
    }

    /// The next picture, or `None` when the show has run out — which
    /// ends it, rather than sitting on the last picture forever.
    pub fn advance(&mut self) -> Option<usize> {
        if self.at + 1 < self.order.len() {
            self.at += 1;
        } else if self.looping && !self.order.is_empty() {
            self.at = 0;
        } else {
            return None;
        }
        self.current()
    }

    /// One back, stopping at the start (or wrapping, when looping).
    pub fn back(&mut self) -> Option<usize> {
        if self.at > 0 {
            self.at -= 1;
        } else if self.looping && !self.order.is_empty() {
            self.at = self.order.len() - 1;
        }
        self.current()
    }

    /// Shuffles or un-shuffles, keeping the picture on screen on screen:
    /// turning shuffle on mid-show must not jump somewhere else.
    ///
    /// `seed` comes from the clock in the window; a test passes a fixed
    /// one. No `rand` dependency for a slideshow order — a xorshift is
    /// plenty to stop two shows looking alike.
    pub fn set_shuffled(&mut self, on: bool, seed: u64) {
        let Some(current) = self.current() else { return };
        if on == self.shuffled {
            return;
        }
        self.shuffled = on;
        let mut rest: Vec<usize> = (0..self.order.len()).filter(|&i| i != current).collect();
        if on {
            let mut state = seed | 1;
            for i in (1..rest.len()).rev() {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                rest.swap(i, (state % (i as u64 + 1)) as usize);
            }
            self.order = std::iter::once(current).chain(rest).collect();
            self.at = 0;
        } else {
            self.order = (0..self.order.len()).collect();
            self.at = current;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_show_that_does_not_loop_ends_after_the_last_picture() {
        let mut s = Show::new(3, 1, false);
        assert_eq!(s.advance(), Some(2));
        assert_eq!(s.advance(), None);
    }

    #[test]
    fn a_looping_show_goes_round_again() {
        let mut s = Show::new(3, 2, true);
        assert_eq!(s.advance(), Some(0));
        assert_eq!(s.back(), Some(2));
    }

    /// The picture on screen is the one the show starts from, and
    /// shuffling keeps it there.
    #[test]
    fn shuffling_keeps_the_picture_on_screen() {
        let mut s = Show::new(20, 7, false);
        s.set_shuffled(true, 42);
        assert_eq!(s.current(), Some(7));
        assert_eq!(s.position(), (1, 20));
        s.set_shuffled(false, 0);
        assert_eq!(s.current(), Some(7));
        assert_eq!(s.position(), (8, 20));
    }

    /// A shuffled show still shows every picture exactly once.
    #[test]
    fn a_shuffled_show_visits_everything_once() {
        let mut s = Show::new(50, 0, false);
        s.set_shuffled(true, 0xDEADBEEF);
        let mut seen = vec![s.current().unwrap()];
        while let Some(i) = s.advance() {
            seen.push(i);
        }
        seen.sort_unstable();
        assert_eq!(seen, (0..50).collect::<Vec<_>>());
    }

    #[test]
    fn a_hand_edited_interval_cannot_make_a_zero_second_show() {
        assert_eq!(Interval::from_seconds(0), Interval::Five);
        assert_eq!(Interval::from_seconds(10), Interval::Ten);
        assert_eq!(Interval::Five.to_string(), "Every 5 s");
    }

    #[test]
    fn an_empty_show_has_nothing_on_screen() {
        let mut s = Show::new(0, 0, true);
        assert_eq!(s.current(), None);
        assert_eq!(s.advance(), None);
    }
}
