//! Which thumbnails the strip shows.
//!
//! Pure over a count and an index, so the awkward cases — the first
//! picture, the last one, a folder shorter than the strip — are tests
//! rather than things to notice by hand later.
//!
//! The rule it implements: the current picture stays near the middle
//! while there is folder on both sides of it, and the window stops at
//! the ends rather than showing empty space. A strip that let the
//! selection sit at the very edge would make "what is next" invisible
//! exactly when someone is paging.

/// The run of items the strip is showing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Window {
    /// First index shown, inclusive.
    pub start: usize,
    /// One past the last index shown.
    pub end: usize,
}

impl Window {
    pub fn len(self) -> usize {
        self.end.saturating_sub(self.start)
    }

    pub fn is_empty(self) -> bool {
        self.start >= self.end
    }

    pub fn contains(self, index: usize) -> bool {
        index >= self.start && index < self.end
    }

    pub fn indices(self) -> impl Iterator<Item = usize> {
        self.start..self.end
    }
}

/// The window of `visible` items centred as near as possible on
/// `cursor`, within a folder of `total`.
pub fn window(total: usize, cursor: usize, visible: usize) -> Window {
    if total == 0 || visible == 0 {
        return Window { start: 0, end: 0 };
    }
    if visible >= total {
        return Window { start: 0, end: total };
    }

    // Half the window to the left, then pulled back inside the folder.
    // Saturating rather than signed arithmetic: near the start the ideal
    // position is negative, and clamping it to zero is exactly right.
    let half = visible / 2;
    let start = cursor.saturating_sub(half).min(total - visible);
    Window { start, end: start + visible }
}

/// How many thumbnails fit across `width` logical pixels.
///
/// Its own function because it is the one place the strip's geometry
/// lives, and a view that computed it inline would be a view with
/// arithmetic in it.
pub fn fits(width: f32, thumbnail: f32, gap: f32) -> usize {
    if width <= 0.0 || thumbnail <= 0.0 {
        return 0;
    }
    // n thumbnails and n-1 gaps: (n * t) + ((n - 1) * g) <= width
    let n = ((width + gap) / (thumbnail + gap)).floor();
    n.max(0.0) as usize
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_folder_shorter_than_the_strip_shows_all_of_it() {
        let w = window(3, 1, 9);
        assert_eq!(w, Window { start: 0, end: 3 });
    }

    #[test]
    fn the_current_picture_sits_in_the_middle_when_it_can() {
        let w = window(100, 50, 9);
        assert!(w.contains(50));
        // Four either side of the centre.
        assert_eq!(w, Window { start: 46, end: 55 });
    }

    /// At the start the window cannot centre, and must not try — showing
    /// empty space to the left of the first picture would be a gap where
    /// the folder simply ends.
    #[test]
    fn at_the_start_the_strip_stops_rather_than_showing_nothing() {
        let w = window(100, 0, 9);
        assert_eq!(w, Window { start: 0, end: 9 });
        assert!(w.contains(0));
    }

    #[test]
    fn at_the_end_the_strip_stops_too() {
        let w = window(100, 99, 9);
        assert_eq!(w, Window { start: 91, end: 100 });
        assert!(w.contains(99));
    }

    /// The property that matters more than any exact arithmetic: the
    /// picture being shown is always in the strip.
    #[test]
    fn the_current_picture_is_always_somewhere_in_the_strip() {
        for total in [1usize, 2, 5, 9, 10, 100] {
            for cursor in 0..total {
                for visible in [1usize, 3, 9, 12] {
                    let w = window(total, cursor, visible);
                    assert!(w.contains(cursor), "total={total} cursor={cursor} visible={visible}");
                    assert!(w.end <= total);
                    assert_eq!(w.len(), visible.min(total));
                }
            }
        }
    }

    #[test]
    fn an_empty_folder_shows_an_empty_strip() {
        assert!(window(0, 0, 9).is_empty());
        assert!(window(10, 0, 0).is_empty());
    }

    #[test]
    fn thumbnails_fit_with_their_gaps_counted() {
        // 4 thumbnails of 80 with 3 gaps of 8 = 344, which fits in 400;
        // a fifth would need 432.
        assert_eq!(fits(400.0, 80.0, 8.0), 4);
        assert_eq!(fits(432.0, 80.0, 8.0), 5);
        assert_eq!(fits(80.0, 80.0, 8.0), 1);
    }

    /// A window mid-resize can be zero wide, and must not produce a
    /// nonsense count for the strip to try to draw.
    #[test]
    fn a_window_with_no_width_fits_no_thumbnails() {
        assert_eq!(fits(0.0, 80.0, 8.0), 0);
        assert_eq!(fits(-10.0, 80.0, 8.0), 0);
        assert_eq!(fits(400.0, 0.0, 8.0), 0);
    }
}
