//! Record system history: hyprforge-procman's recorder, switched on.
//!
//! `hyprforge-procman-recorder.service` samples every process on the
//! machine every ten seconds into a journal only root can read, so the
//! process manager's Recorded tab and `hyprforge-top -r` can say what
//! was eating the machine at three in the morning. The package installs
//! the unit and enables nothing: a root service reading every process,
//! all day, is the user's decision, and this item is where they make it.
//!
//! A system unit, so enabling it is `pkexec systemctl enable --now` —
//! off by default, like every item that asks for a password. Checking
//! needs no password: `systemctl is-enabled` answers anyone.

use super::{Applied, Cx};
use crate::record::Change;
use crate::state::State;
use crate::system::UnitState;

pub const UNIT: &str = "hyprforge-procman-recorder.service";

pub(super) fn check(cx: &Cx<'_>) -> State {
    match cx.sys.system_unit_state(UNIT) {
        Err(e) => State::Unknown { why: e },
        Ok(UnitState::Enabled) => State::Done,
        Ok(UnitState::Disabled) => State::Todo {
            what: "Record every process every ten seconds into a history only root can read (asks for your password)".into(),
        },
        Ok(UnitState::NotFound) => State::Unavailable { why: format!("{UNIT} isn't installed") },
        // Masked is an administrator's "never"; not this item's to undo.
        Ok(UnitState::Masked) => State::Unavailable { why: format!("{UNIT} is masked") },
        Ok(UnitState::Other(word)) => State::Unknown { why: format!("{UNIT} is {word}") },
    }
}

pub(super) fn apply(cx: &Cx<'_>) -> Result<Applied, String> {
    cx.sys.system_enable_now(UNIT, true)?;
    Ok(Applied { change: Change::SystemService { unit: UNIT.into() }, note: None })
}

/// Stops recording. The history already written stays, for the recorder's
/// own retention to age out — deleting it is not "putting things back",
/// it is losing what the user may still want to look at.
pub(super) fn undo(cx: &Cx<'_>, unit: &str) -> Result<Option<String>, String> {
    cx.sys.system_enable_now(unit, false)?;
    Ok(Some("The history recorded so far is kept; it ages out by the recorder's own limits.".into()))
}
