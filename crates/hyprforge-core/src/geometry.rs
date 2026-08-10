//! Converting a monitor's pixel size into the space it occupies in the
//! compositor's layout, which is not the arithmetic it looks like.
//!
//! Everything here follows from one fact: **Hyprland works in 120ths of a
//! scale step.** That's the unit `wp_fractional_scale_v1` is defined in, and
//! an incoming scale is rounded onto that grid before anything else happens
//! to it. So the scale in force is never quite the one that was sent, and
//! the set of scales a panel can take is far smaller than it looks.

/// The grid Hyprland snaps every incoming scale onto, per
/// `wp_fractional_scale_v1`.
const SCALE_STEPS_PER_UNIT: f64 = 120.0;

/// How close to a whole number counts as dividing cleanly.
const CLEAN_EPSILON: f64 = 1e-6;

/// The scale actually in force when `scale` is what was asked for or read
/// back: the nearest 120th.
///
/// This is why a scale never round-trips. 1.6 isn't representable over
/// `wl_fixed` (1/256ths) and arrives as 1.6015625; Hyprland rounds that to
/// 192/120 and runs exactly 1.6. Reading it back without re-snapping gives
/// 2560/1.6015625 = 1598.4 for a panel the compositor is laying out 1600
/// wide — and a neighbour placed at 1598 lands *inside* it.
fn quantize(scale: f64) -> f64 {
    (scale * SCALE_STEPS_PER_UNIT).round() / SCALE_STEPS_PER_UNIT
}

/// The layout space a `pixels`-wide output at `scale` occupies.
///
/// Layout coordinates are logical, so this divides — scaling a display *up*
/// makes it cover *less* layout space.
///
/// The division is done against the snapped scale, which is the one the
/// compositor is really using, so for any scale it accepted the answer is
/// exact. A scale it *didn't* accept has no true logical size to report; the
/// result is rounded up there, because erring large costs a pixel or two of
/// gap between monitors while erring small overlaps them, which Hyprland
/// warns "will cause issues".
pub fn logical_size(pixels: i32, scale: f64) -> i32 {
    if scale <= 0.0 {
        return pixels;
    }
    let exact = pixels as f64 / quantize(scale);
    if (exact - exact.round()).abs() < CLEAN_EPSILON {
        exact.round() as i32
    } else {
        exact.ceil() as i32
    }
}

fn gcd(a: i64, b: i64) -> i64 {
    let (mut a, mut b) = (a.abs(), b.abs());
    while b != 0 {
        let t = b;
        b = a % b;
        a = t;
    }
    a
}

/// The nearest scale to `nominal` that a `width`x`height` output can
/// actually be set to.
///
/// Hyprland rounds an incoming scale to the nearest 120th and then requires
/// that it divide the resolution cleanly in both axes. So a usable scale is
/// `k/120` where `k` divides both `width*120` and `height*120` — that is,
/// `k` divides `gcd(width*120, height*120)` — and nothing else qualifies.
///
/// On a 2560x1600 panel that gcd is 38400, so 192/120 (1.6) and 200/120
/// (5/3) are available, while 1.75 and 1.5 are not: 210 and 180 don't divide
/// 38400. Those two are exactly the values Hyprland rejects on this machine,
/// answering "found suggestion 1.6666666" and "found suggestion 1.6".
///
/// It's worth being precise about why the obvious rule is wrong. Asking only
/// that `width/scale` and `height/scale` be whole admits any `gcd(w,h)/k`,
/// which for 175% gives 320/183 — arithmetically clean, and rejected all the
/// same, because it isn't on the 120ths grid. Offering it doesn't fail
/// loudly either: the compositor substitutes a scale of its own, the setting
/// then disagrees with reality, and every position derived from it is
/// computed against a width nothing is using.
///
/// `k = 120` always divides that gcd, so a scale of 1.0 is always available
/// and the search below always terminates.
pub fn nearest_valid_scale(width: i32, height: i32, nominal: f64) -> f64 {
    if width <= 0 || height <= 0 || nominal <= 0.0 {
        return nominal;
    }
    let steps = SCALE_STEPS_PER_UNIT as i64;
    let g = gcd(width as i64 * steps, height as i64 * steps);
    let target = (nominal * SCALE_STEPS_PER_UNIT).round().max(1.0) as i64;
    if g % target == 0 {
        return target as f64 / SCALE_STEPS_PER_UNIT;
    }
    // Walk outward from the asked-for step until a divisor turns up, taking
    // the smaller (larger scale) on a tie — a slightly smaller desktop is a
    // kinder miss than a slightly larger one on a panel someone chose to
    // scale up.
    for delta in 1..=target {
        for k in [target - delta, target + delta] {
            if k >= 1 && g % k == 0 {
                return k as f64 / SCALE_STEPS_PER_UNIT;
            }
        }
    }
    1.0
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Both halves of the rule, checked the way the compositor checks them:
    /// snap to a 120th, then divide.
    fn is_valid(width: i32, height: i32, scale: f64) -> bool {
        let s = quantize(scale);
        (s - scale).abs() < 1e-9
            && (width as f64 / s).fract().abs() < 1e-9
            && (height as f64 / s).fract().abs() < 1e-9
    }

    #[test]
    fn valid_scales_survive_the_120ths_grid_and_divide_both_axes() {
        for nominal in [1.0, 1.25, 1.5, 1.75, 2.0, 2.25] {
            let s = nearest_valid_scale(2560, 1600, nominal);
            assert!(is_valid(2560, 1600, s), "{nominal} -> {s}, which Hyprland would reject");
        }
    }

    /// The two Hyprland actually rejected on the 2560x1600 panel, with the
    /// suggestions it printed. Getting these right is the whole point.
    #[test]
    fn we_land_on_the_same_scale_hyprland_suggests() {
        // "Invalid scale passed to monitor, 1.7460938 found suggestion 1.6666666"
        let s = nearest_valid_scale(2560, 1600, 1.75);
        assert!((s - 5.0 / 3.0).abs() < 1e-9, "expected 5/3 for 175%, got {s}");
        // "Invalid scale passed to monitor, 1.5 found suggestion 1.6"
        let s = nearest_valid_scale(2560, 1600, 1.5);
        assert!((s - 1.6).abs() < 1e-9, "expected 1.6 for 150%, got {s}");
    }

    /// The rule this replaced admitted any gcd(w,h)/k, which for 175% gives
    /// 320/183 = 1.748634. It divides both axes into whole numbers and
    /// Hyprland still refuses it, because it isn't a 120th.
    #[test]
    fn an_arithmetically_clean_scale_off_the_grid_is_not_offered() {
        let off_grid = 320.0 / 183.0;
        assert_eq!(2560.0 / off_grid, 1464.0);
        assert!(!is_valid(2560, 1600, off_grid), "the old rule's answer must not pass");
        assert!(nearest_valid_scale(2560, 1600, 1.75) != off_grid);
    }

    #[test]
    fn a_valid_nominal_scale_is_returned_unchanged() {
        assert_eq!(nearest_valid_scale(2560, 1600, 1.6), 1.6);
        assert_eq!(nearest_valid_scale(2560, 1600, 2.0), 2.0);
        assert_eq!(nearest_valid_scale(1920, 1080, 1.0), 1.0);
    }

    /// Snapping something already valid must be a no-op, or auto-learn
    /// reading a scale back and re-applying it would walk it a step at a
    /// time.
    #[test]
    fn snapping_is_idempotent() {
        for nominal in [1.0, 1.25, 1.5, 1.75, 2.0, 2.25, 3.0] {
            let once = nearest_valid_scale(2560, 1600, nominal);
            assert_eq!(once, nearest_valid_scale(2560, 1600, once));
        }
    }

    #[test]
    fn nonsense_input_is_passed_through_rather_than_dividing_by_zero() {
        assert_eq!(nearest_valid_scale(0, 0, 1.5), 1.5);
        assert_eq!(nearest_valid_scale(2560, 1600, 0.0), 0.0);
    }

    #[test]
    fn an_exact_scale_gives_an_exact_size() {
        assert_eq!(logical_size(3840, 2.0), 1920);
        assert_eq!(logical_size(2560, 1.25), 2048);
        assert_eq!(logical_size(1920, 1.0), 1920);
        assert_eq!(logical_size(2560, 1.6), 1600);
    }

    /// The cases that caused monitors to overlap in practice: the scale the
    /// compositor is really using can't be sent over `wl_fixed`, so the
    /// value read back is a hair off the 120th actually in force. Snapping
    /// recovers the compositor's own number exactly — no erring-large
    /// fudge needed.
    #[test]
    fn a_quantized_scale_reports_the_size_the_compositor_uses() {
        // 1.6 as sent over the wire.
        assert_eq!(logical_size(2560, 1.6015625), 1600);
        // 5/3 as sent over the wire.
        assert_eq!(logical_size(2560, 1.66796875), 1536);
        assert_eq!(logical_size(1600, 1.66796875), 960);
    }

    /// A scale the compositor would have refused has no true logical size.
    /// Round up there: a pixel of gap is invisible, a pixel of overlap is an
    /// error.
    #[test]
    fn an_impossible_scale_errs_large_rather_than_small() {
        let got = logical_size(2560, 1.75);
        assert!(got as f64 >= 2560.0 / 1.75, "{got} would overlap its neighbour");
        assert!(got - 1462 <= 2, "{got} is further off than a seam's worth");
    }

    #[test]
    fn a_nonsense_scale_does_not_divide_by_zero() {
        assert_eq!(logical_size(1920, 0.0), 1920);
        assert_eq!(logical_size(1920, -1.0), 1920);
    }
}
