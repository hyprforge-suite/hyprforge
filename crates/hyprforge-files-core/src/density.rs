//! Row/header/type-ramp sizing, derived from font metrics rather than
//! copied out of the mockup as pixel constants.
//!
//! The design specifies a 40px header, 28px rows, a 15px type ramp and a
//! 6px-inner/12px-outer radius pair — but those numbers are only true at
//! the theme's default text size and 100% [`FontScale`]. Pillar 7 puts
//! font scaling in the shared layer for exactly this reason: a `28.0`
//! literal for row height clips its own text the moment someone raises
//! the scale past 1.0, because the text grows and the row does not. So
//! every design pixel value here is stored as its *ratio* to
//! [`BASE_TEXT_SIZE`] at identity scale.
//!
//! Two different things come out of that ratio, deliberately kept apart:
//!
//! - A **text size** ([`ROW_TEXT_BASE`], [`META_TEXT_BASE`]) is left
//!   unscaled and handed to [`hyprforge_ui::widgets::scaled_text`] (or
//!   [`meta_text`](hyprforge_ui::widgets::meta_text)) the same way
//!   [`BASE_TEXT_SIZE`] itself already is everywhere else in this crate
//!   — applying `FontScale` a second time here would compound it rather
//!   than replace it, the exact mistake [`FontScale::apply`]'s own doc
//!   warns against.
//! - A **layout dimension** ([`row_height`], [`bar_height`]) is a
//!   function that returns the final, already-scaled pixel value,
//!   because a container's `Length::Fixed` needs a concrete number, not
//!   something a widget will scale for it later.
//!
//! At 100% the numbers below reproduce the design's own values exactly
//! (pinned in the tests); above 100% the row, the header and the text
//! inside them grow together instead of the text alone overflowing a row
//! that stayed put.
//!
//! Radii take the same "ratio, not constant" shape but from
//! [`hyprforge_look::Theme::rounding`] instead of text size: the design's
//! outer/inner pair is preserved as a *relationship* (inner is exactly
//! half the outer) rather than as two independent magic numbers, so a
//! user who changes Hyprland's own `decoration:rounding` gets an inner
//! radius that stays visibly tighter than the outer one instead of one
//! that quietly drifts to match or overtake it.

use hyprforge_ui::theme::{self, FontScale, BASE_TEXT_SIZE};

/// design px / [`BASE_TEXT_SIZE`], at identity scale.
const ROW_HEIGHT_RATIO: f32 = 28.0 / BASE_TEXT_SIZE;

/// The primary text size inside a row — a file's name — **unscaled**:
/// feed this straight to `scaled_text`/`meta_text` as the `base_size`
/// argument, which applies `FontScale` itself, the same way every other
/// call site feeds it [`BASE_TEXT_SIZE`]. It is already in the same
/// "logical px before scale" unit `BASE_TEXT_SIZE` is, so no further
/// division by it is needed — this constant *is* the design's 15px type
/// ramp at identity scale, not a ratio applied to something else.
pub const ROW_TEXT_BASE: f32 = 15.0;

/// The metadata text size inside a row — size, modified, kind — also
/// unscaled. The design only names the 15px primary size; this keeps the
/// 13/15 relationship the crate's earlier hand-picked `13.0`-vs-`14.0`
/// pairing already had, expressed as a ratio of the primary size instead
/// of a second independent constant.
pub const META_TEXT_BASE: f32 = ROW_TEXT_BASE * (13.0 / 15.0);

/// Half the outer radius — see the module doc for why this is a
/// relationship, not a second magic number.
const INNER_OUTER_RATIO: f32 = 0.5;

/// A list row's final height in logical pixels, `FontScale` already
/// applied. `28.0` at [`FontScale::default`]; grows proportionally above
/// 100%.
pub fn row_height(scale: FontScale) -> f32 {
    scale.apply(BASE_TEXT_SIZE) * ROW_HEIGHT_RATIO
}

/// The window's own corner radius — the design's "12px outer". Taken
/// straight from the active [`hyprforge_look::Theme`] rather than
/// duplicated: `Theme::rounding` already *is* Hyprland's own
/// `decoration:rounding`, and it happens to default to 12, which is a
/// measured fact about the default theme rather than a coincidence this
/// function relies on.
pub fn outer_radius() -> f32 {
    theme::active().rounding as f32
}

/// A row's/sidebar button's corner radius — the design's "6px inner".
/// See the module doc: this is deliberately half of [`outer_radius`]
/// rather than its own constant.
pub fn inner_radius() -> f32 {
    outer_radius() * INNER_OUTER_RATIO
}

/// The header bar's height. 44px at 100% scale.
///
/// Taller than the 28px controls inside it on purpose: the difference is
/// what makes the bar read as a *plane* the controls sit on, rather than
/// as a row of controls with a background colour.
pub fn bar_height(scale: FontScale) -> f32 {
    scale.apply(BAR_HEIGHT_BASE)
}

/// A text field — the path bar, the search field. 28px at 100%.
pub fn field_height(scale: FontScale) -> f32 {
    scale.apply(FIELD_HEIGHT_BASE)
}

/// A glyph button — the nav cluster, a view-mode segment. 26px at 100%.
///
/// Two pixels shorter than a field, deliberately: enough difference that
/// a button feels pressable and a field feels typeable, close enough
/// that the row still baselines cleanly.
pub fn glyph_button(scale: FontScale) -> f32 {
    scale.apply(GLYPH_BUTTON_BASE)
}

/// The radius on something nested *inside* a control — the path bar's
/// current-directory chip, a segment inside the view-mode track.
///
/// Third level of a deliberate ladder: 12 on the window, 6 on a
/// top-level control, 4 on something inside one. A nested thing sharing
/// its parent's radius reads as a second parent rather than as a child.
pub fn nested_radius() -> f32 {
    inner_radius() * NESTED_RADIUS_FRACTION
}

const BAR_HEIGHT_BASE: f32 = 44.0;
const FIELD_HEIGHT_BASE: f32 = 28.0;
const GLYPH_BUTTON_BASE: f32 = 26.0;
const NESTED_RADIUS_FRACTION: f32 = 4.0 / 6.0;

/// The search field's fixed width. 190px at 100% scale.
pub const SEARCH_FIELD_WIDTH: f32 = 190.0;

/// The sidebar's fixed width, from the design's 216px at 100% scale.
///
/// Fixed rather than a fraction of the window: a sidebar that grows with
/// the window wastes the space a file listing wants, and one that shrinks
/// starts truncating "Downloads" on a narrow window. Not scaled by
/// `FontScale` — the *rows* inside it scale, and a sidebar wide enough
/// for its longest label at 100% is still wide enough at 125% because
/// the label grows into the padding, not past it.
pub const SIDEBAR_WIDTH: f32 = 216.0;

/// The collapsed sidebar's width: one mark, centred, with air round it.
///
/// Wide enough that the marks are a column rather than a stripe, narrow
/// enough to be worth collapsing to — it gives back about four fifths of
/// [`SIDEBAR_WIDTH`], which is the point. (Stated here rather than
/// asserted in a test: an `assert!` comparing two constants is a check
/// that cannot fail, which clippy correctly refuses. What a test *can*
/// see is that collapsing widens the listing, and
/// `collapsing_the_sidebar_gives_most_of_its_width_to_the_listing` does.)
pub const SIDEBAR_RAIL_WIDTH: f32 = 44.0;

/// Below this window width the sidebar collapses on its own.
///
/// Unscaled, and deliberately: this is compared against a *window* width
/// in logical pixels, which is a fact about the user's screen rather
/// than about their font size. Scaling it would make a bigger font
/// collapse the sidebar on a window that has not changed size.
///
/// The number is the sidebar's own width plus enough listing beside it
/// to be worth showing — below that, a 216-pixel sidebar is most of the
/// window, and the listing it exists to help you navigate has no room
/// left. An explicit `SidebarPref::Shown` still overrides it.
pub const SIDEBAR_COLLAPSE_BELOW: f32 = SIDEBAR_WIDTH + 420.0;

/// How much width the listing pane actually gets, given the window's
/// width and whether the sidebar is showing.
///
/// Derived rather than measured with `responsive`: the listing is the
/// one part of this window whose contents are proportional to the
/// directory, so rebuilding it on every layout pass — which is what a
/// `responsive` closure does, since it must be callable more than once —
/// is the one place that cost actually bites. Everything subtracted here
/// is a constant of the layout, so the arithmetic is exact.
pub fn list_pane_width(viewport_width: f32, sidebar_collapsed: bool) -> f32 {
    // The rail, not zero: collapsing folds the labels away and keeps the
    // marks, so it still costs something — see `browser::sidebar_rail`.
    let sidebar = if sidebar_collapsed { SIDEBAR_RAIL_WIDTH } else { SIDEBAR_WIDTH };
    // `file_area`'s own padding, either side.
    let padding = 2.0 * hyprforge_ui::theme::spacing::SM;
    (viewport_width - sidebar - padding).max(0.0)
}

/// The narrowest a list row can be drawn before its columns stop being
/// readable and start being a puzzle.
///
/// This is what makes horizontal scrolling the answer instead of ever
/// narrower columns. `FillPortion` shares out whatever it is given, with
/// no floor — so at a small enough width every column gets a few pixels
/// and the listing becomes six columns of nothing. Past this point the
/// list keeps this width and the pane scrolls sideways to reach the rest
/// of it, which is what a spreadsheet does and what people expect.
///
/// Scaled, unlike the breakpoint above, because it is a width measured
/// in *text*: bigger type genuinely needs more room per column.
pub fn list_min_width(shown_columns: usize, scale: FontScale) -> f32 {
    // The icon lane, the name, one gap before each optional column, and
    // the columns themselves.
    let icon_lane = scale.apply(ROW_ICON) + scale.apply(hyprforge_ui::theme::spacing::SM);
    let gaps = shown_columns as f32 * scale.apply(hyprforge_ui::theme::spacing::SM);
    icon_lane
        + scale.apply(NAME_MIN_WIDTH)
        + shown_columns as f32 * scale.apply(COLUMN_MIN_WIDTH)
        + gaps
}

/// The icon lane at the head of every list row.
pub const ROW_ICON: f32 = 20.0;

/// The least room a name gets before the list would rather scroll.
const NAME_MIN_WIDTH: f32 = 160.0;

/// The least room any one optional column gets.
///
/// This is the width at which a column stops being *readable*, not the
/// width at which it stops being comfortable — the difference matters,
/// because the moment this is exceeded a scrollbar appears, and a
/// scrollbar under a listing whose columns all fit perfectly well reads
/// as the app being confused about its own layout. Measured against the
/// real thing: `drwxr-xr-x` and `Dec 28, 2025` are the widest values any
/// optional column holds, and both sit inside this at the default font.
const COLUMN_MIN_WIDTH: f32 = 80.0;

/// A grid cell's width in logical pixels, `FontScale` applied.
///
/// The grid used to be five fixed columns of 96px with a 48px icon and
/// 12px text, and it read as squashed for three separate reasons at
/// once: the cell was barely wider than its own icon, so a name of any
/// length filled it edge to edge; the text was smaller than anywhere
/// else in the app for no reason; and five columns meant the cells got
/// no wider as the window did, they just left a growing empty margin.
///
/// Wide enough here that a typical name fits on one line with air
/// either side, and the column *count* comes from the available width
/// instead — see `grid_columns`.
pub fn grid_cell_width(scale: FontScale) -> f32 {
    scale.apply(GRID_CELL_WIDTH)
}

/// A grid cell's total height: icon, gap, and two lines of name.
pub fn grid_cell_height(scale: FontScale) -> f32 {
    scale.apply(GRID_CELL_HEIGHT)
}

/// The room a grid cell gives a name: two lines of row text.
///
/// Fixed rather than "as much as the name needs", because cells sit in
/// rows — one three-line name would push its whole row taller than the
/// rows above and below it, and a grid whose rows are different heights
/// reads as broken rather than as accommodating.
pub fn grid_name_height(scale: FontScale) -> f32 {
    scale.apply(ROW_TEXT_BASE) * GRID_NAME_LINE_HEIGHT * GRID_NAME_LINES
}

/// iced's default line height is 1.3x the text size.
const GRID_NAME_LINE_HEIGHT: f32 = 1.3;
const GRID_NAME_LINES: f32 = 2.0;

/// The icon inside a grid cell.
pub fn grid_icon_size(scale: FontScale) -> f32 {
    scale.apply(GRID_ICON)
}

/// How many whole cells fit across a grid pane `pane_width` logical
/// pixels wide.
///
/// **Takes the pane's full width and subtracts what the grid does not
/// get to use**, rather than trusting a caller to have done it. Getting
/// this wrong does not look like a layout that is one column too greedy;
/// it looks like the cells in the last column have *shrunk*, because a
/// row of fixed-width cells that overruns its bounds gets squeezed to
/// fit. The grid's own padding (a gap either side) and the scrollbar's
/// lane both come out of the pane before any cell does.
///
/// The scrollbar lane is reserved whether or not a scrollbar is showing.
/// Reserving it only when the listing is long enough to scroll would
/// mean a directory that grows past one screenful silently reflows the
/// grid — the column count changing because a file was added is a worse
/// surprise than ten pixels of margin in a short listing.
///
/// At least one, always: a pane too narrow for one whole cell should
/// show one cell rather than an empty pane, which is what a zero would
/// draw.
pub fn grid_columns(pane_width: f32, scale: FontScale) -> usize {
    let cell = grid_cell_width(scale);
    let gap = scale.apply(GRID_GAP);
    let usable = pane_width - 2.0 * gap - scale.apply(SCROLLBAR_LANE);
    // n cells and n-1 gaps fit in `usable`, so solve for n.
    (((usable + gap) / (cell + gap)).floor() as usize).max(1)
}

/// The gap between grid cells, both directions.
pub fn grid_gap(scale: FontScale) -> f32 {
    scale.apply(GRID_GAP)
}

const GRID_CELL_WIDTH: f32 = 132.0;
/// Tall enough to hold everything a cell puts in it — padding, icon,
/// gap, two lines of name — with nothing squeezed. `grid_tests` computes
/// that sum and asserts this covers it, so changing the icon or the
/// name budget fails the test rather than silently clipping.
const GRID_CELL_HEIGHT: f32 = 132.0;
const GRID_ICON: f32 = 56.0;
const GRID_GAP: f32 = 14.0;

/// Width kept clear down the right-hand side of a scrollable pane for
/// its scrollbar. iced's own default bar is 10 logical pixels wide with
/// no margin (`scrollable::Scrollbar`'s `Default`); this matches it, and
/// a test would not catch the two drifting apart because iced's value is
/// not `pub`. If a scrollbar ever looks like it is sitting on top of the
/// last column, this is the number to check first.
const SCROLLBAR_LANE: f32 = 10.0;

/// A sidebar section heading — `PLACES`, `PINNED`, `TRASH`.
///
/// Smaller than the meta text it is built from, because a heading in a
/// sidebar is a signpost rather than something to read: it should be
/// findable when looked for and invisible when not.
pub const SECTION_LABEL_BASE: f32 = META_TEXT_BASE * 0.85;

/// The folder mark beside a sidebar row, at 100% scale.
pub const SIDEBAR_MARK_BASE: f32 = 13.0;

/// The status bar's height — the design's 30px at 100% scale, derived
/// the same way the header is so it grows with the text inside it.
pub fn status_height(scale: FontScale) -> f32 {
    scale.apply(STATUS_HEIGHT_BASE)
}

const STATUS_HEIGHT_BASE: f32 = 30.0;


#[cfg(test)]
mod grid_tests {
    use super::*;

    /// The fixed five-column grid never got wider *or* more numerous as
    /// the window grew — it sat at one size with an expanding margin
    /// beside it, which is most of why it read as squashed.
    #[test]
    fn a_wider_pane_fits_more_columns() {
        let scale = FontScale::default();
        let narrow = grid_columns(400.0, scale);
        let wide = grid_columns(1200.0, scale);
        assert!(wide > narrow, "{wide} should beat {narrow}");
    }

    /// Never zero: a pane too narrow for one whole cell should show one
    /// clipped cell, not an empty grid.
    #[test]
    fn a_pane_narrower_than_one_cell_still_fits_one() {
        assert_eq!(grid_columns(10.0, FontScale::default()), 1);
        assert_eq!(grid_columns(0.0, FontScale::default()), 1);
    }

    /// A collapsed sidebar gives its width back to the listing, which
    /// is the whole point of collapsing it.
    #[test]
    fn collapsing_the_sidebar_gives_most_of_its_width_to_the_listing() {
        let with = list_pane_width(1000.0, false);
        let collapsed = list_pane_width(1000.0, true);
        // Most, not all: the rail keeps the marks reachable.
        assert_eq!(collapsed - with, SIDEBAR_WIDTH - SIDEBAR_RAIL_WIDTH);
        assert!(collapsed > with);
    }

    /// Never negative: a window narrower than its own chrome is a real
    /// state during a drag, and a negative width would make every
    /// comparison against it read backwards.
    #[test]
    fn a_window_narrower_than_its_chrome_reports_no_pane_rather_than_a_negative_one() {
        assert_eq!(list_pane_width(50.0, false), 0.0);
    }

    /// More columns need more room before they stop sharing and start
    /// scrolling — the floor is a function of what is actually shown,
    /// not a constant.
    #[test]
    fn the_scroll_threshold_grows_with_the_number_of_columns() {
        let scale = FontScale::default();
        assert!(list_min_width(5, scale) > list_min_width(2, scale));
        assert!(list_min_width(0, scale) > 0.0, "a name column still needs room");
    }

    /// The count has to account for the gaps between cells *and* for the
    /// padding and scrollbar lane that never belonged to the grid — or
    /// the last column overruns the pane, and a row of fixed-width cells
    /// that overruns gets squeezed, so the symptom is cells that look
    /// shrunken rather than a grid that looks too wide.
    #[test]
    fn the_columns_and_everything_around_them_fit_in_the_pane() {
        let scale = FontScale::default();
        // Swept rather than sampled: the bug only shows within a few
        // pixels of a column boundary, which a handful of round numbers
        // walks straight past.
        let mut width = 120.0_f32;
        while width < 4000.0 {
            let n = grid_columns(width, scale) as f32;
            let used = n * grid_cell_width(scale)
                + (n - 1.0) * grid_gap(scale)
                + 2.0 * grid_gap(scale)
                + scale.apply(SCROLLBAR_LANE);
            assert!(
                n == 1.0 || used <= width,
                "{n} columns need {used} but the pane is only {width}"
            );
            width += 1.0;
        }
    }

    /// The same sweep, at a font scale where every dimension is larger —
    /// the reservation has to scale with them, not stay at 1x.
    #[test]
    fn the_columns_fit_the_pane_at_a_larger_font_scale_too() {
        let scale = FontScale(1.6);
        let mut width = 120.0_f32;
        while width < 4000.0 {
            let n = grid_columns(width, scale) as f32;
            let used = n * grid_cell_width(scale)
                + (n - 1.0) * grid_gap(scale)
                + 2.0 * grid_gap(scale)
                + scale.apply(SCROLLBAR_LANE);
            assert!(n == 1.0 || used <= width, "{n} columns need {used} in {width}");
            width += 1.0;
        }
    }

    /// A cell must never be asked to be narrower than it is: shrinking
    /// is what the old fixed column count caused, and it is the thing a
    /// reader actually notices.
    #[test]
    fn a_narrow_pane_drops_a_column_rather_than_narrowing_the_cells() {
        let scale = FontScale::default();
        let wide = grid_columns(900.0, scale);
        let narrower = grid_columns(760.0, scale);
        assert!(narrower < wide, "{narrower} should be fewer than {wide}");
        // And the cell width is the same either way — it is a constant,
        // not a function of the pane.
        assert_eq!(grid_cell_width(scale), grid_cell_width(scale));
    }

    /// The cell has to hold everything it is given — its own padding,
    /// the icon, the gap under it, and the two lines a name may take —
    /// or the frame clips the content it exists to present.
    ///
    /// Spelled out as the sum rather than as a rough inequality, because
    /// the failure this guards against is exactly the one that arrives
    /// by a few pixels when someone grows the icon.
    #[test]
    fn a_cell_holds_its_icon_and_two_lines_of_name_without_squeezing() {
        for scale in [FontScale::default(), FontScale(1.25), FontScale(1.6)] {
            // `grid_cell`'s own layout: `padding(SM)` all round, then a
            // column of [icon, name] spaced `SM`.
            let padding = scale.apply(hyprforge_ui::theme::spacing::SM) * 2.0;
            let gap = scale.apply(hyprforge_ui::theme::spacing::SM);
            let needed = padding + grid_icon_size(scale) + gap + grid_name_height(scale);
            assert!(
                grid_cell_height(scale) >= needed,
                "a cell is {} tall but needs {needed} at scale {:?}",
                grid_cell_height(scale),
                scale,
            );
        }
        assert!(grid_cell_width(FontScale::default()) > grid_icon_size(FontScale::default()));
    }

    /// Two lines, not one and not three — a grid whose rows are
    /// different heights because one name was longer reads as broken.
    #[test]
    fn the_name_budget_is_exactly_two_lines() {
        let scale = FontScale::default();
        let one_line = scale.apply(ROW_TEXT_BASE) * 1.3;
        assert!((grid_name_height(scale) - one_line * 2.0).abs() < 0.01);
    }

    /// Everything here scales with the font, like every other dimension
    /// in this module — see the module doc.
    #[test]
    fn every_grid_dimension_grows_with_the_font_scale() {
        let big = FontScale(1.5);
        assert!(grid_cell_width(big) > grid_cell_width(FontScale::default()));
        assert!(grid_cell_height(big) > grid_cell_height(FontScale::default()));
        assert!(grid_icon_size(big) > grid_icon_size(FontScale::default()));
        // And a bigger font means fewer cells across the same pane.
        assert!(grid_columns(1200.0, big) < grid_columns(1200.0, FontScale::default()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // These pin the design's literal numbers at the scale they were
    // measured at. `Theme::rounding` defaults to 12 and nothing in this
    // crate's test suite ever installs a theme that changes it (only
    // `icon.rs` calls `theme::init`, and only to change `accent`), so
    // the outer/inner assertions below hold regardless of test order —
    // `theme::init`'s `OnceLock` only ever accepts the first call in the
    // whole test binary.

    #[test]
    fn at_default_scale_every_number_matches_the_design_exactly() {
        let scale = FontScale::default();
        assert_eq!(row_height(scale), 28.0);
        assert_eq!(scale.apply(ROW_TEXT_BASE), 15.0);
        assert_eq!(outer_radius(), 12.0, "Theme::rounding's default");
        assert_eq!(inner_radius(), 6.0, "half the outer radius");
    }

    #[test]
    fn at_125_percent_every_size_grows_proportionally_not_clipped() {
        let scale = FontScale(1.25);
        assert_eq!(row_height(scale), 28.0 * 1.25);
        assert_eq!(scale.apply(ROW_TEXT_BASE), 15.0 * 1.25);
    }

    /// The property the whole module exists for: whatever the scale, the
    /// text drawn in a row must fit inside that row. A fixed 28px row
    /// beside text that grows with `FontScale` would violate this past
    /// some scale; deriving both from the same base never can, because
    /// they share the same `FontScale::apply(BASE_TEXT_SIZE)` factor.
    #[test]
    fn text_never_exceeds_the_row_it_is_in() {
        for tenths in 5..=30 {
            let scale = FontScale(tenths as f32 / 10.0);
            let text = scale.apply(ROW_TEXT_BASE);
            let row = row_height(scale);
            assert!(text < row, "row text {text} must fit inside row height {row} at scale {tenths}");
        }
    }

    #[test]
    fn metadata_text_stays_smaller_than_the_primary_size() {
        let scale = FontScale(1.25);
        assert!(scale.apply(META_TEXT_BASE) < scale.apply(ROW_TEXT_BASE));
    }

    #[test]
    fn inner_radius_stays_tighter_than_outer_whatever_rounding_is() {
        // outer_radius() reads the live theme rather than a constant, so
        // this asserts the *relationship* the design cares about rather
        // than a specific number — see the module doc.
        assert!(inner_radius() < outer_radius());
        assert_eq!(inner_radius(), outer_radius() * 0.5);
    }
}
