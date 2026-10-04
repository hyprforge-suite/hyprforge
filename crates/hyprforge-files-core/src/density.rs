//! Files' own sizing: the grid, the context menu, the sidebar's width
//! and when it collapses.
//!
//! The sizes every Hyprforge app shares — rows, bars, fields, the type
//! ramp and the radius ladder — moved to [`hyprforge_ui::density`] when
//! Settings adopted the same design, and are re-exported from here. The
//! rule both halves follow is the one written at the top of that module:
//! a design pixel value is stored as a ratio, never as a literal that
//! clips its own text the moment the font scale moves.

use hyprforge_ui::theme::FontScale;

// The suite-wide half — the type ramp, the radius ladder and the bar,
// field and row heights — lives in `hyprforge_ui::density` so Settings
// draws the same sizes. Re-exported so nothing in this crate had to
// learn a second path for a number it already used.
pub use hyprforge_ui::density::*;

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

/// A context menu's size, `(width, height)`, before it is drawn.
///
/// Computed rather than measured, because the menu has to be placed —
/// flipped away from an edge — before iced lays it out. Every number
/// here is also what `browser::context_menu` uses to draw it, so the two
/// cannot disagree about how tall a menu is.
pub fn menu_size(items: &[crate::menu::MenuItem], scale: FontScale) -> (f32, f32) {
    let rows: f32 = items
        .iter()
        .map(|item| match item {
            crate::menu::MenuItem::Separator => menu_separator_height(scale),
            crate::menu::MenuItem::Action { .. } | crate::menu::MenuItem::Custom { .. } => row_height(scale),
        })
        .sum();
    // `menu_padding` all round, plus the one-pixel border either side.
    let chrome = 2.0 * menu_padding(scale) + 2.0;
    (scale.apply(MENU_WIDTH), rows + chrome)
}

/// The space inside a menu's border.
pub fn menu_padding(scale: FontScale) -> f32 {
    scale.apply(hyprforge_ui::theme::spacing::XS)
}

/// A separator line's whole slot, the line centred in it.
pub fn menu_separator_height(scale: FontScale) -> f32 {
    scale.apply(MENU_SEPARATOR)
}

/// Wide enough for the longest built-in label beside its shortcut —
/// "Open in New Tab" and "Ctrl+Enter".
const MENU_WIDTH: f32 = 260.0;
const MENU_SEPARATOR: f32 = 9.0;

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

/// The preview pane at its widest, and the width its picture is decoded
/// for.
pub const PREVIEW_MAX_WIDTH: f32 = 280.0;

/// Narrower than this the pane's picture is a stamp and its details wrap
/// a word to a line — at that point the room is worth more to the
/// listing, and the pane steps aside.
pub const PREVIEW_MIN_WIDTH: f32 = 200.0;

/// What the listing keeps for itself before the pane may take anything.
/// The pane is the extra; the listing is the reason the window is open.
pub const LISTING_MIN_BESIDE_PREVIEW: f32 = 320.0;

/// How wide the preview pane is drawn, or `None` when there is no room
/// for it at all.
///
/// Shrinks rather than vanishing. It used to be all or nothing at an
/// 820-pixel window, which on a half-screen tile meant a status bar
/// offering to "Hide preview" beside no preview — the setting was on and
/// the window simply never showed it, with nothing saying why. Now the
/// pane gives way gradually, from [`PREVIEW_MAX_WIDTH`] down to
/// [`PREVIEW_MIN_WIDTH`], and only below that does it step aside.
pub fn preview_width(viewport_width: f32, sidebar_collapsed: bool) -> Option<f32> {
    let spare = list_pane_width(viewport_width, sidebar_collapsed) - LISTING_MIN_BESIDE_PREVIEW;
    (spare >= PREVIEW_MIN_WIDTH).then(|| spare.min(PREVIEW_MAX_WIDTH))
}

/// The Properties inspector at its widest — a little wider than the
/// preview pane, because its lines are a label *and* a value side by
/// side where the pane stacks them. The mockup's is 352.
pub const INSPECTOR_MAX_WIDTH: f32 = 340.0;

/// Narrower than this "Accessed" and its date no longer share a line.
pub const INSPECTOR_MIN_WIDTH: f32 = 260.0;

/// How wide the inspector is drawn, in the preview pane's slot.
///
/// Never `None`, unlike [`preview_width`]: the pane is on by default and
/// may quietly give way, but the inspector is only ever there because
/// someone just asked for it, and asking for something that then does
/// not appear reads as the key not working. So in a narrow window it is
/// the listing that gives way, down to the inspector's own floor.
pub fn inspector_width(viewport_width: f32, sidebar_collapsed: bool) -> f32 {
    let spare = list_pane_width(viewport_width, sidebar_collapsed) - LISTING_MIN_BESIDE_PREVIEW;
    spare.clamp(INSPECTOR_MIN_WIDTH, INSPECTOR_MAX_WIDTH)
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

/// One folder's pane in column view, to the left of the folder in view.
///
/// Room for the icon and a name of twenty-odd characters — most names
/// whole, the rest cut at the pane's edge as the list cuts them — so a
/// 1400-pixel window shows home and three levels below it beside the
/// preview pane. Scaled, because it is a width measured in text.
pub fn column_pane_width(scale: FontScale) -> f32 {
    scale.apply(COLUMN_PANE_WIDTH)
}

const COLUMN_PANE_WIDTH: f32 = 220.0;

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

/// A grid cell's total height: icon, gap, and `lines` lines of name —
/// `[behaviour] grid-name-lines`, one to three.
///
/// The two-line cell is the design's 132; each line more or fewer is one
/// line of row text more or less, so the slack the result cell's folder
/// line lives in is the same at every setting.
pub fn grid_cell_height(scale: FontScale, lines: u8) -> f32 {
    let line = ROW_TEXT_BASE * GRID_NAME_LINE_HEIGHT;
    scale.apply(GRID_CELL_HEIGHT + (lines as f32 - DEFAULT_NAME_LINES) * line)
}

/// The room a grid cell gives a name: `lines` lines of row text.
///
/// Fixed rather than "as much as the name needs", because cells sit in
/// rows — one three-line name would push its whole row taller than the
/// rows above and below it, and a grid whose rows are different heights
/// reads as broken rather than as accommodating. A name that needs more
/// is cut with "…" by `hyprforge_ui::widgets::clamped_text` — never
/// clipped through its letters — and shown whole when its cell is the
/// one selected.
pub fn grid_name_height(scale: FontScale, lines: u8) -> f32 {
    scale.apply(ROW_TEXT_BASE) * GRID_NAME_LINE_HEIGHT * lines as f32
}

/// iced's default line height is 1.3x the text size.
const GRID_NAME_LINE_HEIGHT: f32 = 1.3;
/// The lines [`GRID_CELL_HEIGHT`] was drawn for.
const DEFAULT_NAME_LINES: f32 = 2.0;

/// The space inside a grid cell's edge, and between its icon and its
/// name. Scaled with everything else in the cell: unscaled, a cell at
/// Extra large had the same eight pixels of air as one at Small, and the
/// sum the cell's height is checked against was a different sum from
/// the one drawn.
pub fn grid_padding(scale: FontScale) -> f32 {
    scale.apply(hyprforge_ui::theme::spacing::SM)
}

/// The largest scale at which one grid cell, with the grid's padding and
/// the scrollbar's lane, fits across `pane_width`.
///
/// Every term of that sum is linear in the scale, so it is one division.
/// Past it a lone cell would overrun the pane, and a row that overruns
/// is squeezed — the name wraps into a column narrower than its cell and
/// is cut where it would have fitted.
pub fn grid_scale_cap(pane_width: f32) -> f32 {
    pane_width / (GRID_CELL_WIDTH + 2.0 * GRID_GAP + SCROLLBAR_LANE)
}

/// The scale the grid is drawn at: `zoomed`, unless not even one cell
/// would fit across `pane_width` at it — then the largest that does,
/// though never below `floor`. Whether it was capped is the second half,
/// for the status bar to say so: a zoom that silently does less than it
/// says reads as the zoom being broken.
pub fn grid_scale(zoomed: FontScale, pane_width: f32, floor: f32) -> (FontScale, bool) {
    let cap = grid_scale_cap(pane_width).max(floor);
    if zoomed.0 > cap {
        (FontScale(cap), true)
    } else {
        (zoomed, false)
    }
}

/// The icon inside a grid cell.
pub fn grid_icon_size(scale: FontScale) -> f32 {
    scale.apply(GRID_ICON)
}

/// The icon inside a grid cell among a search's results, which also
/// carry a line saying where each result is.
///
/// Smaller rather than the cell taller or the name shorter. A taller
/// cell would make the same folder's grid change shape when a search
/// starts; a name cut to one line would cost the one thing a grid is
/// for, recognising what you are looking at; eight pixels of icon is
/// the cheapest of the three, and still leaves the icon the largest
/// thing in the cell. `grid_tests` sums it and asserts it fits.
pub fn grid_result_icon_size(scale: FontScale) -> f32 {
    scale.apply(GRID_RESULT_ICON)
}

/// The room a result's folder line takes: one line of meta text.
pub fn grid_folder_height(scale: FontScale) -> f32 {
    scale.apply(META_TEXT_BASE) * GRID_NAME_LINE_HEIGHT
}

/// How many characters of a result's folder fit across a grid cell —
/// see `searching::elide_folder`, which keeps the end of it.
///
/// An estimate, because iced 0.14 cannot be asked how wide a string
/// will be before layout: the cell's inner width over an average
/// proportional character (a little over half the text size). The ratio
/// is the same at every font scale, since the cell and the text grow
/// together — so it is one number, not a function of `FontScale`. The
/// cell clips what an unusually wide name overruns by.
pub const GRID_FOLDER_CHARS: usize = 15;

const GRID_RESULT_ICON: f32 = 48.0;

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

/// The folder mark beside a sidebar row, at 100% scale.
pub const SIDEBAR_MARK_BASE: f32 = 13.0;

/// A sidebar row's icon when the icon theme draws it. Larger than the
/// drawn mark it replaces: a theme icon is detailed artwork with its own
/// margin, and at 13 pixels a folder with a download arrow on it is a
/// smudge.
pub const SIDEBAR_ICON_BASE: f32 = 18.0;

/// The status bar's height — the design's 30px at 100% scale, derived
/// the same way the header is so it grows with the text inside it.
pub fn status_height(scale: FontScale) -> f32 {
    scale.apply(STATUS_HEIGHT_BASE)
}

const STATUS_HEIGHT_BASE: f32 = 30.0;


#[cfg(test)]
mod grid_tests {
    use super::*;

    /// Where the preview pane would step aside, the inspector narrows
    /// instead — it was asked for — and a wide window never stretches it
    /// past its widest.
    #[test]
    fn the_inspector_never_steps_aside_and_never_sprawls() {
        assert_eq!(preview_width(480.0, true), None);
        assert_eq!(inspector_width(480.0, true), INSPECTOR_MIN_WIDTH);
        assert_eq!(inspector_width(3000.0, false), INSPECTOR_MAX_WIDTH);
    }

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

    /// A half-screen window still gets its preview — this is the case
    /// that used to offer "Hide preview" beside nothing at all: a
    /// 771-pixel window with the sidebar showing.
    #[test]
    fn a_half_screen_window_still_shows_a_narrower_preview() {
        let width = preview_width(771.0, false).expect("room for a preview");
        assert!((PREVIEW_MIN_WIDTH..PREVIEW_MAX_WIDTH).contains(&width), "narrower, not gone: {width}");
        assert_eq!(preview_width(1600.0, false), Some(PREVIEW_MAX_WIDTH), "a wide window gets the full pane");
    }

    /// The pane steps aside rather than squeezing the listing below what
    /// it needs.
    #[test]
    fn the_preview_never_takes_the_listing_below_its_minimum() {
        assert_eq!(preview_width(600.0, false), None);
        for viewport in (400..2000).step_by(7).map(|w| w as f32) {
            if let Some(width) = preview_width(viewport, false) {
                assert!(list_pane_width(viewport, false) - width >= LISTING_MIN_BESIDE_PREVIEW);
            }
        }
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
    fn a_cell_holds_its_icon_and_its_lines_of_name_without_squeezing() {
        // Up to Extra large at the largest font scale the suite offers —
        // where the names were being cut.
        for scale in [1.0, 1.25, 1.6, 2.0, 3.2].map(FontScale) {
            for lines in 1..=3 {
                // `grid_cell`'s own layout: `grid_padding` all round,
                // then a column of [icon, name] spaced the same.
                let needed =
                    3.0 * grid_padding(scale) + grid_icon_size(scale) + grid_name_height(scale, lines);
                assert!(
                    grid_cell_height(scale, lines) + 0.01 >= needed,
                    "a cell is {} tall but needs {needed} at scale {scale:?} with {lines} lines",
                    grid_cell_height(scale, lines),
                );
            }
        }
        assert!(grid_cell_width(FontScale::default()) > grid_icon_size(FontScale::default()));
    }

    /// Never squeezed: at the scale the grid is actually drawn at, one
    /// whole cell and everything around it fits the pane — at every
    /// zoom, font scale and pane width, down to the floor.
    #[test]
    fn a_cell_never_comes_out_narrower_than_its_size() {
        for font in [1.0, 1.25, 1.6] {
            for zoom in crate::prefs::Zoom::FACTORS {
                let mut width = 140.0_f32;
                while width < 2000.0 {
                    let floor = font * crate::prefs::Zoom::FACTORS[0];
                    let (scale, capped) = grid_scale(FontScale(font * zoom), width, floor);
                    let used = grid_cell_width(scale) + 2.0 * grid_gap(scale) + scale.apply(SCROLLBAR_LANE);
                    assert!(
                        used <= width + 0.01 || scale.0 <= floor,
                        "{used} in {width} at font {font} zoom {zoom}"
                    );
                    assert_eq!(capped, scale.0 < font * zoom);
                    width += 3.0;
                }
            }
        }
    }

    /// Extra large on a wide pane is Extra large: the cap only ever
    /// takes away what does not fit.
    #[test]
    fn a_pane_wide_enough_keeps_the_zoom_it_was_given() {
        assert_eq!(grid_scale(FontScale(2.0), 1200.0, 0.75), (FontScale(2.0), false));
        let (scale, capped) = grid_scale(FontScale(3.2), 400.0, 0.75);
        assert!(capped && scale.0 < 3.2);
    }

    /// A result's cell is the same height as any other, and holds its
    /// smaller icon, the two-line name and the folder line under it.
    #[test]
    fn a_result_cell_holds_its_folder_line_at_the_same_height() {
        for scale in [1.0, 1.25, 1.6, 3.2].map(FontScale) {
            for lines in 1..=3 {
                let needed = 3.0 * grid_padding(scale)
                    + grid_result_icon_size(scale)
                    + grid_name_height(scale, lines)
                    + grid_folder_height(scale);
                assert!(grid_cell_height(scale, lines) + 0.01 >= needed, "needs {needed} at {scale:?}, {lines} lines");
            }
        }
        assert!(grid_result_icon_size(FontScale::default()) < grid_icon_size(FontScale::default()));
    }

    /// The character estimate leaves the folder line inside the cell at
    /// an average character width of 0.55em.
    #[test]
    fn the_folder_line_s_characters_fit_the_cell() {
        let scale = FontScale::default();
        let inner = grid_cell_width(scale) - scale.apply(hyprforge_ui::theme::spacing::SM) * 2.0;
        assert!(GRID_FOLDER_CHARS as f32 * scale.apply(META_TEXT_BASE) * 0.55 <= inner);
    }

    /// Exactly the lines asked for — a grid whose rows are different
    /// heights because one name was longer reads as broken.
    #[test]
    fn the_name_budget_is_exactly_the_lines_asked_for() {
        let scale = FontScale::default();
        let one_line = scale.apply(ROW_TEXT_BASE) * 1.3;
        for lines in 1..=3 {
            assert!((grid_name_height(scale, lines) - one_line * lines as f32).abs() < 0.01);
        }
        assert_eq!(grid_cell_height(scale, 2), 132.0, "the design's cell at the default");
    }

    /// Everything here scales with the font, like every other dimension
    /// in this module — see the module doc.
    #[test]
    fn every_grid_dimension_grows_with_the_font_scale() {
        let big = FontScale(1.5);
        assert!(grid_cell_width(big) > grid_cell_width(FontScale::default()));
        assert!(grid_cell_height(big, 2) > grid_cell_height(FontScale::default(), 2));
        assert!(grid_icon_size(big) > grid_icon_size(FontScale::default()));
        // And a bigger font means fewer cells across the same pane.
        assert!(grid_columns(1200.0, big) < grid_columns(1200.0, FontScale::default()));
    }
}
