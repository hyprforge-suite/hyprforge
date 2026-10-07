//! The steps that finish a Hyprforge install, each one checked, applied
//! and undone — the page of instructions the installer used to print,
//! done instead of described.
//!
//! Hyprforge-internal, published so the suite's applications can build
//! from crates.io; the API follows the suite, not semver. The Settings
//! app is the one consumer: its Set up page renders [`ITEMS`] with their
//! [`State`]s, and `hyprforge-settings --setup` is the same thing for a
//! terminal and for the installer.
//!
//! # What an item is
//!
//! An [`Item`] is plain data — an id, a label, a one-line why, whether it
//! is ticked by default, which programs it needs — with three operations
//! behind it, reached through [`check`], [`apply`] and [`undo`]:
//!
//! - **check** reports a [`State`]: done, to do (and what it would
//!   change), unavailable (and why), or unknown. A check that could not
//!   run is [`State::Unknown`], never done — the installer's three-way
//!   rule, and CLAUDE.md's "could not read is not empty".
//! - **apply** runs only on an item that just checked as to do, and
//!   records in `setup.toml` what it changed and what was there before.
//! - **undo** puts back exactly what was recorded, and nothing setup did
//!   not do: a service a package enabled for every user is not "disabled"
//!   by an undo, because setup never enabled it.
//!
//! Every item writes through the module that already owns its file —
//! `shortcuts.toml` through `hyprforge-shortcuts`, `idle.toml` through
//! `hyprforge-ecosystem`, `mimeapps.list` through `hyprforge-mime` — so
//! the Settings page for that file shows what setup did and can change
//! it.
//!
//! # Two decisions someone would otherwise reverse
//!
//! **A chord that is taken is skipped and named, never overwritten.**
//! Keybinds are required last, so a bind written over the user's own
//! would silently win. The item reports [`State::Unavailable`] with what
//! holds the chord.
//!
//! **Only an item that is to do is applied, and an unreadable record
//! refuses everything.** [`apply`] re-checks each item first; `--yes`
//! never touches something unknown. A `setup.toml` that will not parse
//! stops both apply and undo — undoing "nothing" would leave every change
//! in place, and applying would overwrite the only record of what was
//! there before.
//!
//! **One item is done as root, and none by default.** "Previous versions
//! of your files" lets this user read snapper's snapshots of `/home`,
//! which takes snapper's own `set-config` through `pkexec` — a password
//! prompt — so it is off unless ticked. Everything else setup does is in
//! files this user owns.
//!
//! # What is where
//!
//! [`Env`] is every path, explicit so a test can point all of it at a
//! temp directory; [`System`] is every other process (`systemctl --user`,
//! `hyprctl`, `pgrep`, the session bus), with [`RealSystem`] bounding each
//! call and `mock::MockSystem` (feature `mock`) standing in for all of
//! it, `pkexec snapper` included; [`record`] is `setup.toml`; [`state`] is the four states and the
//! `--porcelain` line the installer parses.

mod env;
mod items;
#[cfg(any(test, feature = "mock"))]
pub mod mock;
pub mod record;
pub mod state;
pub mod system;
#[cfg(test)]
mod tests;

pub use env::Env;
pub use items::{item, with_file_chooser, without_file_chooser, Item, PortalEdit, ITEMS, SUITE_UNITS};
pub use record::{Record, RecordError};
pub use state::{porcelain_line, State};
pub use system::{RealSystem, System, UnitState};

use items::Cx;

/// How one item's apply or undo went.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outcome {
    pub id: &'static str,
    pub result: Result<String, String>,
}

impl Outcome {
    pub fn ok(&self) -> bool {
        self.result.is_ok()
    }
}

/// Checks one item.
pub fn check(env: &Env, sys: &dyn System, item: &Item) -> State {
    item.check(&Cx { env, sys })
}

/// Every item with its state, in order.
pub fn check_all(env: &Env, sys: &dyn System) -> Vec<(&'static Item, State)> {
    ITEMS.iter().map(|item| (item, check(env, sys, item))).collect()
}

/// Whether setup has ever recorded a change — the Settings app opens on
/// Set up at first launch only while this is false.
pub fn has_record(env: &Env) -> bool {
    record::exists(&env.setup_toml())
}

/// Applies the items named in `ids`, in [`ITEMS`] order whatever order
/// they were given in.
///
/// Each is checked again first: only [`State::Todo`] is applied. Done is
/// reported as such and recorded as nothing — so a later undo leaves it
/// alone — and unavailable or unknown is a failed outcome carrying the
/// reason. The record is saved after every item, so an interruption
/// halfway keeps what the first half did undoable.
///
/// `Err` without touching anything when `setup.toml` cannot be read.
pub fn apply(env: &Env, sys: &dyn System, ids: &[&str]) -> Result<Vec<Outcome>, RecordError> {
    let path = env.setup_toml();
    let mut record = record::load(&path)?;
    let cx = Cx { env, sys };
    let mut outcomes = Vec::new();
    for item in ITEMS.iter().filter(|i| ids.contains(&i.id)) {
        let result = match item.check(&cx) {
            State::Done => Ok("already done".to_string()),
            State::Unavailable { why } | State::Unknown { why } => Err(why),
            State::Todo { what } => match item.apply(&cx) {
                Err(e) => Err(e),
                Ok(applied) => {
                    record
                        .items
                        .insert(item.id.to_string(), record::Entry { applied: what.clone(), change: applied.change });
                    match record::save(&path, &record) {
                        // The change is made; only its record is missing.
                        // Saying so is all that is left to do.
                        Err(e) => Err(format!("{what} — done, but it can't be undone: {e}")),
                        Ok(()) => Ok(match applied.note {
                            Some(note) => format!("{what} ({note})"),
                            None => what,
                        }),
                    }
                }
            },
        };
        outcomes.push(Outcome { id: item.id, result });
    }
    Ok(outcomes)
}

/// Undoes what setup recorded for `ids`, or for everything recorded when
/// `ids` is empty — in reverse [`ITEMS`] order, so the wiring that the
/// other items' files depend on goes last.
///
/// An item with no record is reported as such and left alone: setup
/// didn't change it, so there is nothing of setup's to take back.
///
/// `Err` without touching anything when `setup.toml` cannot be read —
/// see the crate doc.
pub fn undo(env: &Env, sys: &dyn System, ids: &[&str]) -> Result<Vec<Outcome>, RecordError> {
    let path = env.setup_toml();
    let mut record = record::load(&path)?;
    let cx = Cx { env, sys };
    let mut outcomes = Vec::new();
    for item in ITEMS.iter().rev() {
        let asked = ids.contains(&item.id);
        let unit_masked = item.unit().is_some_and(|u| record.masked.iter().any(|m| m == u));
        let entry = record.items.get(item.id).cloned();
        if !(asked || ids.is_empty() && (entry.is_some() || unit_masked)) {
            continue;
        }
        let mut result = match &entry {
            None => Ok("setup didn't change this, so there is nothing to undo".to_string()),
            Some(entry) => match item.undo(&cx, &entry.change) {
                Err(e) => Err(e),
                Ok(note) => {
                    record.items.remove(item.id);
                    Ok(match note {
                        Some(note) => format!("undid: {} ({note})", entry.applied),
                        None => format!("undid: {}", entry.applied),
                    })
                }
            },
        };
        // "Turn off for me" is taken back with the item it turned off.
        if let (Ok(_), Some(unit)) = (&result, item.unit().filter(|_| unit_masked)) {
            result = match sys.unmask(unit) {
                Ok(()) => {
                    record.masked.retain(|m| m != unit);
                    result.map(|r| format!("{r}; unmasked {unit}"))
                }
                Err(e) => Err(format!("couldn't unmask {unit}: {e}")),
            };
        }
        if let Err(e) = record::save(&path, &record) {
            result = Err(format!("couldn't update the setup record: {e}"));
        }
        outcomes.push(Outcome { id: item.id, result });
    }
    Ok(outcomes)
}

/// Turns a service off for this user alone: `systemctl --user mask
/// --now`, recorded so undoing the item unmasks it.
///
/// The off switch the Set up page offers for a service setup did not
/// enable. A package's `systemctl --global preset` links a unit for every
/// user under `/etc`, and `systemctl --user disable` only removes links
/// under `~/.config` — it would leave the unit enabled and report
/// success. A mask is the one per-user switch that wins over that.
pub fn turn_off_for_me(env: &Env, sys: &dyn System, id: &str) -> Result<Outcome, RecordError> {
    let path = env.setup_toml();
    let mut record = record::load(&path)?;
    let Some(item) = item(id) else {
        return Ok(Outcome { id: "unknown", result: Err(format!("no setup item called {id}")) });
    };
    let Some(unit) = item.unit() else {
        return Ok(Outcome { id: item.id, result: Err(format!("{} isn't a service", item.label)) });
    };
    let result = match sys.mask_now(unit) {
        Err(e) => Err(e),
        Ok(()) => {
            if !record.masked.iter().any(|m| m == unit) {
                record.masked.push(unit.to_string());
            }
            match record::save(&path, &record) {
                Ok(()) => Ok(format!("masked {unit} for you; undo unmasks it")),
                Err(e) => Err(format!("masked {unit}, but it can't be undone from here: {e}")),
            }
        }
    };
    Ok(Outcome { id: item.id, result })
}
