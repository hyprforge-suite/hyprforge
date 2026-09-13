//! Where the popup lands, kept as pure functions over plain numbers.
//!
//! Everything here is deliberately free of `hyprctl`, Wayland or iced —
//! it only knows about points and sizes. That is what makes it testable
//! without a compositor: the piece most likely to be wrong (an
//! off-by-one at an edge, a sign flipped for the bottom-right corner) is
//! exactly the piece a unit test can pin directly, without a monitor to
//! click on.
//!
//! Units are logical pixels throughout — `hyprctl cursorpos` and the
//! `x`/`y` fields of `hyprctl monitors -j` are already logical, and
//! `width`/`height` there are physical and must be divided by `scale`
//! before they reach anything in this module. That division happens in
//! `main.rs`, at the boundary where the JSON is read.

/// A point in logical compositor space (or, once translated, in a
/// monitor's own logical space).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

/// A size in logical pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Size {
    pub width: f64,
    pub height: f64,
}

/// One monitor's logical rectangle, in the same global space `hyprctl
/// cursorpos` reports.
#[derive(Debug, Clone, PartialEq)]
pub struct Monitor {
    pub name: String,
    pub origin: Point,
    pub size: Size,
}

impl Monitor {
    /// Whether `global` falls inside this monitor's rectangle.
    fn contains(&self, global: Point) -> bool {
        global.x >= self.origin.x
            && global.x < self.origin.x + self.size.width
            && global.y >= self.origin.y
            && global.y < self.origin.y + self.size.height
    }
}

/// The monitor the cursor is currently over, if any.
///
/// `None` when the reported position lands outside every monitor
/// `hyprctl` described — a coordinate read at the wrong moment (a
/// monitor unplugged between the two `hyprctl` calls) rather than
/// anything to guess at. The caller falls back to the first monitor
/// rather than refusing to show the popup at all.
pub fn monitor_at(monitors: &[Monitor], global: Point) -> Option<&Monitor> {
    monitors.iter().find(|m| m.contains(global))
}

/// Clamps a popup's top-left corner so the whole popup stays on screen.
///
/// `cursor` and `monitor` are both in the *same* monitor-local logical
/// space (the origin has already been subtracted — see `main.rs`). The
/// popup is anchored top-left in the layer-shell sense, so this returns
/// the margins to hand the compositor directly.
///
/// The rule is: put the top-left corner at the cursor when there is
/// room; otherwise slide the corner back just far enough that the
/// popup's far edge lands on the monitor's edge instead of past it.
/// Never slides past 0 in either axis — a popup taller or wider than the
/// whole monitor is clamped to the top-left corner rather than pushed
/// negative, which would put it off the *opposite* edge instead.
pub fn clamp_popup(cursor: Point, popup: Size, monitor: Size) -> Point {
    let max_x = (monitor.width - popup.width).max(0.0);
    let max_y = (monitor.height - popup.height).max(0.0);
    Point {
        x: cursor.x.clamp(0.0, max_x),
        y: cursor.y.clamp(0.0, max_y),
    }
}

/// Where the popup's rows are, in the same logical-pixel space `view.rs`
/// draws into — so a pointer position can be turned into a row index
/// without an iced runtime to ask, since this popup builds one
/// [`iced_runtime::user_interface::UserInterface`] per frame and throws it
/// away rather than keeping one running that could answer "what's under
/// the cursor" itself (see `surface.rs::draw`).
///
/// Every number here is a value `view.rs` sets explicitly on the widgets
/// it builds (a fixed row height, a fixed header height, a literal
/// spacing) rather than something read back from iced's own text
/// metrics. That is deliberate: a row's fixed height only ever depends on
/// the theme's font size, the one thing both this module and `view.rs`
/// already have, so the two cannot silently drift out of step the way
/// they could if this tried to reverse-engineer cosmic-text's line
/// metrics instead. If a row's drawn geometry ever changes in `view.rs`,
/// change the arithmetic here to match — that is the whole contract.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RowLayout {
    /// Padding around the whole popup's content, all four sides —
    /// `container::padding` in `view::view`.
    pub padding: f64,
    /// Height of the header line ("Type to filter" / the current filter
    /// text) plus the gap under it, before the first row starts.
    pub header_height: f64,
    /// Height of one entry row, thumbnail or text alike — both are drawn
    /// into a container of this same fixed height (see
    /// `view::entry_row`), so an image row hit-tests identically to a
    /// text one.
    pub row_height: f64,
    /// Vertical gap between two rows — `column::spacing` in `view::view`.
    pub row_spacing: f64,
}

impl RowLayout {
    /// Popup padding, in logical pixels — `Padding::from(10)` in
    /// `view::view`.
    pub const PADDING: f64 = 10.0;
    /// Gap between the header and the first row —
    /// `Space::new().height(6)` in `view::view`. `pub` so `view.rs` can
    /// size the header text to exactly `header_height - HEADER_GAP` and
    /// let the literal `Space` widget account for the rest, rather than
    /// this module and `view.rs` each hard-coding `6.0` and hoping they
    /// stay equal.
    pub const HEADER_GAP: f64 = 6.0;
    /// Gap between rows — `column(rows).spacing(2)` in `view::view`.
    pub const ROW_SPACING: f64 = 2.0;
    /// Padding inside each row's own container, top and bottom —
    /// `Padding::from(6)` in `view::entry_row`.
    pub const ROW_PADDING: f64 = 6.0;
    /// Side length of an image row's thumbnail — `THUMBNAIL_SIZE` in
    /// `view::entry_row`. `pub` (rather than a second constant of the
    /// same value living in `view.rs`) because a row's height has to be
    /// tall enough for whichever of a thumbnail or a line of text is
    /// bigger, and this module is the one place that arithmetic happens.
    pub const THUMBNAIL_SIZE: f64 = 32.0;

    /// Derives the layout from the theme's font size, the one variable
    /// both sides of the hit-test already agree on.
    ///
    /// A row holds exactly one line — `view::entry_row`'s label sets
    /// `Wrapping::None` precisely so a row's height cannot depend on how
    /// much text happened to be in it — so a text row's content height is
    /// one line at the theme's font size, `LineHeight`'s default relative
    /// factor of `1.2` applied the same way iced applies it. An image row
    /// draws a fixed-size thumbnail instead of a line of text (see
    /// `view::entry_row`'s `Content::Image` branch), so the row height has
    /// to fit whichever of the two is taller — a small font with a large
    /// thumbnail must not clip the thumbnail, and a large font with the
    /// thumbnail's fixed size must not clip the text.
    pub fn for_font_size(font_size: f32) -> RowLayout {
        let font_size = font_size as f64;
        let line = |size: f64| size * 1.2;
        let content_height = line(font_size).max(Self::THUMBNAIL_SIZE);
        RowLayout {
            padding: Self::PADDING,
            header_height: line(font_size * 0.85) + Self::HEADER_GAP,
            row_height: content_height + Self::ROW_PADDING * 2.0,
            row_spacing: Self::ROW_SPACING,
        }
    }

    /// The row index under `local_y` — measured from the popup surface's
    /// own top-left corner, exactly the coordinate space
    /// `PointerEvent::position` reports — among `visible_count` rows
    /// currently built (see `Model::visible_range`).
    ///
    /// `None` covers every way a position is not over a row: above the
    /// first row (still in the header or its padding), in the gap
    /// between two rows, or past the last row that is actually on
    /// screen. All three are "do nothing", the same as a keyboard press
    /// this popup does not recognise.
    pub fn row_at(&self, local_y: f64, visible_count: usize) -> Option<usize> {
        if visible_count == 0 {
            return None;
        }
        let y = local_y - self.padding - self.header_height;
        if y < 0.0 {
            return None;
        }
        let stride = self.row_height + self.row_spacing;
        let index = (y / stride) as usize;
        if index >= visible_count {
            return None;
        }
        // Reject a hit that landed in the gap *after* this row's own
        // height, rather than clamping it to the row anyway — a gap that
        // silently selected whichever row was nearest would make the
        // dead zone between two rows pick one of them at random as far
        // as the user could tell.
        let within_row = y - (index as f64 * stride);
        if within_row > self.row_height {
            return None;
        }
        Some(index)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const POPUP: Size = Size { width: 320.0, height: 400.0 };
    const MONITOR: Size = Size { width: 1600.0, height: 1000.0 };

    #[test]
    fn the_popup_lands_exactly_at_the_cursor_when_there_is_room() {
        let cursor = Point { x: 753.0, y: 312.0 };
        assert_eq!(clamp_popup(cursor, POPUP, MONITOR), cursor);
    }

    #[test]
    fn the_popup_flips_left_off_the_right_edge() {
        // Cursor near the right edge: the popup's right edge must not
        // pass the monitor's right edge.
        let cursor = Point { x: 1550.0, y: 312.0 };
        let placed = clamp_popup(cursor, POPUP, MONITOR);
        assert_eq!(placed.x, MONITOR.width - POPUP.width);
        assert!(placed.x + POPUP.width <= MONITOR.width);
    }

    #[test]
    fn the_popup_flips_up_off_the_bottom_edge() {
        let cursor = Point { x: 753.0, y: 980.0 };
        let placed = clamp_popup(cursor, POPUP, MONITOR);
        assert_eq!(placed.y, MONITOR.height - POPUP.height);
        assert!(placed.y + POPUP.height <= MONITOR.height);
    }

    #[test]
    fn the_popup_stays_fully_on_screen_in_every_corner() {
        for cursor in [
            Point { x: 0.0, y: 0.0 },
            Point { x: MONITOR.width, y: 0.0 },
            Point { x: 0.0, y: MONITOR.height },
            Point { x: MONITOR.width, y: MONITOR.height },
        ] {
            let placed = clamp_popup(cursor, POPUP, MONITOR);
            assert!(placed.x >= 0.0, "{placed:?} left the left edge");
            assert!(placed.y >= 0.0, "{placed:?} left the top edge");
            assert!(placed.x + POPUP.width <= MONITOR.width, "{placed:?} ran off the right");
            assert!(placed.y + POPUP.height <= MONITOR.height, "{placed:?} ran off the bottom");
        }
    }

    /// A popup bigger than the monitor cannot be fully on screen either
    /// way; it must clamp to the top-left corner rather than go negative
    /// and run off the *other* edge instead.
    #[test]
    fn a_popup_larger_than_the_monitor_clamps_to_the_top_left_rather_than_going_negative() {
        let huge = Size { width: MONITOR.width * 2.0, height: MONITOR.height * 2.0 };
        let placed = clamp_popup(Point { x: 500.0, y: 500.0 }, huge, MONITOR);
        assert_eq!(placed, Point { x: 0.0, y: 0.0 });
    }

    #[test]
    fn the_monitor_under_the_cursor_is_found_among_several() {
        let left = Monitor {
            name: "eDP-2".into(),
            origin: Point { x: 0.0, y: 0.0 },
            size: Size { width: 1600.0, height: 1000.0 },
        };
        let right = Monitor {
            name: "DP-3".into(),
            origin: Point { x: 1600.0, y: 0.0 },
            size: Size { width: 1920.0, height: 1080.0 },
        };
        let monitors = vec![left.clone(), right.clone()];

        assert_eq!(monitor_at(&monitors, Point { x: 753.0, y: 312.0 }), Some(&left));
        assert_eq!(monitor_at(&monitors, Point { x: 1700.0, y: 50.0 }), Some(&right));
    }

    /// A coordinate that lands outside every monitor is reported as
    /// such, not guessed at — the caller decides the fallback.
    #[test]
    fn a_cursor_outside_every_monitor_finds_none() {
        let monitors = vec![Monitor {
            name: "eDP-2".into(),
            origin: Point { x: 0.0, y: 0.0 },
            size: Size { width: 1600.0, height: 1000.0 },
        }];
        assert_eq!(monitor_at(&monitors, Point { x: 5000.0, y: 5000.0 }), None);
    }

    #[test]
    fn a_pointer_over_the_header_hits_no_row() {
        let layout = RowLayout::for_font_size(15.0);
        assert_eq!(layout.row_at(0.0, 5), None);
        assert_eq!(layout.row_at(layout.padding, 5), None);
    }

    #[test]
    fn the_first_row_starts_right_after_the_header() {
        let layout = RowLayout::for_font_size(15.0);
        let first_row_top = layout.padding + layout.header_height;
        assert_eq!(layout.row_at(first_row_top, 5), Some(0));
        assert_eq!(
            layout.row_at(first_row_top + layout.row_height - 0.01, 5),
            Some(0),
            "must still be row 0 right up to its own bottom edge"
        );
    }

    #[test]
    fn a_pointer_in_the_gap_between_two_rows_hits_neither() {
        let layout = RowLayout::for_font_size(15.0);
        let first_row_top = layout.padding + layout.header_height;
        let gap_middle = first_row_top + layout.row_height + layout.row_spacing / 2.0;
        assert_eq!(layout.row_at(gap_middle, 5), None);
    }

    #[test]
    fn successive_rows_are_found_in_order() {
        let layout = RowLayout::for_font_size(15.0);
        let stride = layout.row_height + layout.row_spacing;
        let first_row_top = layout.padding + layout.header_height;
        for index in 0..5 {
            let middle = first_row_top + index as f64 * stride + layout.row_height / 2.0;
            assert_eq!(layout.row_at(middle, 5), Some(index));
        }
    }

    #[test]
    fn a_pointer_past_the_last_visible_row_hits_nothing() {
        let layout = RowLayout::for_font_size(15.0);
        let stride = layout.row_height + layout.row_spacing;
        let first_row_top = layout.padding + layout.header_height;
        let past_the_end = first_row_top + 5.0 * stride;
        assert_eq!(layout.row_at(past_the_end, 5), None);
    }

    #[test]
    fn an_empty_visible_window_hits_nothing_no_matter_where_the_pointer_is() {
        let layout = RowLayout::for_font_size(15.0);
        assert_eq!(layout.row_at(layout.padding + layout.header_height, 0), None);
    }

    #[test]
    fn a_bigger_font_makes_taller_rows() {
        // Big enough that the *text* line, not the fixed-size thumbnail,
        // is what is driving `row_height` — otherwise two font sizes
        // that both fit under `THUMBNAIL_SIZE` would produce the same
        // row height and this test would prove nothing.
        let small = RowLayout::for_font_size(12.0);
        let large = RowLayout::for_font_size(40.0);
        assert!(large.row_height > small.row_height);
        assert!(large.header_height > small.header_height);
    }

    /// A row has to fit a thumbnail even when the font is small enough
    /// that a line of text alone would make a shorter row — otherwise an
    /// image row's fixed-size thumbnail gets clipped by `entry_row`'s
    /// `.clip(true)`, the exact regression fixing the text-overflow
    /// problem could have introduced for image rows if the row height
    /// were derived from the font alone.
    #[test]
    fn a_small_font_still_leaves_room_for_the_thumbnail() {
        let layout = RowLayout::for_font_size(10.0);
        assert!(
            layout.row_height >= RowLayout::THUMBNAIL_SIZE + RowLayout::ROW_PADDING * 2.0,
            "row_height {} must fit a {}-pixel thumbnail plus its padding",
            layout.row_height,
            RowLayout::THUMBNAIL_SIZE,
        );
    }
}
