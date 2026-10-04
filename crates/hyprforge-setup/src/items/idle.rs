//! hypridle's `lock_cmd`, pointed at Hyprforge's lock screen.
//!
//! Through `idle.toml` and the generated `idle.conf`, the files the Idle &
//! lock page owns — so that page shows the new command and can change it.
//! `idle.conf` is sourced at the *end* of `hypridle.conf`, so its
//! `general { lock_cmd }` is assigned after the user's own block and is
//! the one that stands; hypridle reading that pair without complaint is
//! checked against the real daemon in hyprforge-ecosystem's parse tier.
//!
//! hypridle has no IPC, so a change needs a restart. Setup restarts it
//! only if it was already running (`pkill -x` and one detached start —
//! `hyprforge_ecosystem::apply::restart_idle`), and never starts one that
//! was not: a second hypridle is two daemons locking the screen.

use super::{Applied, Cx};
use crate::record::Change;
use crate::state::State;
use hyprforge_core::hyprlang;
use hyprforge_ecosystem::idle;

/// `pidof` first, so a lock request while already locked is a no-op
/// rather than a second lock client fighting the first for the session
/// lock — the idiom hypridle's own documentation uses for hyprlock.
pub const LOCK_CMD: &str = "pidof hyprforge-lock || hyprforge-lock";

fn load(cx: &Cx<'_>) -> Result<idle::Settings, String> {
    hyprforge_ecosystem::storage::load(&cx.env.idle_toml()).map_err(|e| e.to_string())
}

/// `hypridle.conf`, or empty if there is none yet.
fn read_hypridle(cx: &Cx<'_>) -> Result<String, String> {
    let path = cx.env.hypridle_conf();
    match std::fs::read_to_string(&path) {
        Ok(text) => Ok(text),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(e) => Err(format!("couldn't read {}: {e}", path.display())),
    }
}

/// The `lock_cmd` the user's own `hypridle.conf` sets, if any — what the
/// session runs today when nothing of Hyprforge's overrides it.
fn users_own(hypridle: &str) -> Option<String> {
    let doc = hyprlang::parse(hypridle);
    // The last assignment in the last block that sets it: hyprlang
    // assigns a repeated key again, so that is the one in effect.
    let found = doc
        .blocks("general")
        .filter_map(|b| match b {
            hyprlang::Item::Block { items, .. } => items.iter().rev().find_map(|i| match i {
                hyprlang::Item::Assignment { key, value, .. } if key == "lock_cmd" => {
                    Some(value.as_str())
                }
                _ => None,
            }),
            hyprlang::Item::Assignment { .. } => None,
        })
        .last();
    found.map(|v| v.trim().to_string()).filter(|v| !v.is_empty())
}

pub(super) fn check(cx: &Cx<'_>) -> State {
    let settings = match load(cx) {
        Ok(s) => s,
        Err(e) => return State::Unknown { why: e },
    };
    let hypridle = match read_hypridle(cx) {
        Ok(text) => text,
        Err(e) => return State::Unknown { why: e },
    };
    let sourced = hyprlang::detect(&hypridle, &hyprlang::source_line(&cx.env.idle_conf()))
        == hyprlang::SourcePlan::AlreadyPresent;
    let ours = settings.general.lock_cmd.trim();
    if ours == LOCK_CMD && sourced && cx.env.idle_conf().exists() {
        return State::Done;
    }
    let current = if ours.is_empty() { users_own(&hypridle) } else { Some(ours.to_string()) };
    State::Todo {
        what: match current {
            Some(other) if other != LOCK_CMD => {
                format!("Run hyprforge-lock instead of \"{other}\" when the session locks")
            }
            _ => "Run hyprforge-lock when the session locks".to_string(),
        },
    }
}

/// Writes `settings` and restarts hypridle if, and only if, it is running.
fn write(cx: &Cx<'_>, settings: &idle::Settings) -> Result<Option<String>, String> {
    hyprforge_ecosystem::storage::save(&cx.env.idle_toml(), settings).map_err(|e| e.to_string())?;
    hyprforge_ecosystem::apply::write_idle(&cx.env.idle_conf(), &cx.env.hypridle_conf(), settings)
        .map_err(|e| e.to_string())?;
    Ok(match cx.sys.process_running("hypridle") {
        Some(true) => match cx.sys.restart_idle() {
            Ok(()) => None,
            Err(e) => Some(format!("saved, but hypridle didn't restart ({e}) — restart it to apply")),
        },
        Some(false) => None,
        None => Some("saved; couldn't tell whether hypridle is running, so it wasn't restarted".into()),
    })
}

pub(super) fn apply(cx: &Cx<'_>) -> Result<Applied, String> {
    let mut settings = load(cx)?;
    let previous = settings.general.lock_cmd.clone();
    settings.general.set_command("lock_cmd", LOCK_CMD.to_string());
    let note = write(cx, &settings)?;
    Ok(Applied { change: Change::IdleLock { previous }, note })
}

/// Puts the previous `lock_cmd` back — unless the user has set a
/// different one since, which is theirs and stays.
pub(super) fn undo(cx: &Cx<'_>, previous: &str) -> Result<Option<String>, String> {
    let mut settings = load(cx)?;
    if settings.general.lock_cmd.trim() != LOCK_CMD {
        return Ok(Some(format!(
            "lock_cmd has been changed since setup set it (to \"{}\"), so it was left alone",
            settings.general.lock_cmd
        )));
    }
    settings.general.set_command("lock_cmd", previous.to_string());
    write(cx, &settings)
}
