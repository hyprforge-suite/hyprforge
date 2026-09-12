//! Does **systemd-logind** agree?
//!
//! The unit tests check this crate against its own idea of the D-Bus
//! interface. That is the same trap the option catalogues were in before
//! tier 2 existed: code can be internally consistent and wrong about the
//! service it talks to. A renamed argument or a changed return shape is
//! invisible here until something asks the real thing.
//!
//! **Every test in this file is read-only by default.** Nothing here
//! takes a real inhibit: this crate's whole reason to exist is that a
//! held-then-dropped file descriptor silently does nothing, so a test
//! that took one and then failed an assertion before releasing it would
//! leave this machine — a machine somebody is using — with a stray
//! system-wide "block sleep and idle" lock outstanding, with no signal
//! anywhere that it happened. That risk was not worth taking for
//! coverage a `ListInhibitors` read cannot already give: this machine
//! already has a hand-run `systemd-inhibit --what=sleep:handle-lid-switch
//! --why=working sleep 8h` running, which is exactly the "someone else
//! holds a lock" case `list_inhibitors` needs to prove it can report.
//!
//! **`Inhibit` is never called here**, under any `what`, and `shutdown`
//! is not exercised even indirectly by this file.
//!
//! Run with `cargo test -p hyprforge-power -- --ignored`.

use hyprforge_power::backend::InhibitBackend;
use hyprforge_power::logind::LogindBackend;
use hyprforge_power::types::InhibitError;

/// Same convention as the NetworkManager and BlueZ live tests: libtest
/// has no skipped state, so a check that could not run announces itself
/// rather than returning early and printing `ok` like one that passed.
const SKIP_MARKER: &str = "HYPRFORGE-SKIP:";

/// Connects, or says why it could not.
async fn backend() -> Option<LogindBackend> {
    match LogindBackend::connect().await {
        Ok(backend) => Some(backend),
        Err(e) => {
            eprintln!("{SKIP_MARKER} no system bus to reach logind on ({e})");
            None
        }
    }
}

/// The claim every other call rests on: logind is on the system bus
/// under its well-known name and answers `ListInhibitors` in the shape
/// this crate expects — `(what, who, why, mode, uid, pid)`, deserializing
/// without error regardless of how many locks exist right now.
#[tokio::test]
#[ignore]
async fn logind_answers_list_inhibitors_on_the_system_bus() {
    let Some(backend) = backend().await else { return };
    match backend.list_inhibitors().await {
        Ok(inhibitors) => {
            println!("{} inhibitor(s) currently held", inhibitors.len());
            for i in &inhibitors {
                println!(
                    "  what={:?} who={:?} why={:?} mode={:?} uid={} pid={}",
                    i.what, i.who, i.why, i.mode, i.uid, i.pid
                );
            }
        }
        Err(InhibitError::Unavailable) => {
            eprintln!("{SKIP_MARKER} systemd-logind is not answering");
        }
        Err(e) => panic!("logind answered, but not in the shape this crate expects: {e}"),
    }
}

/// The scope note's checkable case: this development machine has a
/// hand-run `systemd-inhibit --what=sleep:handle-lid-switch --why=working
/// sleep 8h` holding a lock — the exact thing the "keep awake" toggle is
/// meant to replace. If it is running, `ListInhibitors` must report a row
/// for it. If it is not (a different machine, or the process has since
/// exited), that is not a failure of this crate — it just means there is
/// nothing to find, and the test says so rather than asserting blind.
#[tokio::test]
#[ignore]
async fn a_hand_run_systemd_inhibit_is_visible_in_the_list() {
    let Some(backend) = backend().await else { return };
    let inhibitors = match backend.list_inhibitors().await {
        Ok(inhibitors) => inhibitors,
        Err(InhibitError::Unavailable) => {
            eprintln!("{SKIP_MARKER} systemd-logind is not answering");
            return;
        }
        Err(e) => panic!("listing inhibitors failed: {e}"),
    };

    let found = inhibitors
        .iter()
        .find(|i| i.who.contains("systemd-inhibit") || i.why == "working");
    match found {
        Some(i) => {
            assert!(i.what.contains("sleep"), "expected sleep among {:?}", i.what);
            assert_eq!(i.mode, "block");
            println!("found the hand-run inhibitor: {i:?}");
        }
        None => {
            eprintln!(
                "{SKIP_MARKER} no `systemd-inhibit --why=working` lock is currently held on \
                 this machine"
            );
        }
    }
}

/// `Inhibit` itself is never called in this file — see the module-level
/// doc comment for why holding a real, system-wide lock is not a risk
/// worth taking in an unattended test. This test only pins that the
/// *type this crate would send* assembles correctly, which the unit
/// tests in `types.rs` already cover in depth; it is repeated here as a
/// reminder, next to the live tests, of exactly what was deliberately
/// left unexercised against the real bus.
#[test]
fn taking_a_real_inhibit_is_deliberately_not_exercised_by_this_file() {
    use hyprforge_power::types::{What, WhatSet};
    // Not a call to `LogindBackend::take` — just proof the value it would
    // be called with is the one this file's doc comment describes.
    assert_eq!(WhatSet::keep_awake().to_arg(), "sleep:idle");
    assert!(!WhatSet::keep_awake().to_arg().contains("shutdown"));
    let _ = What::Shutdown; // named, never sent, anywhere in this file.
}
