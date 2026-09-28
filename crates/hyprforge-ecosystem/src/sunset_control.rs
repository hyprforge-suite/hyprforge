//! Runtime control of hyprsunset — turning night light on and off while
//! the session runs, as opposed to [`crate::sunset`], which only renders
//! the schedule hyprsunset reads at startup.
//!
//! # Mechanism: `hyprctl hyprsunset ...`, not the raw socket
//!
//! hyprsunset exposes a Unix socket at
//! `$XDG_RUNTIME_DIR/hypr/<instance>/.hyprsunset.sock`, but `hyprctl`
//! already speaks its tiny text protocol and is the interface the wiki
//! documents — `hyprctl hyprsunset temperature <k>` is exactly what
//! [`crate::apply::temperature`] uses today to push a schedule live.
//! Reimplementing the wire format here would be a second, unreviewed
//! parser for a protocol this project already has one working client
//! for, so this module drives the same command instead of the socket
//! directly.
//!
//! Every call goes through [`hyprforge_core::command::output`], never
//! `Command::output()` — `hyprctl` can hang exactly like any other
//! external program this project talks to.
//!
//! # What can and cannot be read back
//!
//! `hyprctl hyprsunset temperature` with no argument reports the last
//! temperature hyprsunset was told to hold — confirmed live on this
//! machine, and exactly what [`Hyprsunset::current_temperature`] reads.
//! But hyprsunset has **no request that reports whether identity mode is
//! active**: asking for the temperature after `hyprctl hyprsunset
//! identity` still returns the last numeric temperature, unchanged,
//! because identity does not touch it. There is no `hyprctl hyprsunset
//! status` or equivalent — every subcommand not in `--help` answers
//! `invalid command`.
//!
//! So this crate can tell you *what temperature hyprsunset would show if
//! it weren't in identity mode*, but not *whether it currently is*. A
//! caller that needs to display an on/off toggle has to track the mode it
//! last asked for itself; reading it back from hyprsunset is not an
//! option this daemon offers. That gap is deliberate to name rather than
//! paper over with a guess.
//!
//! # Two different "off"s
//!
//! [`SunsetControlError::NotRunning`] — hyprsunset isn't there to ask —
//! is a distinct outcome from turning night light off, which is an
//! ordinary success ([`SunsetBackend::turn_off`] returning `Ok(())`).
//! Collapsing "nothing answered" into "night light is off" is exactly
//! the mistake CLAUDE.md's rule about `hl.config`'s missing-vs-unparseable
//! files describes, one layer further out: a stopped daemon must not be
//! reported as a night light setting.

use hyprforge_core::command;
use std::process::Command;
use std::time::Duration;

pub use crate::sunset::{MAX_TEMPERATURE, MIN_TEMPERATURE};

/// A warm default for the common case: someone flips night light on
/// without picking a number. Cooler than a candle (~1900K), warmer than
/// the neutral 6000K [`crate::sunset::Profile::default`] schedules pick,
/// which is deliberate — a schedule's daytime profile has no reason to be
/// warm, but a manual "turn it on" toggle is reached for specifically to
/// warm the screen.
pub const DEFAULT_WARM_TEMPERATURE: i64 = 4500;

/// How long a single `hyprctl hyprsunset ...` call is given.
///
/// `hyprctl` round-trips through Hyprland's own socket to reach
/// hyprsunset's, but both are local and answer near-instantly when
/// anything is listening — [`hyprforge_core::command::TIMEOUT`] is
/// already generous for that.
pub const TIMEOUT: Duration = command::TIMEOUT;

/// Night light, as something to set. Not a status: see the module doc
/// for why hyprsunset cannot report which of these currently holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NightLight {
    /// Identity matrix — no colour change at all.
    Off,
    /// A colour temperature in Kelvin, checked against
    /// [`MIN_TEMPERATURE`]..=[`MAX_TEMPERATURE`] before it is ever sent.
    On(i64),
}

impl Default for NightLight {
    fn default() -> Self {
        NightLight::On(DEFAULT_WARM_TEMPERATURE)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum SunsetControlError {
    /// Confirmed absent — `pgrep -x hyprsunset` found nothing. Distinct
    /// from [`SunsetControlError::CouldNotCheck`] for the same reason
    /// `apply::Applied::DaemonNotRunning` and `::DaemonUnknown` are kept
    /// apart: "checked, it isn't there" and "the check itself failed" are
    /// different facts, and folding the second into the first tells
    /// someone their daemon is stopped when it might be sitting right
    /// there, merely unreachable for a moment.
    #[error(
        "hyprsunset isn't running, so night light can't be changed. It normally starts from \
         `exec-once` in your Hyprland config."
    )]
    NotRunning,
    /// `pgrep` itself didn't start or didn't answer in time — not
    /// evidence of anything about hyprsunset.
    #[error("couldn't tell whether hyprsunset is running: {0}")]
    CouldNotCheck(String),
    #[error("temperature must be between {min} and {max}K (got {value})")]
    OutOfRange { min: i64, max: i64, value: i64 },
    #[error("hyprsunset didn't answer within {0:?}")]
    TimedOut(Duration),
    /// hyprsunset ran and answered, but refused the request or the
    /// answer wasn't the shape expected.
    #[error("hyprsunset refused this request: {0}")]
    Refused(String),
}

/// Rejects a temperature outside what hyprsunset accepts, before
/// anything is sent to it. Confirmed live on this machine: hyprsunset
/// itself reports `Invalid temperature (should be an integer in range
/// 1000-20000)` for exactly this range, so checking it here rather than
/// forwarding it and hoping is not a guess about the daemon's behaviour.
pub fn validate_temperature(kelvin: i64) -> Result<(), SunsetControlError> {
    if (MIN_TEMPERATURE..=MAX_TEMPERATURE).contains(&kelvin) {
        Ok(())
    } else {
        Err(SunsetControlError::OutOfRange {
            min: MIN_TEMPERATURE,
            max: MAX_TEMPERATURE,
            value: kelvin,
        })
    }
}

/// The `hyprctl` argument vector to read the temperature hyprsunset is
/// currently holding. Split out as a pure function so the exact command
/// line is assertable without running anything.
pub fn read_temperature_command() -> Vec<&'static str> {
    vec!["hyprsunset", "temperature"]
}

/// The `hyprctl` argument vector to set a temperature. Does not validate
/// — callers go through [`validate_temperature`] first, so an
/// out-of-range value never reaches this function with the intent of
/// being sent.
pub fn set_temperature_command(kelvin: i64) -> Vec<String> {
    vec!["hyprsunset".to_string(), "temperature".to_string(), kelvin.to_string()]
}

/// The `hyprctl` argument vector to turn night light off. This is
/// `identity`, never `temperature 6000` — the two are not the same
/// request, and hyprsunset's own `--temperature` default of 6000K is
/// still a colour transform (a very mild one), not "no colour change".
/// Sending `temperature 6000` where `identity` was meant would set a
/// value that happens to look neutral instead of asking for the matrix
/// that guarantees it.
pub fn turn_off_command() -> Vec<&'static str> {
    vec!["hyprsunset", "identity"]
}

/// Whatever can drive night light at runtime — the real hyprsunset
/// process, or `mock::MockBackend` (behind the `mock` feature) in a test.
pub trait SunsetBackend {
    /// The temperature hyprsunset is currently set to hold. See the
    /// module doc: this is **not** whether night light is on or off,
    /// because hyprsunset has nothing that reports that.
    fn current_temperature(&self) -> Result<i64, SunsetControlError>;

    /// Sets a colour temperature. Returns
    /// [`SunsetControlError::OutOfRange`] without touching hyprsunset at
    /// all when `kelvin` is outside the accepted range.
    fn set_temperature(&self, kelvin: i64) -> Result<(), SunsetControlError>;

    /// Turns night light off (identity matrix). Success here — `Ok(())`
    /// — is night light being off. It is never the same outcome as
    /// [`SunsetControlError::NotRunning`], which means the request never
    /// reached anything at all.
    fn turn_off(&self) -> Result<(), SunsetControlError>;
}

/// Applies a [`NightLight`] choice to any backend, dispatching to
/// [`SunsetBackend::set_temperature`] or [`SunsetBackend::turn_off`].
pub fn apply(backend: &dyn SunsetBackend, state: NightLight) -> Result<(), SunsetControlError> {
    match state {
        NightLight::Off => backend.turn_off(),
        NightLight::On(kelvin) => backend.set_temperature(kelvin),
    }
}

/// The real backend: the running hyprsunset, driven through `hyprctl`.
pub struct Hyprsunset;

impl Hyprsunset {
    /// `Some(false)` for "checked, it isn't running", `None` — reported
    /// as [`SunsetControlError::CouldNotCheck`] by callers — for "the
    /// check itself failed". Same shape as `apply::running` in this
    /// crate, and for the same reason: collapsing the two would read a
    /// `pgrep` that failed to start as "hyprsunset is not running".
    fn running() -> Option<bool> {
        command::output(Command::new("pgrep").args(["-x", "hyprsunset"]), TIMEOUT)
            .map(|o| o.status.success())
            .ok()
    }

    /// Requires hyprsunset to be confirmed running before issuing `args`,
    /// so a request is never sent to (and never silently swallowed by)
    /// a daemon that isn't there.
    fn hyprctl(args: &[&str]) -> Result<String, SunsetControlError> {
        match Self::running() {
            Some(true) => {}
            Some(false) => return Err(SunsetControlError::NotRunning),
            None => {
                return Err(SunsetControlError::CouldNotCheck(
                    "pgrep didn't start or didn't answer in time".to_string(),
                ))
            }
        }

        let output = command::output(Command::new("hyprctl").args(args), TIMEOUT).map_err(|e| {
            if e.kind() == std::io::ErrorKind::TimedOut {
                SunsetControlError::TimedOut(TIMEOUT)
            } else {
                SunsetControlError::Refused(e.to_string())
            }
        })?;

        let body = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let lower = body.to_lowercase();
        if !output.status.success() || lower.contains("invalid") || lower.contains("error") {
            return Err(SunsetControlError::Refused(if body.is_empty() {
                "hyprctl reported failure with no message".to_string()
            } else {
                body
            }));
        }
        Ok(body)
    }
}

impl SunsetBackend for Hyprsunset {
    fn current_temperature(&self) -> Result<i64, SunsetControlError> {
        let body = Self::hyprctl(&read_temperature_command())?;
        body.parse::<i64>()
            .map_err(|_| SunsetControlError::Refused(format!("not a temperature: {body}")))
    }

    fn set_temperature(&self, kelvin: i64) -> Result<(), SunsetControlError> {
        validate_temperature(kelvin)?;
        let args = set_temperature_command(kelvin);
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        Self::hyprctl(&args).map(|_| ())
    }

    fn turn_off(&self) -> Result<(), SunsetControlError> {
        Self::hyprctl(&turn_off_command()).map(|_| ())
    }
}

// The settings/tray screens that will drive night light need to test
// against something other than a real hyprsunset, the same way the
// Network screen tests against `hyprforge_network::backend::mock`. That
// means this mock has to compile as ordinary (non-test) code under the
// `mock` feature, not just under `#[cfg(test)]` inside this crate.
#[cfg(any(test, feature = "mock"))]
pub mod mock {
    use super::*;
    use std::sync::Mutex;

    /// A hyprsunset that does what the test says.
    pub struct MockBackend {
        /// `None` models hyprsunset not being reachable at all —
        /// distinct from "off", which this crate cannot even observe
        /// (see the module doc). Defaults to running with hyprsunset's
        /// own documented default.
        pub temperature: Mutex<Option<i64>>,
    }

    impl Default for MockBackend {
        fn default() -> Self {
            Self { temperature: Mutex::new(Some(6000)) }
        }
    }

    impl MockBackend {
        pub fn new() -> Self {
            Self::default()
        }

        /// A backend standing in for hyprsunset not running at all.
        pub fn not_running() -> Self {
            Self { temperature: Mutex::new(None) }
        }
    }

    impl SunsetBackend for MockBackend {
        fn current_temperature(&self) -> Result<i64, SunsetControlError> {
            self.temperature.lock().unwrap().ok_or(SunsetControlError::NotRunning)
        }

        fn set_temperature(&self, kelvin: i64) -> Result<(), SunsetControlError> {
            validate_temperature(kelvin)?;
            let mut current = self.temperature.lock().unwrap();
            if current.is_none() {
                return Err(SunsetControlError::NotRunning);
            }
            *current = Some(kelvin);
            Ok(())
        }

        fn turn_off(&self) -> Result<(), SunsetControlError> {
            // Real hyprsunset leaves the reported temperature untouched
            // when identity is toggled on — see the module doc — so the
            // mock mirrors that rather than clearing the value, which
            // would claim a readback this crate cannot actually get.
            if self.temperature.lock().unwrap().is_none() {
                return Err(SunsetControlError::NotRunning);
            }
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::mock::MockBackend;
    use super::*;

    #[test]
    fn an_out_of_range_temperature_is_refused_before_the_daemon_is_invoked() {
        // A backend with nothing running: if this ever contacted the
        // daemon it would fail with `NotRunning`, not `OutOfRange`. Only
        // `OutOfRange` coming back proves validation ran first.
        let backend = MockBackend::not_running();
        let err = backend.set_temperature(500).unwrap_err();
        assert!(matches!(err, SunsetControlError::OutOfRange { .. }), "{err:?}");

        let err = backend.set_temperature(999_999).unwrap_err();
        assert!(matches!(err, SunsetControlError::OutOfRange { .. }), "{err:?}");
    }

    #[test]
    fn the_extremes_of_the_declared_range_are_accepted() {
        assert!(validate_temperature(MIN_TEMPERATURE).is_ok());
        assert!(validate_temperature(MAX_TEMPERATURE).is_ok());
        assert!(validate_temperature(MIN_TEMPERATURE - 1).is_err());
        assert!(validate_temperature(MAX_TEMPERATURE + 1).is_err());
    }

    /// The distinction CLAUDE.md's missing-vs-unparseable rule is about,
    /// one layer out: a daemon that isn't there must never be reported as
    /// a night light setting. Turning night light off is an ordinary
    /// success; a daemon that cannot be reached is an error, and the two
    /// are never the same `Result`.
    #[test]
    fn hyprsunset_not_running_is_a_distinct_outcome_from_night_light_being_off() {
        let running = MockBackend::new();
        assert!(running.turn_off().is_ok(), "turning night light off is a success, not an error");

        let absent = MockBackend::not_running();
        assert!(matches!(absent.turn_off(), Err(SunsetControlError::NotRunning)));
        assert!(matches!(absent.current_temperature(), Err(SunsetControlError::NotRunning)));
        assert!(matches!(absent.set_temperature(4500), Err(SunsetControlError::NotRunning)));
    }

    #[test]
    fn the_command_line_for_a_temperature_is_exactly_hyprsunset_temperature_and_the_value() {
        assert_eq!(set_temperature_command(4500), vec!["hyprsunset", "temperature", "4500"]);
        assert_eq!(set_temperature_command(MIN_TEMPERATURE), vec![
            "hyprsunset",
            "temperature",
            &MIN_TEMPERATURE.to_string()
        ]);
    }

    #[test]
    fn the_command_line_to_read_the_temperature_carries_no_value() {
        assert_eq!(read_temperature_command(), vec!["hyprsunset", "temperature"]);
    }

    /// `identity`, never `temperature 6000` — hyprsunset's own default
    /// temperature is still a (mild) colour transform, not the identity
    /// matrix, so the two commands are not interchangeable and this
    /// pins which one "off" actually sends.
    #[test]
    fn turning_off_sends_identity_not_a_6000k_temperature() {
        let off = turn_off_command();
        assert_eq!(off, vec!["hyprsunset", "identity"]);
        assert!(!off.contains(&"temperature"), "{off:?}");
        assert!(!off.contains(&"6000"), "{off:?}");
    }

    #[test]
    fn a_mock_reports_the_temperature_it_was_set_to() {
        let backend = MockBackend::new();
        backend.set_temperature(3800).unwrap();
        assert_eq!(backend.current_temperature().unwrap(), 3800);
    }

    /// The gap named in the module doc: reading back after `turn_off`
    /// still reports the last temperature, exactly like the real
    /// hyprsunset does — because nothing on this interface can report
    /// "identity is currently active".
    #[test]
    fn reading_the_temperature_after_turning_off_does_not_reveal_the_off_state() {
        let backend = MockBackend::new();
        backend.set_temperature(3800).unwrap();
        backend.turn_off().unwrap();
        assert_eq!(
            backend.current_temperature().unwrap(),
            3800,
            "a temperature read cannot show that night light is off"
        );
    }

    #[test]
    fn applying_night_light_off_calls_turn_off_and_on_calls_set_temperature() {
        let backend = MockBackend::new();
        apply(&backend, NightLight::On(3000)).unwrap();
        assert_eq!(backend.current_temperature().unwrap(), 3000);

        // Off is representable and does not error, even though (per the
        // gap above) reading afterwards still shows 3000.
        apply(&backend, NightLight::Off).unwrap();
    }

    #[test]
    fn the_default_night_light_is_on_and_warm_rather_than_neutral() {
        match NightLight::default() {
            NightLight::On(k) => {
                assert!(k < 6000, "a warm default should read below neutral daylight");
                assert!(validate_temperature(k).is_ok());
            }
            NightLight::Off => panic!("the default should be a usable warm setting, not off"),
        }
    }
}
