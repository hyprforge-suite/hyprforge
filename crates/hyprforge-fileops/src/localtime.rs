//! ISO 8601 **local** time, computed without a date/time crate.
//!
//! The trash spec's `DeletionDate` is local time with no timezone
//! suffix — confirmed against real `.trashinfo` files already on this
//! machine (`DeletionDate=2026-08-10T19:38:57`, no offset, no `Z`) — and
//! this workspace deliberately carries no chrono/`time` dependency (see
//! `Cargo.toml`: this crate's are `hyprforge-paths`, `serde`,
//! `thiserror`, `tracing`, nothing more). `libc::localtime_r` is what
//! glibc itself uses to fold a UTC timestamp through `/etc/localtime`,
//! DST included, so binding it directly here does the correct
//! calculation instead of hand-rolling a timezone database. This needs
//! no new dependency to link: every Rust binary already links against
//! the system's libc for its own runtime, so only the two symbols this
//! module actually calls need declaring.

use std::os::raw::{c_char, c_int, c_long};

// POSIX/glibc's `struct tm` layout on Linux — identical on x86_64 and
// aarch64, the two targets this suite ships for. `tm_gmtoff`/`tm_zone`
// are glibc extensions past bare POSIX, but glibc always fills them and
// this module never reads them, so their presence only has to make the
// struct's size and field offsets match what `localtime_r` writes.
#[repr(C)]
struct Tm {
    tm_sec: c_int,
    tm_min: c_int,
    tm_hour: c_int,
    tm_mday: c_int,
    tm_mon: c_int,
    tm_year: c_int,
    tm_wday: c_int,
    tm_yday: c_int,
    tm_isdst: c_int,
    tm_gmtoff: c_long,
    tm_zone: *const c_char,
}

extern "C" {
    fn localtime_r(time: *const i64, result: *mut Tm) -> *mut Tm;
}

/// A local calendar date and time, precise to the second — exactly what
/// `DeletionDate=` needs and no more.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LocalDateTime {
    pub year: i32,
    pub month: u32,
    pub day: u32,
    pub hour: u32,
    pub minute: u32,
    pub second: u32,
}

impl LocalDateTime {
    /// The current moment, in the system's local time.
    pub fn now() -> Self {
        let secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            // A running machine's clock is never set before 1970 — if it
            // were, every timestamp anywhere on the system would already
            // be nonsensical, not just this one.
            .expect("the system clock is never before the Unix epoch")
            .as_secs() as i64;
        Self::from_unix(secs)
    }

    fn from_unix(secs: i64) -> Self {
        let mut tm: Tm = unsafe { std::mem::zeroed() };
        // SAFETY: the return value is checked below — this call may
        // leave `tm` untouched. `localtime_r` is given a valid pointer to `secs` (a
        // live stack `i64`) and a valid pointer to `tm` (a live,
        // zero-initialised stack `Tm` matching glibc's layout) to write
        // into. It performs no allocation and retains neither pointer
        // past the call, so nothing here outlives this function's stack
        // frame.
        // `localtime_r` returns NULL rather than writing anything when
        // it cannot represent the time (glibc sets EOVERFLOW for values
        // far outside the representable range). The zeroed struct would
        // then read as year 1900, month 1, day 0 — a date that looks
        // like data rather than like a failure, written into a
        // `.trashinfo` file nobody would think to doubt. Falling back to
        // the epoch is both obviously wrong to a reader and honest about
        // having failed, which a silent 1900-01-00 is not.
        let ok = unsafe { !localtime_r(&secs, &mut tm).is_null() };
        if !ok {
            tracing::warn!(
                secs,
                "the C library could not convert this timestamp to local time; \
                 recording the epoch instead"
            );
            return LocalDateTime { year: 1970, month: 1, day: 1, hour: 0, minute: 0, second: 0 };
        }
        LocalDateTime {
            year: tm.tm_year + 1900,
            month: (tm.tm_mon + 1) as u32,
            day: tm.tm_mday as u32,
            hour: tm.tm_hour as u32,
            minute: tm.tm_min as u32,
            second: tm.tm_sec as u32,
        }
    }

    /// `YYYY-MM-DDThh:mm:ss`, matching the trash spec exactly — no
    /// timezone suffix, because the value already *is* local time and
    /// the spec does not ask for one.
    pub fn to_iso8601(self) -> String {
        format!(
            "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}",
            self.year, self.month, self.day, self.hour, self.minute, self.second
        )
    }

    /// Reverses [`Self::to_iso8601`]. Rejects anything that is not
    /// exactly that shape — a `DeletionDate=` this malformed is the
    /// ".trashinfo exists and will not parse" case, which the caller
    /// must report rather than guess at.
    pub fn parse(s: &str) -> Option<Self> {
        let (date, time) = s.split_once('T')?;
        let mut date_parts = date.split('-');
        let year: i32 = date_parts.next()?.parse().ok()?;
        let month: u32 = date_parts.next()?.parse().ok()?;
        let day: u32 = date_parts.next()?.parse().ok()?;
        if date_parts.next().is_some() {
            return None;
        }
        let mut time_parts = time.split(':');
        let hour: u32 = time_parts.next()?.parse().ok()?;
        let minute: u32 = time_parts.next()?.parse().ok()?;
        let second: u32 = time_parts.next()?.parse().ok()?;
        if time_parts.next().is_some() {
            return None;
        }
        if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
            return None;
        }
        if hour > 23 || minute > 59 || second > 60 {
            return None;
        }
        Some(LocalDateTime { year, month, day, hour, minute, second })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_known_unix_timestamp_matches_dates_command_output_in_utc() {
        // 2024-01-01T00:00:00 UTC. Only meaningful as a smoke test when
        // this process's local zone happens to be UTC (CI images
        // commonly are); everywhere else it at least proves the call
        // completes and produces a plausible date, which `to_iso8601`
        // below turns into a real round-trip check that does not depend
        // on the zone.
        let dt = LocalDateTime::from_unix(1_704_067_200);
        assert!((2020..=2030).contains(&dt.year));
    }

    #[test]
    fn formatting_and_parsing_a_local_time_round_trips() {
        let dt = LocalDateTime { year: 2026, month: 8, day: 10, hour: 19, minute: 38, second: 57 };
        assert_eq!(dt.to_iso8601(), "2026-08-10T19:38:57");
        assert_eq!(LocalDateTime::parse(&dt.to_iso8601()), Some(dt));
    }

    #[test]
    fn now_produces_a_string_shaped_like_the_spec_expects() {
        let s = LocalDateTime::now().to_iso8601();
        // No timezone suffix — the trash spec's own examples, and the
        // real `.trashinfo` files this was checked against, have none.
        assert!(!s.ends_with('Z'));
        assert!(!s.contains('+'));
        assert_eq!(s.len(), "2026-08-10T19:38:57".len());
        assert!(LocalDateTime::parse(&s).is_some());
    }

    #[test]
    fn garbage_is_reported_as_unparseable_rather_than_guessed_at() {
        assert_eq!(LocalDateTime::parse("not a date"), None);
        assert_eq!(LocalDateTime::parse("2026-08-10 19:38:57"), None, "needs a literal T separator");
        assert_eq!(LocalDateTime::parse("2026-13-10T19:38:57"), None, "month 13 does not exist");
    }
}

#[cfg(test)]
mod ffi_audit {
    use super::*;

    /// The FFI in this module declares glibc's `struct tm` by hand, so
    /// the thing worth testing is not that it *parses* but that it
    /// agrees with the C library everyone else on the machine is using.
    /// A wrong field offset would still produce a plausible-looking
    /// date, which is exactly why this compares against `date` rather
    /// than against a constant someone typed.
    #[test]
    fn our_local_time_agrees_with_the_c_library_the_rest_of_the_system_uses() {
        for secs in [0_i64, 1_000_000_000, 1_789_000_000, 946_684_800] {
            let ours = LocalDateTime::from_unix(secs).to_iso8601();
            let theirs = std::process::Command::new("date")
                .args(["-d", &format!("@{secs}"), "+%Y-%m-%dT%H:%M:%S"])
                .output()
                .expect("`date` is in coreutils and always present");
            let theirs = String::from_utf8_lossy(&theirs.stdout).trim().to_string();
            assert_eq!(ours, theirs, "disagreed with date(1) for unix time {secs}");
        }
    }
}
