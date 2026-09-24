//! Turning the three different ways these formats record a time into one
//! [`SystemTime`].
//!
//! tar stores Unix seconds and needs nothing. 7z stores Windows FILETIME
//! — 100-nanosecond ticks since 1601 — and needs an epoch shift. zip
//! stores a DOS date and time as *civil fields* with no timezone at all:
//! the year, month, day, hour, minute and second that were showing on
//! the clock of whichever machine wrote the archive, wherever that was.
//!
//! There is no correct answer for that last one, only a stated one. This
//! reads a zip's fields as UTC, which is what `unzip` and every other
//! reader that does not guess does, and means an archive written at
//! noon in one timezone shows as noon everywhere — the same time in
//! different places rather than a different time in each. Guessing the
//! reader's own zone instead would silently shift every timestamp in
//! every archive that crossed a border.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Unix seconds to a [`SystemTime`], before or after 1970.
pub fn from_unix(seconds: i64) -> SystemTime {
    if seconds >= 0 {
        UNIX_EPOCH + Duration::from_secs(seconds as u64)
    } else {
        UNIX_EPOCH - Duration::from_secs(seconds.unsigned_abs())
    }
}

/// Windows FILETIME — 100ns ticks since 1601-01-01 UTC — to a
/// [`SystemTime`]. `None` for a zero, which is 7z's way of saying the
/// field was never set rather than a claim about the seventeenth
/// century.
pub fn from_filetime(ticks: u64) -> Option<SystemTime> {
    if ticks == 0 {
        return None;
    }
    /// Seconds between 1601-01-01 and 1970-01-01.
    const EPOCH_SHIFT: u64 = 11_644_473_600;
    let seconds = ticks / 10_000_000;
    let nanos = (ticks % 10_000_000) as u32 * 100;
    let unix = seconds.checked_sub(EPOCH_SHIFT)?;
    Some(UNIX_EPOCH + Duration::from_secs(unix) + Duration::from_nanos(nanos as u64))
}

/// Civil fields, read as UTC — see the module doc for why UTC and not
/// something cleverer.
///
/// `None` for a date that is not a date: DOS packs the month and day
/// into four and five bits, so `month = 0` and `day = 0` are both
/// representable and both appear in real archives written by tools that
/// left the field empty.
pub fn from_civil_utc(
    year: i32,
    month: u32,
    day: u32,
    hour: u32,
    minute: u32,
    second: u32,
) -> Option<SystemTime> {
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    if hour > 23 || minute > 59 || second > 60 {
        return None;
    }
    let days = days_from_civil(year, month, day)?;
    let seconds = days * 86_400 + (hour * 3600 + minute * 60 + second) as i64;
    Some(from_unix(seconds))
}

/// Days since 1970-01-01 for a civil date.
///
/// Howard Hinnant's `days_from_civil`, which is the algorithm `chrono`
/// and every C++ standard library date implementation uses. Written out
/// rather than pulled in with a calendar crate: this crate is a leaf
/// (see the crate doc) and one date conversion is not worth a
/// dependency, and unlike a hand-rolled leap-year rule this one is
/// exact for every year rather than for the ones someone thought to
/// test.
fn days_from_civil(year: i32, month: u32, day: u32) -> Option<i64> {
    let y = if month <= 2 { year - 1 } else { year } as i64;
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400; // [0, 399]
    let m = month as i64;
    let d = day as i64;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1; // [0, 365]
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy; // [0, 146096]
    Some(era * 146_097 + doe - 719_468)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unix_of(time: SystemTime) -> i64 {
        match time.duration_since(UNIX_EPOCH) {
            Ok(d) => d.as_secs() as i64,
            Err(e) => -(e.duration().as_secs() as i64),
        }
    }

    #[test]
    fn the_epoch_itself_round_trips() {
        assert_eq!(unix_of(from_civil_utc(1970, 1, 1, 0, 0, 0).unwrap()), 0);
    }

    #[test]
    fn a_known_date_matches_the_number_date_lists_for_it() {
        // 2021-01-01T00:00:00Z — checked against `date -u -d ... +%s`.
        assert_eq!(unix_of(from_civil_utc(2021, 1, 1, 0, 0, 0).unwrap()), 1_609_459_200);
        // A leap day, because February is where a hand-rolled version
        // of this would go wrong.
        assert_eq!(unix_of(from_civil_utc(2024, 2, 29, 12, 0, 0).unwrap()), 1_709_208_000);
        // 2000 is a leap year and 1900 was not — the rule most
        // shortcuts get wrong.
        assert_eq!(unix_of(from_civil_utc(2000, 3, 1, 0, 0, 0).unwrap()), 951_868_800);
    }

    #[test]
    fn an_empty_dos_date_is_no_date_rather_than_the_year_zero() {
        assert_eq!(from_civil_utc(1980, 0, 0, 0, 0, 0), None);
        assert_eq!(from_civil_utc(1980, 13, 1, 0, 0, 0), None);
    }

    #[test]
    fn an_unset_filetime_is_no_date_rather_than_1601() {
        assert_eq!(from_filetime(0), None);
    }

    #[test]
    fn a_filetime_lands_on_the_second_it_names() {
        // 1970-01-01T00:00:00Z in FILETIME ticks.
        assert_eq!(unix_of(from_filetime(116_444_736_000_000_000).unwrap()), 0);
    }

    #[test]
    fn a_time_before_1970_does_not_wrap_around() {
        let before = from_civil_utc(1969, 12, 31, 23, 59, 59).unwrap();
        assert_eq!(unix_of(before), -1);
        assert!(before < UNIX_EPOCH);
    }
}
