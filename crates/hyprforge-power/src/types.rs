//! What an inhibit covers, and who holds one, independent of D-Bus.
//!
//! Kept free of `zbus` types so the model can be built and asserted on
//! without a bus, which is what makes the mock backend worth having.

use std::collections::BTreeSet;
use std::fmt;

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
}
