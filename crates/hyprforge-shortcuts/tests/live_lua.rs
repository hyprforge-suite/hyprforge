//! Manual test that a real Hyprland accepts the binds we generate. Not run by
//! default — `cargo test --test live_lua -- --ignored`, on a machine running
//! Hyprland.
//!
//! This is the test that would have caught the "expected a table" failure that
//! took a whole config down: every claim the catalog makes about a
//! dispatcher's call shape is checked against the compositor that has to
//! accept it, rather than against this crate's own idea of Lua. An entry with
//! the wrong shape or a misspelled key fails here instead of in someone's
//! config.
//!
//! Every probe binds a chord nothing sane uses (SUPER+ALT+CTRL+SHIFT+F13/F14),
//! so evaluating them can't shadow a real shortcut, and they're gone at the
//! next reload either way.
//!
//! The tests take [`COMPOSITOR`] because they share one: each probe loads
//! binds into the running Hyprland and then reloads to drop them, so two
//! running at once means one test's reload deletes another's bind before it
//! can be observed. That failed exactly the way a real bug would — a
//! description "not reaching `hyprctl binds`" — so the lock is what keeps a
//! genuine failure here believable.

use hyprforge_shortcuts::catalog::{self, CallShape, Entry, ParamKind};
use hyprforge_shortcuts::codegen::generate;
use hyprforge_shortcuts::model::{Action, BindFlags, KeyCombo, Modifier, ParamValue, Shortcut};
use std::collections::BTreeMap;
use std::process::Command;
use std::sync::{Mutex, MutexGuard};

/// The running compositor, which every probe here mutates. A poisoned lock
/// is still usable: the only shared state is Hyprland's own bind list, and
/// each test reloads before it starts.
static COMPOSITOR: Mutex<()> = Mutex::new(());

fn compositor() -> MutexGuard<'static, ()> {
    COMPOSITOR.lock().unwrap_or_else(|e| e.into_inner())
}

/// The four-modifier chord no real config binds, plus an F-key per probe so
/// they don't overwrite each other.
fn probe_combo(n: usize) -> KeyCombo {
    KeyCombo {
        mods: vec![Modifier::Super, Modifier::Alt, Modifier::Ctrl, Modifier::Shift],
        // F13–F24 exist in xkb and no keyboard here emits them.
        key: format!("F{}", 13 + n % 12),
    }
}

/// A plausible value for a param, good enough for Hyprland to type-check the
/// call. Deliberately harmless: no real workspace, monitor or command is
/// named, and nothing here would do anything even if it did fire.
fn sample_value(kind: ParamKind) -> ParamValue {
    match kind {
        ParamKind::Bool => ParamValue::Bool(true),
        ParamKind::Int => ParamValue::Int(1),
        ParamKind::Workspace => ParamValue::Int(1),
        ParamKind::Enum(values) => ParamValue::Str(values[0].to_string()),
        ParamKind::Text => ParamValue::Str("hyprforge-probe".to_string()),
    }
}

/// Which params to set on a probe.
///
/// Both matter, and they catch opposite mistakes. `All` catches an optional
/// key Hyprland doesn't actually accept. `RequiredOnly` catches a key marked
/// optional that the compositor in fact demands — the shape a user gets by
/// leaving a field blank, and exactly how `hl.pass` was caught being
/// mis-documented.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Fill {
    All,
    RequiredOnly,
}

fn probe_shortcuts(fill: Fill) -> Vec<Shortcut> {
    catalog::ENTRIES
        .iter()
        .enumerate()
        .map(|(i, entry)| {
            let params: BTreeMap<String, ParamValue> = match entry.shape {
                // A positional dispatcher takes exactly one value.
                CallShape::Positional => entry
                    .params
                    .first()
                    .map(|p| (p.key.to_string(), sample_value(p.kind)))
                    .into_iter()
                    .collect(),
                CallShape::None => BTreeMap::new(),
                CallShape::Table => entry
                    .params
                    .iter()
                    .filter(|p| fill == Fill::All || p.required)
                    .map(|p| (p.key.to_string(), sample_value(p.kind)))
                    .collect(),
            };
            Shortcut {
                name: format!("hyprforge-probe-{}", entry.id),
                enabled: true,
                combo: probe_combo(i),
                action: Action::with_params(entry.dispatcher, params),
                description: format!("live probe {}", entry.id),
                flags: BindFlags::default(),
            }
        })
        .collect()
}

/// Every catalog entry, loaded into the running compositor.
///
/// Failure here means the catalog is lying about a dispatcher — the whole
/// point of having one. The generated Lua is printed on failure so the
/// offending line is readable directly.
#[test]
#[ignore]
fn hyprland_accepts_every_catalog_entry() {
    let _live = compositor();
    reload();
    let errors_before = config_errors();
    assert!(
        errors_before.is_empty(),
        "config already has errors, so nothing here could be attributed:\n{errors_before}"
    );

    eval_and_assert_clean(&generate(&probe_shortcuts(Fill::All)), "every param set");
    eval_and_assert_clean(
        &generate(&probe_shortcuts(Fill::RequiredOnly)),
        "required params only",
    );
    reload();
}

/// The raw escape hatch and the bind flags, which aren't catalog entries and
/// so aren't covered above.
#[test]
#[ignore]
fn hyprland_accepts_raw_actions_and_flags() {
    let _live = compositor();
    reload();
    assert!(config_errors().is_empty(), "config already has errors");

    let shortcuts = vec![
        Shortcut {
            name: "hyprforge-probe-raw".to_string(),
            enabled: true,
            combo: probe_combo(0),
            action: Action::with_raw("focus", "{ direction = [[left]] }"),
            description: "live probe raw".to_string(),
            flags: BindFlags::default(),
        },
        Shortcut {
            name: "hyprforge-probe-flags".to_string(),
            enabled: true,
            combo: probe_combo(1),
            action: Action::with_params("window.drag", BTreeMap::new()),
            description: "live probe flags".to_string(),
            // Not every flag at once: Hyprland rejects `repeating` together
            // with `release` or `long_press` ("long_press / release is
            // incompatible with repeat"), which is why
            // `BindFlags::conflict` exists and why the editor won't offer
            // the combination.
            flags: BindFlags {
                mouse: true,
                locked: true,
                repeating: true,
                non_consuming: true,
                ..BindFlags::default()
            },
        },
        Shortcut {
            name: "hyprforge-probe-flags-release".to_string(),
            enabled: true,
            combo: probe_combo(2),
            action: Action::with_params("window.close", BTreeMap::new()),
            description: "live probe release flags".to_string(),
            flags: BindFlags {
                release: true,
                long_press: true,
                locked: true,
                ..BindFlags::default()
            },
        },
    ];

    eval_and_assert_clean(&generate(&shortcuts), "raw and flags");
    reload();
}

/// The flag combination Hyprland refuses must actually be refused — this
/// pins the constraint `BindFlags::conflict` encodes to the compositor's
/// real behaviour, so the editor isn't guarding against a rule that quietly
/// stopped existing.
#[test]
#[ignore]
fn hyprland_rejects_repeating_with_release() {
    let _live = compositor();
    reload();
    assert!(config_errors().is_empty(), "config already has errors");

    let shortcut = Shortcut {
        name: "hyprforge-probe-bad-flags".to_string(),
        enabled: true,
        combo: probe_combo(0),
        action: Action::with_params("window.close", BTreeMap::new()),
        description: "live probe incompatible flags".to_string(),
        flags: BindFlags { repeating: true, release: true, ..BindFlags::default() },
    };
    assert!(shortcut.flags.conflict().is_some(), "the model should call this a conflict");

    let dir = std::env::temp_dir().join("hyprforge-live-binds");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("probe-bad-flags.lua");
    std::fs::write(&path, generate(&[shortcut])).unwrap();
    let out = Command::new("hyprctl")
        .arg("eval")
        .arg(format!("dofile([[{}]])", path.display()))
        .output()
        .expect("hyprctl not runnable");
    let body = String::from_utf8_lossy(&out.stdout).to_lowercase();
    let _ = std::fs::remove_file(&path);
    reload();

    assert!(body.contains("incompatible"), "expected a rejection, got: {body}");
}

/// The description round-trips through the compositor — that's what lets the
/// app recognise its own binds in `hyprctl binds` later.
#[test]
#[ignore]
fn a_generated_description_reaches_hyprctl_binds() {
    let _live = compositor();
    let shortcuts = vec![Shortcut {
        name: "hyprforge-probe-desc".to_string(),
        enabled: true,
        combo: probe_combo(0),
        action: Action::with_params("window.close", BTreeMap::new()),
        description: "live probe description".to_string(),
        flags: BindFlags::default(),
    }];
    reload();
    eval_and_assert_clean(&generate(&shortcuts), "description");

    // Deliberately before the reload below: reloading drops the probe bind,
    // so listing has to happen while it's still live.
    let listed = Command::new("hyprctl").arg("binds").arg("-j").output().unwrap();
    let listed = String::from_utf8_lossy(&listed.stdout);
    assert!(
        listed.contains("hyprforge: live probe description"),
        "the description did not reach hyprctl binds"
    );
    reload();
}

fn eval_and_assert_clean(lua: &str, what: &str) {
    let dir = std::env::temp_dir().join("hyprforge-live-binds");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(format!("probe-{what}.lua").replace(' ', "-"));
    std::fs::write(&path, lua).unwrap();

    let out = Command::new("hyprctl")
        .arg("eval")
        .arg(format!("dofile([[{}]])", path.display()))
        .output()
        .expect("hyprctl not runnable");
    let body = String::from_utf8_lossy(&out.stdout);
    let body = body.trim().to_string();

    let errors = config_errors();
    let clean = !body.to_lowercase().contains("error") && errors.is_empty();
    if !clean {
        eprintln!("--- generated ({what}) ---\n{lua}");
    }
    let _ = std::fs::remove_file(&path);
    assert!(clean, "Hyprland rejected the {what} probes.\neval: {body}\nconfigerrors: {errors}");
}

/// Clears the probe binds out of the live session, and with them any
/// `configerrors` a failed probe left behind. Every test brackets itself with
/// this rather than relying on the previous one having cleaned up — otherwise
/// one genuine failure cascades into every test after it failing for the
/// wrong reason.
fn reload() {
    let _ = Command::new("hyprctl").arg("reload").output();
}

fn config_errors() -> String {
    let out = Command::new("hyprctl").arg("configerrors").output().expect("hyprctl not runnable");
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// Sanity check that runs without Hyprland: every entry produces a bind line.
/// Catches a catalog entry that generates nothing at all, which the ignored
/// tests above would never get the chance to reject.
#[test]
fn every_catalog_entry_generates_a_bind() {
    let lua = generate(&probe_shortcuts(Fill::All));
    assert_eq!(
        lua.lines().filter(|l| l.starts_with("hl.bind(")).count(),
        catalog::ENTRIES.len(),
        "not every catalog entry produced a bind:\n{lua}"
    );
    // Nothing may render an empty call for a table dispatcher — that is
    // exactly the shape Hyprland rejects.
    for entry in catalog::ENTRIES.iter().filter(|e: &&Entry| e.shape == CallShape::Table) {
        assert!(
            !lua.contains(&format!("hl.dsp.{}()", entry.dispatcher)),
            "{} generated an empty call for a table dispatcher",
            entry.id
        );
    }
}
