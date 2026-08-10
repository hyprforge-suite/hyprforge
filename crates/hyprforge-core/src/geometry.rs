//! Converting a monitor's pixel size into the space it occupies in the
//! compositor's layout, which is not the arithmetic it looks like.

/// Smallest scale difference `wl_fixed` can represent: it carries 1/256ths,
/// so a value is at most half a step from the scale actually in force.
const WL_FIXED_HALF_STEP: f64 = 1.0 / 512.0;

/// How close to a whole number counts as dividing cleanly.
const CLEAN_EPSILON: f64 = 1e-6;

/// The layout space a `pixels`-wide output at `scale` occupies.
///
/// Layout coordinates are logical, so this divides — scaling a display *up*
/// makes it cover *less* layout space. The subtlety is that `scale` usually
/// can't be trusted to full precision.
///
/// Hyprland requires a scale that "divides your resolution cleanly (without
/// decimals)", so its own logical size is always a whole number. But the
/// scale reaches us over `wl_fixed`, in 1/256ths, and most useful scales
/// aren't representable: 1.6 arrives as 1.6015625 and 5/3 as 1.66796875.
/// Dividing by those gives 1598.4 and 1534.8 where the compositor is really
/// using 1600 and 1536 — so rounding puts every neighbouring output one or
/// two pixels *inside* its neighbour, and the compositor complains that the
/// monitors overlap. The error is systematic, not noise: `wl_fixed` rounds
/// to nearest, and a scale rounded up yields a width that comes out short.
///
/// So: when the division is clean, the scale is exact and the answer is
/// exact. When it isn't, the true scale is somewhere within half a step, and
/// this returns the largest width that range allows. Erring large costs at
/// most a pixel or two of gap between monitors, which nothing complains
/// about; erring small produces overlapping outputs, which Hyprland warns
/// "will cause issues".
pub fn logical_size(pixels: i32, scale: f64) -> i32 {
    if scale <= 0.0 {
        return pixels;
    }
    let exact = pixels as f64 / scale;
    if (exact - exact.round()).abs() < CLEAN_EPSILON {
        return exact.round() as i32;
    }
    // Smallest scale the reported value could be standing in for gives the
    // largest width it could really be.
    let lower_scale = (scale - WL_FIXED_HALF_STEP).max(f64::MIN_POSITIVE);
    (pixels as f64 / lower_scale).ceil() as i32
}

fn gcd(a: i32, b: i32) -> i32 {
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
/// Hyprland accepts a scale only if it divides the resolution cleanly in
/// *both* axes — `width/scale` and `height/scale` must each be whole. Write
/// `g = gcd(width, height)`; then any usable scale is `g/k` for a whole `k`,
/// and nothing else qualifies. On a 2560x1600 panel `g` is 320, so 1.6 works
/// (k=200) and so does 5/3 (k=192), while 1.75 and 1.5 do not.
///
/// Offering a scale that isn't of that form doesn't fail loudly: the
/// compositor quietly substitutes one that is. The setting then disagrees
/// with reality, every position derived from it is computed against a width
/// the compositor isn't using, and monitors end up overlapping — which is
/// how a "175%" panel came to be running at 5/3 with its neighbour a few
/// pixels inside it.
pub fn nearest_valid_scale(width: i32, height: i32, nominal: f64) -> f64 {
    if width <= 0 || height <= 0 || nominal <= 0.0 {
        return nominal;
    }
    let g = gcd(width, height) as f64;
    let k = (g / nominal).round().max(1.0);
    g / k
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_scales_divide_both_axes_into_whole_numbers() {
        for nominal in [1.0, 1.25, 1.5, 1.75, 2.0, 2.25] {
            let s = nearest_valid_scale(2560, 1600, nominal);
            let w = 2560.0 / s;
            let h = 1600.0 / s;
            assert!(
                (w - w.round()).abs() < 1e-9 && (h - h.round()).abs() < 1e-9,
                "{nominal} -> {s} gives {w}x{h}, which the compositor would reject"
            );
        }
    }

    #[test]
    fn a_valid_nominal_scale_is_returned_unchanged() {
        assert_eq!(nearest_valid_scale(2560, 1600, 1.6), 1.6);
        assert_eq!(nearest_valid_scale(2560, 1600, 2.0), 2.0);
        assert_eq!(nearest_valid_scale(1920, 1080, 1.0), 1.0);
    }

    /// The specific setting that started this: 175% is not achievable on a
    /// 2560x1600 panel, and silently became 5/3.
    #[test]
    fn an_unachievable_scale_is_moved_to_the_nearest_achievable_one() {
        let s = nearest_valid_scale(2560, 1600, 1.75);
        assert!((s - 1.75).abs() < 0.02, "should stay close to what was asked: {s}");
        // Both axes land on whole numbers, so the compositor takes it as-is
        // instead of substituting something else behind the setting.
        assert!((2560.0 / s).fract().abs() < 1e-9, "width not whole: {}", 2560.0 / s);
        assert!((1600.0 / s).fract().abs() < 1e-9, "height not whole: {}", 1600.0 / s);
        assert_eq!(logical_size(2560, s), 1464);
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
    /// value we read divides short.
    #[test]
    fn a_quantized_scale_never_reports_less_than_the_truth() {
        // 1.6 as sent over the wire; the compositor's own width is 1600.
        assert!(
            logical_size(2560, 1.6015625) >= 1600,
            "computed {} for a monitor the compositor lays out as 1600 wide",
            logical_size(2560, 1.6015625)
        );
        // 5/3 as sent over the wire; the compositor's own width is 1536.
        assert!(
            logical_size(2560, 1.66796875) >= 1536,
            "computed {} for a monitor the compositor lays out as 1536 wide",
            logical_size(2560, 1.66796875)
        );
    }

    /// Erring large is deliberate, but it has to stay small enough to be
    /// invisible — a couple of pixels of gap, not a visible seam.
    #[test]
    fn erring_large_stays_within_a_few_pixels() {
        for (px, wire, truth) in [
            (2560, 1.6015625, 1600),
            (2560, 1.66796875, 1536),
            (3840, 1.5, 2560),
        ] {
            let got = logical_size(px, wire);
            assert!(
                got >= truth && got - truth <= 3,
                "{px}px at {wire}: got {got}, compositor uses {truth}"
            );
        }
    }

    #[test]
    fn a_nonsense_scale_does_not_divide_by_zero() {
        assert_eq!(logical_size(1920, 0.0), 1920);
        assert_eq!(logical_size(1920, -1.0), 1920);
    }
}
