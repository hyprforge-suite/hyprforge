//! hyprforge-polkit as this session's polkit agent, replacing whichever
//! agent had the job.
//!
//! polkit accepts one authentication agent per session: the second to
//! register is refused. So enabling ours is not enough while another is
//! running — and on Hyprland the usual one, hyprpolkitagent, is started
//! by a line in the user's own config (`systemctl --user start
//! hyprpolkitagent`, which Hyprland's wiki suggests), not by an enable
//! link. Disabling it would change nothing: the line starts it again at
//! the next login, before or after ours, and whichever registers first
//! answers.
//!
//! A per-user **mask** is the switch that holds. `systemctl --user start`
//! refuses a masked unit, so that config line fails harmlessly and the
//! config is never edited; the enable links, if any, are left exactly as
//! they were; and `unmask` is a complete undo. Undo also starts the agent
//! again if it was running, so the session is not left with no agent at
//! all until the next login.
//!
//! An agent started some other way — a bare `exec` of
//! `polkit-gnome-authentication-agent-1`, say — cannot be switched off
//! from here for good, and is named instead, as notifd's competitors are
//! (`items/services.rs`).

use super::{Applied, Cx};
use crate::record::{Change, ReplacedAgent};
use crate::state::State;
use crate::system::UnitState;

pub(crate) const UNIT: &str = "hyprforge-polkit.service";

/// Agents that come as a user unit: `(name, unit)`.
const UNITS: [(&str, &str); 2] =
    [("hyprpolkitagent", "hyprpolkitagent.service"), ("polkit-kde-agent", "plasma-polkit-agent.service")];

/// Agents that do not, by the name `/proc/<pid>/comm` gives them — the
/// first fifteen characters of the executable's name, so
/// `polkit-gnome-authentication-agent-1` is `polkit-gnome-au`. `pgrep -x`
/// matches that truncated name, and only that.
const PROCESSES: [(&str, &str); 4] = [
    ("polkit-gnome", "polkit-gnome-au"),
    ("polkit-mate", "polkit-mate-aut"),
    ("lxqt-policykit", "lxqt-policykit-"),
    ("polkit-kde-agent", "polkit-kde-auth"),
];

/// The units in the way: running now, or enabled to run at login.
fn units_in_the_way(cx: &Cx<'_>) -> Result<Vec<(&'static str, &'static str, bool)>, String> {
    let mut found = Vec::new();
    for (name, unit) in UNITS {
        let state = cx.sys.unit_state(unit)?;
        let active = cx.sys.unit_active(unit)?;
        if state == UnitState::Enabled || active {
            found.push((name, unit, active));
        }
    }
    Ok(found)
}

/// An agent running outside systemd's reach, by name. Its unit's own
/// process is not one: that is the unit's to stop.
fn running_by_hand(cx: &Cx<'_>, units: &[(&str, &str, bool)]) -> Result<Option<&'static str>, String> {
    for (name, process) in PROCESSES {
        if units.iter().any(|(unit_name, _, active)| *unit_name == name && *active) {
            continue;
        }
        match cx.sys.process_running(process) {
            Some(true) => return Ok(Some(name)),
            Some(false) => {}
            None => return Err(format!("couldn't find out whether {name} is running")),
        }
    }
    Ok(None)
}

pub(super) fn check(cx: &Cx<'_>) -> State {
    let own = match cx.sys.unit_state(UNIT) {
        Err(e) => return State::Unknown { why: format!("couldn't ask systemd about {UNIT}: {e}") },
        Ok(UnitState::NotFound) => return State::Unavailable { why: format!("{UNIT} isn't installed") },
        Ok(UnitState::Masked) => {
            return State::Unavailable {
                why: format!(
                    "{UNIT} is masked — turned off for you on purpose. \
                     `systemctl --user unmask {UNIT}` undoes that"
                ),
            }
        }
        Ok(UnitState::Other(word)) => {
            return State::Unavailable { why: format!("systemd reports {UNIT} as {word}, which setup doesn't enable") }
        }
        Ok(state) => state,
    };
    let units = match units_in_the_way(cx) {
        Ok(units) => units,
        Err(e) => return State::Unknown { why: e },
    };
    match running_by_hand(cx, &units) {
        Err(e) => return State::Unknown { why: e },
        Ok(Some(name)) => {
            return State::Unavailable {
                why: format!(
                    "{name} is running, started by your own config rather than as a service — \
                     only one agent can answer polkit, so remove it from your autostart, then run setup again"
                ),
            }
        }
        Ok(None) => {}
    }
    let names: Vec<&str> = units.iter().map(|(name, _, _)| *name).collect();
    match (own, names.is_empty()) {
        (UnitState::Enabled, true) => State::Done,
        (_, false) => State::Todo {
            what: format!("Replace {}: mask it and enable hyprforge-polkit", names.join(" and ")),
        },
        (_, true) => State::Todo { what: format!("Enable and start {UNIT}") },
    }
}

pub(super) fn apply(cx: &Cx<'_>) -> Result<Applied, String> {
    let mut replaced = Vec::new();
    // The other agent goes first: polkit refuses ours while it holds the
    // session. `mask --now` stops it too.
    for (_, unit, active) in units_in_the_way(cx)? {
        cx.sys.mask_now(unit)?;
        replaced.push(ReplacedAgent { unit: unit.to_string(), was_active: active });
    }
    let enabled_ours = !matches!(cx.sys.unit_state(UNIT), Ok(UnitState::Enabled));
    // `enable --now` on an enabled unit starts it, which is wanted: it
    // may have given up at login because the other agent was there first.
    cx.sys.enable_now(UNIT)?;
    Ok(Applied { change: Change::PolkitAgent { enabled_ours, replaced }, note: None })
}

/// Turns ours off if setup turned it on, and brings back the agent it
/// replaced: unmasked, and running again if it was running.
pub(super) fn undo(cx: &Cx<'_>, enabled_ours: bool, replaced: &[ReplacedAgent]) -> Result<Option<String>, String> {
    if enabled_ours {
        cx.sys.disable_now(UNIT)?;
    } else {
        // Not setup's to disable, but it must stop for the other agent to
        // register — it registers only at start.
        cx.sys.stop(UNIT)?;
    }
    for r in replaced {
        cx.sys.unmask(&r.unit)?;
        if r.was_active {
            cx.sys.start(&r.unit)?;
        }
    }
    let note = (!enabled_ours && !replaced.is_empty()).then(|| {
        format!("{UNIT} was enabled before setup ran and still is; it will compete with the restored agent at the next login")
    });
    Ok(note)
}
