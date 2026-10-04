//! `GTK_USE_PORTAL=1`, so GTK apps that would draw their own file dialog
//! ask the portal — and so get Files' one.
//!
//! Off by default: it changes every GTK app's dialog, which is more than
//! anyone asked for by installing a file manager. Through the Session
//! page's environment variables (`session.toml` → `hl.env` in
//! `session.lua`), where it shows up and can be removed by hand. A new
//! variable reaches programs started after the session reads it, so the
//! item says it takes effect at next login.
//!
//! Which row counts is `hyprforge_core::supersede`'s rule, as everywhere
//! `hl.env` is read: the *last* enabled row for a name is the one in
//! effect.

use super::{Applied, Cx};
use crate::record::Change;
use crate::state::State;
use crate::system::Generated;
use hyprforge_session::environment::Variable;
use hyprforge_session::storage::Session;

const NAME: &str = "GTK_USE_PORTAL";
const VALUE: &str = "1";

fn load(cx: &Cx<'_>) -> Result<Session, String> {
    hyprforge_session::storage::load(&cx.env.session_toml()).map_err(|e| e.to_string())
}

/// The value in effect, if the variable is set at all.
fn in_effect(session: &Session) -> Option<&str> {
    session
        .environment
        .variables
        .iter()
        .rev()
        .find(|v| v.enabled && v.name.trim() == NAME)
        .map(|v| v.value.as_str())
}

pub(super) fn check(cx: &Cx<'_>) -> State {
    let session = match load(cx) {
        Ok(s) => s,
        Err(e) => return State::Unknown { why: e },
    };
    match in_effect(&session) {
        Some(VALUE) => State::Done,
        Some(other) => State::Todo {
            what: format!("Set {NAME}=1 (now {other:?}) — takes effect at next login"),
        },
        None => State::Todo { what: format!("Set {NAME}=1 — takes effect at next login") },
    }
}

/// Saves and loads `session`, putting the old TOML back if Hyprland
/// refuses the generated file.
///
/// Generated with no "existing gestures": those come from evaluating the
/// user's own `hyprland.lua`, which needs the Lua importer this crate
/// does not link. A saved gesture that clashes with one of theirs would
/// make Hyprland refuse the file — in which case it is rolled back and
/// this reports the refusal, rather than anything being lost. The Session
/// page, which does evaluate the config, is the way through then.
fn save_and_load(cx: &Cx<'_>, previous: &Session, session: &Session) -> Result<Option<String>, String> {
    let path = cx.env.session_toml();
    hyprforge_session::storage::save(&path, session).map_err(|e| e.to_string())?;
    let file = Generated {
        path: cx.env.session_lua(),
        contents: hyprforge_session::apply::generate(session, &[]),
        empty: hyprforge_session::apply::generate(&Session::default(), &[]),
        subject: "session settings",
    };
    match cx.sys.apply_lua(&file) {
        Ok(_) => Ok(Some("takes effect at next login".to_string())),
        Err(e) => {
            if let Err(restore) = hyprforge_session::storage::save(&path, previous) {
                return Err(format!("{e} — and putting session.toml back failed too: {restore}"));
            }
            Err(e)
        }
    }
}

pub(super) fn apply(cx: &Cx<'_>) -> Result<Applied, String> {
    let previous = load(cx)?;
    let mut session = previous.clone();
    // Appended, so it is the last row for the name and the one in effect.
    session.environment.variables.push(Variable {
        name: NAME.to_string(),
        value: VALUE.to_string(),
        enabled: true,
    });
    let note = save_and_load(cx, &previous, &session)?;
    Ok(Applied { change: Change::GtkPortal {}, note })
}

/// Removes the row setup appended — the last enabled `GTK_USE_PORTAL=1`.
pub(super) fn undo(cx: &Cx<'_>) -> Result<Option<String>, String> {
    let previous = load(cx)?;
    let Some(i) = previous
        .environment
        .variables
        .iter()
        .rposition(|v| v.enabled && v.name.trim() == NAME && v.value == VALUE)
    else {
        return Ok(None);
    };
    let mut session = previous.clone();
    session.environment.variables.remove(i);
    save_and_load(cx, &previous, &session)
}
