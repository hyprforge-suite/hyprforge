//! Row/header/type-ramp sizing every Hyprforge app shares, derived from
//! font metrics rather than copied out of a mockup as pixel constants.
//!
//! Files' design came first and moved here when Settings adopted the
//! same one, so the two draw a row, a bar and a corner the same size.
//! What only Files needs — the grid, its menus, its sidebar — stayed in
//! `hyprforge-files-core`.
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
//!   unscaled and handed to [`crate::widgets::scaled_text`] (or
//!   [`meta_text`](crate::widgets::meta_text)) the same way
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

use crate::theme::{self, FontScale, BASE_TEXT_SIZE};

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

/// A sidebar section heading — `PLACES`, `PINNED`, `TRASH`.
///
/// Smaller than the meta text it is built from, because a heading in a
/// sidebar is a signpost rather than something to read: it should be
/// findable when looked for and invisible when not.
pub const SECTION_LABEL_BASE: f32 = META_TEXT_BASE * 0.85;


/// A setting row's least height — the design's 52px card row at 100%.
///
/// A floor, not a height: a row carrying a hint line under its label
/// grows past it rather than clipping the hint, and the floor is what
/// keeps a page of one-line rows on a steady rhythm.
pub fn setting_row_height(scale: FontScale) -> f32 {
    scale.apply(SETTING_ROW_HEIGHT_BASE)
}

const SETTING_ROW_HEIGHT_BASE: f32 = 52.0;

#[cfg(test)]
mod tests {
    use super::*;

    // These pin the design's literal numbers at the scale they were
    // measured at. `Theme::rounding` defaults to 12 and nothing in this
    // crate's test suite ever calls `theme::init`, so the outer/inner
    // assertions below hold regardless of test order — `theme::init`'s
    // `OnceLock` only ever accepts the first call in the whole test
    // binary, and a test that installed a theme would decide it for
    // every other test here.

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

    /// A setting row is a floor for a one-line row, so it must hold one
    /// line of row text at any scale — and still be the design's 52px at
    /// 100%.
    #[test]
    fn a_setting_row_holds_its_label_at_every_scale() {
        assert_eq!(setting_row_height(FontScale::default()), 52.0);
        for tenths in 5..=30 {
            let scale = FontScale(tenths as f32 / 10.0);
            assert!(scale.apply(ROW_TEXT_BASE) < setting_row_height(scale));
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
