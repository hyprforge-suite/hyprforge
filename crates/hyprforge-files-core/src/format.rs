//! Turning a byte count, or a modification time, into something a person
//! reads at a glance.
//!
//! Pure, and kept separate from [`crate::types::Entry`] itself so the
//! boundary math (where "999 B" becomes "1.0 KiB") is one function with
//! its own tests, rather than an inline `format!` call duplicated once
//! for the list view and once for the grid view.

use std::time::SystemTime;

/// The binary unit ladder, `1024` a step — matching what `stat`, `du`
/// and every mainstream file manager actually compute with, even where
/// they print a decimal-looking label. The brief's own boundary values
/// (999/1000/1023/1024) only make sense against `1024`, not `1000`: a
/// decimal ladder would put both 999 and 1000 on opposite sides of a
/// unit change, and it does not.
const UNITS: &[&str] = &["B", "KiB", "MiB", "GiB", "TiB", "PiB", "EiB"];

/// Formats `bytes` as a human-readable size.
///
/// Anything under 1024 bytes is shown as a bare integer — "999 B", "1000
/// B" — because a fractional byte count (`0.98 KiB`) is a worse answer
/// than the exact number when the exact number is this small. From 1024
/// up, one decimal place and the next unit, walking the ladder until the
/// value is back under 1024 or the units run out.
pub fn human_readable_size(bytes: u64) -> String {
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    format!("{value:.1} {}", UNITS[unit])
}

/// Formats a modification time the way the design's list view shows it:
/// local time, a fixed short calendar form so a column of these lines up
/// rather than each row producing a differently-shaped string.
///
/// `None` — [`crate::types::Entry::modified`]'s own state for "the
/// filesystem wouldn't say" — renders as an em dash rather than an empty
/// cell, so a missing timestamp reads as "unknown" and not as a blank
/// the eye skips over.
pub fn format_modified(modified: Option<SystemTime>) -> String {
    match modified {
        Some(t) => chrono::DateTime::<chrono::Local>::from(t)
            .format("%b %-d, %Y %H:%M")
            .to_string(),
        None => "\u{2014}".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_modified_time_is_an_em_dash_not_a_blank_cell() {
        assert_eq!(format_modified(None), "\u{2014}");
    }

    #[test]
    fn a_known_time_formats_as_a_fixed_short_calendar_form() {
        // 2024-01-05T09:03:00Z, chosen because a single-digit day and
        // hour exercise the zero-padding decisions in the format string.
        let t = SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1704445380);
        let formatted = format_modified(Some(t));
        // The exact clock reading depends on the local timezone this
        // test runs in, but the *shape* — "Mon D, YYYY HH:MM" — must not
        // depend on it, so only the shape is pinned here.
        assert!(formatted.contains("2024"), "{formatted}");
        assert_eq!(formatted.matches(':').count(), 1, "{formatted}");
    }

    #[test]
    fn nine_hundred_ninety_nine_bytes_has_no_unit_change() {
        assert_eq!(human_readable_size(999), "999 B");
    }

    #[test]
    fn one_thousand_bytes_is_still_plain_bytes() {
        // The decimal-looking boundary that a `>= 1000` off-by-one would
        // get wrong if this ladder were base-1000 instead of base-1024.
        assert_eq!(human_readable_size(1000), "1000 B");
    }

    #[test]
    fn ten_twenty_three_bytes_is_the_last_value_still_in_bytes() {
        assert_eq!(human_readable_size(1023), "1023 B");
    }

    #[test]
    fn ten_twenty_four_bytes_crosses_into_kibibytes() {
        assert_eq!(human_readable_size(1024), "1.0 KiB");
    }

    #[test]
    fn the_largest_unit_is_reached_without_running_off_the_ladder() {
        // u64::MAX is a little under 16 EiB; the ladder must stop at the
        // last unit rather than index past it.
        assert_eq!(human_readable_size(u64::MAX), "16.0 EiB");
    }

    #[test]
    fn a_mid_ladder_value_rounds_to_one_decimal_place() {
        assert_eq!(human_readable_size(1024 * 1024 * 3 / 2), "1.5 MiB");
    }

    #[test]
    fn zero_bytes_is_plain() {
        assert_eq!(human_readable_size(0), "0 B");
    }
}
