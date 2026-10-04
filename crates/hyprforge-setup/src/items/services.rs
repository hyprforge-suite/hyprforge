//! The suite's background services, and replacing another notification
//! daemon with notifd.
//!
//! Done means `is-enabled` says enabled, however it got that way — a
//! per-user link or a package's global preset read the same. Undo only
//! reverses what setup recorded doing: a unit the packages enabled was
//! never enabled *by setup*, so undo leaves it alone rather than claim to
//! have turned it off. Turning off a globally enabled unit for one user
//! takes a mask, and that is its own explicit action
//! ([`crate::turn_off_for_me`]).

use super::{Applied, Cx};
use crate::record::{Change, Replaced};
use crate::state::State;
use crate::system::UnitState;

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ServiceSpec {
    pub unit: &'static str,
    pub requires: &'static [&'static str],
}

pub(crate) const DISPLAYD: ServiceSpec =
    ServiceSpec { unit: "hyprforge-displayd.service", requires: &["hyprforge-displayd"] };
pub(crate) const TRAYD: ServiceSpec =
    ServiceSpec { unit: "hyprforge-trayd.service", requires: &["hyprforge-trayd"] };
pub(crate) const CLIPD: ServiceSpec =
    ServiceSpec { unit: "hyprforge-clipd.service", requires: &["hyprforge-clipd"] };

/// notif's unit keeps its own name, as its binaries do: users' existing
/// `systemctl` lines and keybinds keep working.
pub(crate) const NOTIFD_UNIT: &str = "notifd.service";

/// Other notification daemons: `(name, unit, process)`. Only one program
/// can own `org.freedesktop.Notifications`, and notifd exits if another
/// already does.
const COMPETITORS: [(&str, &str, &str); 3] = [
    ("dunst", "dunst.service", "dunst"),
    ("mako", "mako.service", "mako"),
    ("swaync", "swaync.service", "swaync"),
];

/// Checks one unit: done if enabled, todo if it can be enabled.
fn unit_check(cx: &Cx<'_>, unit: &str) -> State {
    match cx.sys.unit_state(unit) {
        Err(e) => State::Unknown { why: format!("couldn't ask systemd about {unit}: {e}") },
        Ok(UnitState::Enabled) => State::Done,
        Ok(UnitState::Disabled) => State::Todo { what: format!("Enable and start {unit}") },
        Ok(UnitState::NotFound) => State::Unavailable { why: format!("{unit} isn't installed") },
        Ok(UnitState::Masked) => State::Unavailable {
            why: format!(
                "{unit} is masked — turned off for you on purpose. \
                 `systemctl --user unmask {unit}` undoes that"
            ),
        },
        Ok(UnitState::Other(word)) => State::Unavailable {
            why: format!("systemd reports {unit} as {word}, which setup doesn't enable"),
        },
    }
}

pub(super) fn check(cx: &Cx<'_>, spec: &ServiceSpec) -> State {
    unit_check(cx, spec.unit)
}

pub(super) fn apply(cx: &Cx<'_>, spec: &ServiceSpec) -> Result<Applied, String> {
    cx.sys.enable_now(spec.unit)?;
    Ok(Applied { change: Change::Service { unit: spec.unit.to_string() }, note: None })
}

/// Disables what setup enabled. A unit still enabled afterwards was also
/// enabled for every user by a package, and the note says so rather than
/// reporting it off.
pub(super) fn undo(cx: &Cx<'_>, unit: &str) -> Result<Option<String>, String> {
    cx.sys.disable_now(unit)?;
    Ok(still_enabled_note(cx, unit))
}

fn still_enabled_note(cx: &Cx<'_>, unit: &str) -> Option<String> {
    matches!(cx.sys.unit_state(unit), Ok(UnitState::Enabled)).then(|| {
        format!(
            "{unit} is still enabled for every user by its package; \
             \"Turn off for me\" masks it for you alone"
        )
    })
}

/// A competitor that is in the way: enabled or running as a unit, or
/// running some other way (an `exec-once`, which setup cannot stop for
/// good). `Err` when it could not be found out.
enum InTheWay {
    Unit(&'static str, &'static str),
    Process(&'static str),
}

fn competitors(cx: &Cx<'_>) -> Result<Vec<InTheWay>, String> {
    let mut found = Vec::new();
    for (name, unit, process) in COMPETITORS {
        let state = cx.sys.unit_state(unit)?;
        let active = cx.sys.unit_active(unit)?;
        if state == UnitState::Enabled || active {
            found.push(InTheWay::Unit(name, unit));
            continue;
        }
        match cx.sys.process_running(process) {
            Some(true) => found.push(InTheWay::Process(name)),
            Some(false) => {}
            None => return Err(format!("couldn't find out whether {name} is running")),
        }
    }
    Ok(found)
}

pub(super) fn check_notifd(cx: &Cx<'_>) -> State {
    let own = unit_check(cx, NOTIFD_UNIT);
    if matches!(own, State::Unavailable { .. } | State::Unknown { .. }) {
        return own;
    }
    let found = match competitors(cx) {
        Ok(found) => found,
        Err(e) => return State::Unknown { why: e },
    };
    // One started by the user's own config can't be switched off from
    // here for good: it comes back at the next login. Named, so the user
    // knows what to take out of their autostart.
    if let Some(InTheWay::Process(name)) =
        found.iter().find(|f| matches!(f, InTheWay::Process(_)))
    {
        return State::Unavailable {
            why: format!(
                "{name} is running, started by your own config rather than as a service — \
                 remove it from your autostart, then run setup again"
            ),
        };
    }
    let names: Vec<&str> = found
        .iter()
        .filter_map(|f| match f {
            InTheWay::Unit(name, _) => Some(*name),
            InTheWay::Process(_) => None,
        })
        .collect();
    match (own, names.is_empty()) {
        (State::Done, true) => State::Done,
        (_, false) => State::Todo {
            what: format!("Replace {}: turn it off and enable notifd", names.join(" and ")),
        },
        (_, true) => State::Todo { what: format!("Enable and start {NOTIFD_UNIT}") },
    }
}

pub(super) fn apply_notifd(cx: &Cx<'_>) -> Result<Applied, String> {
    let mut replaced = Vec::new();
    for found in competitors(cx)? {
        let InTheWay::Unit(_, unit) = found else { continue };
        cx.sys.disable_now(unit)?;
        // Still enabled after a disable means a package enabled it for
        // every user; only a mask stops it at the next login.
        let masked = if matches!(cx.sys.unit_state(unit), Ok(UnitState::Enabled)) {
            cx.sys.mask_now(unit)?;
            true
        } else {
            false
        };
        replaced.push(Replaced { unit: unit.to_string(), masked });
    }
    let enabled_notifd = !matches!(cx.sys.unit_state(NOTIFD_UNIT), Ok(UnitState::Enabled));
    if enabled_notifd {
        cx.sys.enable_now(NOTIFD_UNIT)?;
    } else {
        // Enabled but perhaps not running, because the competitor held the
        // bus name when it started. A restart, never a start: this only
        // acts on something already meant to run.
        let _ = cx.sys.try_restart(NOTIFD_UNIT);
    }
    Ok(Applied { change: Change::Notifd { enabled_notifd, replaced }, note: None })
}

/// Turns notifd off if setup turned it on, and brings back what it
/// replaced, the way it was turned off.
pub(super) fn undo_notifd(
    cx: &Cx<'_>,
    enabled_notifd: bool,
    replaced: &[Replaced],
) -> Result<Option<String>, String> {
    let mut note = None;
    if enabled_notifd {
        cx.sys.disable_now(NOTIFD_UNIT)?;
        note = still_enabled_note(cx, NOTIFD_UNIT);
    } else if !replaced.is_empty() {
        // Setup did not enable notifd, so undo does not turn it off —
        // but it will hold the bus name the returning daemon wants, and
        // that is worth saying rather than leaving a mystery.
        note = Some(format!(
            "{NOTIFD_UNIT} was enabled before setup ran and still is; only one \
             notification daemon can run, so turn one of them off"
        ));
    }
    for r in replaced {
        if r.masked {
            cx.sys.unmask(&r.unit)?;
        }
        // Unmasking starts nothing, and it was running before.
        cx.sys.enable_now(&r.unit)?;
    }
    Ok(note)
}
