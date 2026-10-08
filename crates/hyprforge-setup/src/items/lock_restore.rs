//! `misc:allow_session_lock_restore`, so a crashed lock screen can be
//! replaced rather than leaving the session stuck.
//!
//! Hyprland keeps a session locked when its lock client dies — correctly,
//! since the alternative is a crash that unlocks — and by default it also
//! refuses any *new* client the lock, so nothing can take over. The
//! session sits behind Hyprland's "lockscreen app died" screen until
//! someone runs `hl.clear_crashed_lockscreen()` from another VT.
//! hyprforge-lock relaunches itself after a crash (its `supervise`
//! module), and this option is what lets the relaunch be granted.
//!
//! Through `system.toml` and the generated `system.lua`, the files the
//! System page owns — the same key that page shows as "Allow restarting
//! the lock screen", so the page reflects what setup did and can undo it
//! by hand. Verified against Hyprland 0.56.2: the option exists and
//! defaults to off.
//!
//! What it allows, said plainly: while a lock is dead, any client of this
//! user can take it over. A process running as the user can already
//! clear a crashed lock with `hyprctl`, so this moves no trust boundary;
//! someone at the keyboard who crashes the lock has no shell to start a
//! client from.

use super::{Applied, Cx};
use crate::record::Change;
use crate::state::State;
use crate::system::Generated;
use hyprforge_system::{Settings, Value};

pub(crate) const KEY: &str = "misc:allow_session_lock_restore";

fn load(cx: &Cx<'_>) -> Result<Settings, String> {
    hyprforge_system::storage::load(&cx.env.system_toml()).map_err(|e| e.to_string())
}

pub(super) fn check(cx: &Cx<'_>) -> State {
    match load(cx) {
        Err(e) => State::Unknown { why: e },
        Ok(settings) if settings.get(KEY) == Some(&Value::Bool(true)) => State::Done,
        Ok(_) => State::Todo { what: "Let a restarted lock screen take over from one that crashed".into() },
    }
}

fn generated(cx: &Cx<'_>, settings: &Settings) -> Generated {
    Generated {
        path: cx.env.system_lua(),
        contents: hyprforge_system::apply::generate(settings),
        empty: hyprforge_system::apply::generate(&Settings::default()),
        subject: "system settings",
    }
}

/// Saves and loads `settings`, putting the old TOML back if Hyprland
/// refuses (it has already had its old Lua put back).
fn save_and_load(cx: &Cx<'_>, previous: &Settings, settings: &Settings) -> Result<Option<String>, String> {
    let path = cx.env.system_toml();
    hyprforge_system::storage::save(&path, settings).map_err(|e| e.to_string())?;
    match cx.sys.apply_lua(&generated(cx, settings)) {
        Ok(loaded) => Ok(super::loaded_note(loaded)),
        Err(e) => {
            if let Err(restore) = hyprforge_system::storage::save(&path, previous) {
                return Err(format!("{e} — and putting system.toml back failed too: {restore}"));
            }
            Err(e)
        }
    }
}

pub(super) fn apply(cx: &Cx<'_>) -> Result<Applied, String> {
    let before = load(cx)?;
    let previous = before.get(KEY).cloned();
    let mut settings = before.clone();
    settings.set(KEY, Value::Bool(true));
    let note = save_and_load(cx, &before, &settings)?;
    Ok(Applied { change: Change::LockRestore { previous }, note })
}

/// Puts back what `system.toml` held — unless the user has changed the
/// key since, in which case their choice stands and is left alone.
pub(super) fn undo(cx: &Cx<'_>, previous: Option<&Value>) -> Result<Option<String>, String> {
    let before = load(cx)?;
    if before.get(KEY) != Some(&Value::Bool(true)) {
        return Ok(Some("it has been changed since, so it was left as it is".into()));
    }
    let mut settings = before.clone();
    match previous {
        Some(value) => settings.set(KEY, value.clone()),
        None => settings.clear(KEY),
    }
    if settings == before {
        return Ok(None);
    }
    save_and_load(cx, &before, &settings)
}
