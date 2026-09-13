//! What an inhibit covers, and who holds one, independent of D-Bus.
//!
//! Kept free of `zbus` types so the model can be built and asserted on
//! without a bus, which is what makes the mock backend worth having.

use std::collections::BTreeSet;
use std::fmt;
use std::time::Duration;

/// One thing `systemd-logind` can be told not to do.
///
/// logind's `Inhibit` call takes `what` as a colon-separated string and
/// silently ignores anything it does not recognise — a typo'd `"idel"`
/// inhibits nothing and still returns a valid file descriptor, so the
/// call *looks* like it worked and the machine sleeps anyway. Spelling
/// each value out as a variant here, rather than accepting a `&str`,
/// makes that typo a compile error instead of a machine that sleeps
/// during a call.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum What {
    Shutdown,
    Sleep,
    Idle,
    HandlePowerKey,
    HandleSuspendKey,
    HandleHibernateKey,
    HandleLidSwitch,
}

impl What {
    /// The token logind expects for this value, straight from
    /// `logind.conf(5)` / the `org.freedesktop.login1.Manager` docs.
    pub fn as_str(self) -> &'static str {
        match self {
            What::Shutdown => "shutdown",
            What::Sleep => "sleep",
            What::Idle => "idle",
            What::HandlePowerKey => "handle-power-key",
            What::HandleSuspendKey => "handle-suspend-key",
            What::HandleHibernateKey => "handle-hibernate-key",
            What::HandleLidSwitch => "handle-lid-switch",
        }
    }
}

impl fmt::Display for What {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A non-empty, de-duplicated set of [`What`] values, in the exact form
/// `Inhibit`'s `what` argument wants: colon-separated, e.g. `"sleep:idle"`.
///
/// This is the type that makes a mis-spelled `what` impossible: there is
/// no way to construct one from a free-typed string, only from `What`
/// variants the compiler already checked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WhatSet(BTreeSet<What>);

impl WhatSet {
    /// Builds a set from one or more values. Order does not matter to
    /// logind and duplicates collapse on their own — `BTreeSet` gives a
    /// stable, declaration-order iteration so the assembled string is
    /// deterministic across calls, which matters for the test that pins
    /// its exact form.
    pub fn new(whats: impl IntoIterator<Item = What>) -> Self {
        WhatSet(whats.into_iter().collect())
    }

    /// The "keep awake" toggle this crate exists for: block sleep and
    /// idle, leaving shutdown and the hardware keys alone.
    pub fn keep_awake() -> Self {
        WhatSet::new([What::Sleep, What::Idle])
    }

    pub fn contains(&self, what: What) -> bool {
        self.0.contains(&what)
    }

    /// The exact string to send as `Inhibit`'s `what` argument.
    pub fn to_arg(&self) -> String {
        self.0.iter().map(|w| w.as_str()).collect::<Vec<_>>().join(":")
    }
}

/// One inhibitor as `ListInhibitors` reports it: `(what, who, why, mode,
/// uid, pid)`.
///
/// `what` is kept as the raw string logind returned rather than parsed
/// into a [`WhatSet`]: this describes *someone else's* lock, and another
/// process is free to pass logind a `what` this crate has no variant for
/// (or one logind itself would silently ignore). Re-parsing it into our
/// own restricted type would either drop information from a real lock or
/// have to fail on a value that is perfectly legal on the bus — neither
/// is the right answer for something only ever displayed, not acted on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InhibitorInfo {
    pub what: String,
    pub who: String,
    pub why: String,
    pub mode: String,
    pub uid: u32,
    pub pid: u32,
}

/// What went wrong, in terms a screen can show without saying "check the
/// logs" (vision pillar #3).
///
/// [`InhibitError::Unavailable`] is separate from "nothing is
/// inhibiting" on purpose: a `systemd-logind` that is not answering must
/// never render as an empty inhibitor list, the same distinction
/// `NetworkError::Unavailable` draws for NetworkManager.
#[derive(Debug, thiserror::Error)]
pub enum InhibitError {
    #[error(
        "systemd-logind isn't answering, so the machine's sleep behaviour can't be checked or \
         changed. It's part of systemd and should already be running — try again, or restart \
         the machine if it keeps failing."
    )]
    Unavailable,
    #[error("systemd-logind didn't answer within {0:?}.")]
    TimedOut(std::time::Duration),
    #[error("{0}")]
    Refused(String),
}

/// What a battery is doing, as `org.freedesktop.UPower.Device`'s `State`
/// property reports it.
///
/// Spelled out as a variant, the same choice [`What`] makes and for the
/// same reason: the raw property is a `u32`
/// (`UP_DEVICE_STATE_*` in upower's own headers) with a fixed, small
/// vocabulary, and a value this crate has not seen documented is
/// [`BatteryState::Unknown`] rather than a guess.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BatteryState {
    Charging,
    Discharging,
    Empty,
    FullyCharged,
    PendingCharge,
    PendingDischarge,
    Unknown,
}

impl BatteryState {
    /// `UP_DEVICE_STATE_*`: 1 charging .. 6 pending-discharge, 0 unknown.
    /// Everything else is also folded into `Unknown` — the daemon adding
    /// a new state some day must not become a value this crate panics on
    /// or silently misreports as something it isn't.
    pub fn from_upower(value: u32) -> BatteryState {
        match value {
            1 => BatteryState::Charging,
            2 => BatteryState::Discharging,
            3 => BatteryState::Empty,
            4 => BatteryState::FullyCharged,
            5 => BatteryState::PendingCharge,
            6 => BatteryState::PendingDischarge,
            _ => BatteryState::Unknown,
        }
    }
}

/// Below this, a discharging battery reads as "low" on screen.
///
/// UPower has no opinion of its own on where that line is — this is a
/// choice, made once, here, with a reason written down, rather than
/// scattered as a magic number wherever a screen wants to turn a percent
/// red. Twenty is roughly where laptop vendors' own firmware starts
/// warning.
pub const LOW_BATTERY_PERCENT: u8 = 20;

/// What the battery screen needs to know, independent of UPower's object
/// model.
///
/// There is deliberately no path or object identity in here: a machine
/// has at most one battery this crate cares about (see
/// [`crate::backend::BatteryBackend::battery`] for how that one is
/// chosen), so nothing above the client needs to address it by anything
/// more than "the battery".
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BatteryInfo {
    /// 0..=100. UPower reports a `d`; this crate rounds it once, at the
    /// boundary, so nothing above that line has to decide how.
    pub percentage: u8,
    pub state: BatteryState,
    /// Estimated time until empty, while discharging. `None` when not
    /// discharging, or while UPower is still estimating (it reports `0`
    /// for a few seconds after a state change).
    pub time_to_empty: Option<Duration>,
    /// Estimated time until full, while charging. Same "still
    /// estimating" caveat as `time_to_empty`.
    pub time_to_full: Option<Duration>,
}

impl BatteryInfo {
    /// Low only while discharging: a battery at 15% that is plugged in
    /// and charging is on its way up, not a warning.
    pub fn is_low(&self) -> bool {
        self.state == BatteryState::Discharging && self.percentage <= LOW_BATTERY_PERCENT
    }

    /// Whichever of `time_to_empty` / `time_to_full` applies to the
    /// current state — a screen showing one number should not have to
    /// know which field means what.
    pub fn time_remaining(&self) -> Option<Duration> {
        match self.state {
            BatteryState::Discharging | BatteryState::PendingDischarge => self.time_to_empty,
            BatteryState::Charging | BatteryState::PendingCharge => self.time_to_full,
            _ => None,
        }
    }

    /// [`BatteryInfo::time_remaining`], in words a person reads —
    /// `"1h 32m"` or, under an hour, `"45m"`. `None` when there is
    /// nothing worth saying yet.
    pub fn time_remaining_words(&self) -> Option<String> {
        let remaining = self.time_remaining()?;
        if remaining.is_zero() {
            return None;
        }
        let minutes = remaining.as_secs() / 60;
        let (hours, minutes) = (minutes / 60, minutes % 60);
        Some(if hours > 0 {
            format!("{hours}h {minutes}m")
        } else {
            format!("{minutes}m")
        })
    }
}

/// What went wrong asking UPower about the battery, in terms a screen can
/// show without saying "check the logs" (vision pillar #3).
///
/// [`BatteryError::Unavailable`] is kept separate from "this machine has
/// no battery" on purpose: the absence of a battery is
/// [`Ok(None)`](crate::backend::BatteryBackend::battery) — a desktop with
/// only `AC` is a normal machine, not a UPower failure — and collapsing
/// the two would make a real "UPower isn't running" indistinguishable
/// from "this is a desktop".
#[derive(Debug, thiserror::Error)]
pub enum BatteryError {
    #[error(
        "UPower isn't answering, so battery status can't be read. It's normally started on \
         demand by D-Bus activation — try again, or restart the machine if it keeps failing."
    )]
    Unavailable,
    #[error("UPower didn't answer within {0:?}.")]
    TimedOut(Duration),
    #[error("{0}")]
    Refused(String),
}

/// One of `power-profiles-daemon`'s fixed three profiles.
///
/// Spelled out as a variant for the same reason [`What`] is: the daemon's
/// whole contract is exactly these three names
/// (`power-profiles-daemon(8)`), so a typo or a name the daemon has never
/// offered becomes a compile error here instead of a string that silently
/// fails to match anything when sent back.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PowerProfile {
    PowerSaver,
    Balanced,
    Performance,
}

impl PowerProfile {
    /// The exact string `power-profiles-daemon` uses, both in its
    /// `Profiles` property and as the value `ActiveProfile` is set to.
    pub fn as_str(self) -> &'static str {
        match self {
            PowerProfile::PowerSaver => "power-saver",
            PowerProfile::Balanced => "balanced",
            PowerProfile::Performance => "performance",
        }
    }

    /// The reverse of [`PowerProfile::as_str`]. `None` for a name
    /// `power-profiles-daemon` has not documented — a future profile this
    /// crate does not know what to do with, better reported than guessed
    /// at.
    pub fn parse(name: &str) -> Option<PowerProfile> {
        match name {
            "power-saver" => Some(PowerProfile::PowerSaver),
            "balanced" => Some(PowerProfile::Balanced),
            "performance" => Some(PowerProfile::Performance),
            _ => None,
        }
    }
}

impl fmt::Display for PowerProfile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// What went wrong asking `power-profiles-daemon` about the active
/// profile, in terms a screen can show without saying "check the logs"
/// (vision pillar #3).
///
/// [`ProfileError::Unavailable`] is kept separate from any particular
/// profile reading for the same reason every other `Unavailable` here is:
/// a daemon that is not running must never render as a specific,
/// plausible-looking answer.
#[derive(Debug, thiserror::Error)]
pub enum ProfileError {
    #[error(
        "power-profiles-daemon isn't answering, so the power profile can't be read or changed. \
         Start it with `systemctl start power-profiles-daemon`."
    )]
    Unavailable,
    #[error("power-profiles-daemon didn't answer within {0:?}.")]
    TimedOut(Duration),
    #[error(
        "power-profiles-daemon reported a profile named {0:?}, which this version of hyprforge \
         doesn't recognise."
    )]
    UnknownProfile(String),
    #[error("{0}")]
    Refused(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The whole point of `WhatSet`: there is no constructor that takes a
    /// string, so `"idel"` — the exact typo logind would silently accept
    /// and then ignore — cannot be built at all. This test exists to be
    /// broken by anyone who "helpfully" adds a `WhatSet::from_str`.
    #[test]
    fn a_misspelled_what_cannot_be_constructed() {
        let set = WhatSet::new([What::Sleep, What::Idle]);
        assert_eq!(set.to_arg(), "sleep:idle");
        // There is deliberately no `WhatSet::new(["idel"])` to try: the
        // only inputs the type accepts are `What` variants, which the
        // compiler already checked against logind's own vocabulary.
    }

    /// Pins the exact colon-separated form `Inhibit`'s `what` argument
    /// expects, in declaration order, so the string sent over the bus is
    /// never left to `HashMap`/`HashSet` iteration order.
    #[test]
    fn the_what_string_is_assembled_colon_separated_in_declaration_order() {
        let set = WhatSet::new([What::HandleLidSwitch, What::Sleep, What::Idle]);
        assert_eq!(set.to_arg(), "sleep:idle:handle-lid-switch");
    }

    #[test]
    fn a_single_value_has_no_trailing_or_leading_colon() {
        assert_eq!(WhatSet::new([What::Shutdown]).to_arg(), "shutdown");
    }

    #[test]
    fn duplicate_values_collapse_to_one() {
        let set = WhatSet::new([What::Sleep, What::Sleep, What::Idle]);
        assert_eq!(set.to_arg(), "sleep:idle");
    }

    #[test]
    fn keep_awake_blocks_sleep_and_idle_and_nothing_else() {
        let set = WhatSet::keep_awake();
        assert!(set.contains(What::Sleep));
        assert!(set.contains(What::Idle));
        assert!(!set.contains(What::Shutdown));
        assert!(!set.contains(What::HandleLidSwitch));
    }

    /// The distinction this crate exists to keep: logind not answering is
    /// not the same as nobody currently inhibiting anything.
    #[test]
    fn an_unreachable_logind_says_so_rather_than_reporting_nothing_inhibiting() {
        let message = InhibitError::Unavailable.to_string();
        assert!(message.contains("isn't answering"));
        assert!(
            !message.to_lowercase().contains("check the logs"),
            "an error with no way out of it is the dead end pillar 3 forbids"
        );
    }

    // --- BatteryState ------------------------------------------------

    #[test]
    fn every_documented_upower_state_value_maps_to_its_own_variant() {
        assert_eq!(BatteryState::from_upower(1), BatteryState::Charging);
        assert_eq!(BatteryState::from_upower(2), BatteryState::Discharging);
        assert_eq!(BatteryState::from_upower(3), BatteryState::Empty);
        assert_eq!(BatteryState::from_upower(4), BatteryState::FullyCharged);
        assert_eq!(BatteryState::from_upower(5), BatteryState::PendingCharge);
        assert_eq!(BatteryState::from_upower(6), BatteryState::PendingDischarge);
    }

    /// The distinction this mapping exists to keep: a value the daemon
    /// has never documented (or one we simply haven't seen yet) becomes
    /// `Unknown`, not a panic and not a guess dressed up as a real state.
    #[test]
    fn an_undocumented_upower_state_value_is_unknown_rather_than_a_panic_or_a_guess() {
        assert_eq!(BatteryState::from_upower(0), BatteryState::Unknown);
        assert_eq!(BatteryState::from_upower(99), BatteryState::Unknown);
    }

    // --- BatteryInfo ---------------------------------------------------

    fn battery(state: BatteryState, percentage: u8) -> BatteryInfo {
        BatteryInfo { percentage, state, time_to_empty: None, time_to_full: None }
    }

    #[test]
    fn a_discharging_battery_at_or_below_the_threshold_reads_as_low() {
        assert!(battery(BatteryState::Discharging, LOW_BATTERY_PERCENT).is_low());
        assert!(battery(BatteryState::Discharging, 5).is_low());
        assert!(!battery(BatteryState::Discharging, LOW_BATTERY_PERCENT + 1).is_low());
    }

    /// A battery on its way up must never read as a low-battery warning,
    /// even at a percentage that would be alarming while discharging.
    #[test]
    fn a_charging_battery_never_reads_as_low_no_matter_the_percentage() {
        assert!(!battery(BatteryState::Charging, 5).is_low());
        assert!(!battery(BatteryState::PendingCharge, 1).is_low());
        assert!(!battery(BatteryState::FullyCharged, 100).is_low());
    }

    #[test]
    fn time_remaining_picks_the_field_that_matches_the_current_state() {
        let mut info = battery(BatteryState::Discharging, 40);
        info.time_to_empty = Some(Duration::from_secs(3600));
        info.time_to_full = Some(Duration::from_secs(60));
        assert_eq!(info.time_remaining(), Some(Duration::from_secs(3600)));

        let mut info = battery(BatteryState::Charging, 40);
        info.time_to_empty = Some(Duration::from_secs(3600));
        info.time_to_full = Some(Duration::from_secs(60));
        assert_eq!(info.time_remaining(), Some(Duration::from_secs(60)));
    }

    /// Fully charged and sitting there is neither "counting down" nor
    /// "counting up" — nothing is remaining to say.
    #[test]
    fn a_fully_charged_battery_has_no_time_remaining_to_report() {
        let mut info = battery(BatteryState::FullyCharged, 100);
        info.time_to_empty = Some(Duration::from_secs(3600));
        info.time_to_full = Some(Duration::from_secs(60));
        assert_eq!(info.time_remaining(), None);
    }

    #[test]
    fn time_remaining_in_words_uses_hours_only_when_there_are_any() {
        let mut info = battery(BatteryState::Discharging, 40);
        info.time_to_empty = Some(Duration::from_secs(3600 + 32 * 60));
        assert_eq!(info.time_remaining_words().as_deref(), Some("1h 32m"));

        let mut info = battery(BatteryState::Discharging, 10);
        info.time_to_empty = Some(Duration::from_secs(45 * 60));
        assert_eq!(info.time_remaining_words().as_deref(), Some("45m"));
    }

    /// UPower reports exactly `0` while it is still estimating, right
    /// after a state change — that is "nothing to say yet", not "zero
    /// minutes left".
    #[test]
    fn a_zero_duration_is_treated_as_still_estimating_not_as_no_time_left() {
        let mut info = battery(BatteryState::Discharging, 40);
        info.time_to_empty = Some(Duration::ZERO);
        assert_eq!(info.time_remaining_words(), None);
    }

    /// The distinction this crate exists to keep, for the battery: a
    /// UPower that is not answering must never render the same as
    /// "nothing is low", "fully charged", or any other specific-looking
    /// answer.
    #[test]
    fn an_unreachable_upower_says_so_rather_than_reporting_a_specific_battery_state() {
        let message = BatteryError::Unavailable.to_string();
        assert!(message.contains("isn't answering"));
        assert!(!message.to_lowercase().contains("check the logs"));
    }

    // --- PowerProfile --------------------------------------------------

    #[test]
    fn each_profile_round_trips_through_its_wire_string() {
        for profile in [PowerProfile::PowerSaver, PowerProfile::Balanced, PowerProfile::Performance]
        {
            assert_eq!(PowerProfile::parse(profile.as_str()), Some(profile));
        }
    }

    #[test]
    fn the_wire_strings_are_exactly_what_power_profiles_daemon_documents() {
        assert_eq!(PowerProfile::PowerSaver.as_str(), "power-saver");
        assert_eq!(PowerProfile::Balanced.as_str(), "balanced");
        assert_eq!(PowerProfile::Performance.as_str(), "performance");
    }

    /// A name the daemon has never documented must not silently match one
    /// of the three known profiles.
    #[test]
    fn an_unrecognised_profile_name_parses_to_nothing() {
        assert_eq!(PowerProfile::parse("turbo"), None);
        assert_eq!(PowerProfile::parse(""), None);
        assert_eq!(PowerProfile::parse("Balanced"), None, "the wire form is lowercase");
    }

    /// The distinction this crate exists to keep, for profiles: a daemon
    /// that is not running must never render as a plausible-looking
    /// active profile.
    #[test]
    fn an_unreachable_power_profiles_daemon_says_so_rather_than_a_specific_profile() {
        let message = ProfileError::Unavailable.to_string();
        assert!(message.contains("isn't answering"));
        assert!(!message.to_lowercase().contains("check the logs"));
    }

    /// A profile name the daemon reports but this crate does not
    /// recognise is its own error, not silently coerced into one of the
    /// three known variants.
    #[test]
    fn an_unrecognised_profile_reported_by_the_daemon_names_the_profile_it_could_not_parse() {
        let message = ProfileError::UnknownProfile("turbo".to_string()).to_string();
        assert!(message.contains("turbo"));
    }
}
