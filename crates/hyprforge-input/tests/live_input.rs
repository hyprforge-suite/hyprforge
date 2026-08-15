//! Manual tests that a real Hyprland agrees with everything
//! [`hyprforge_input::catalog`] claims. Not run by default —
//! `cargo test --test live_input -- --ignored`, on a machine running
//! Hyprland.
//!
//! This is the test that makes the catalog trustworthy. Every entry asserts
//! two things about the compositor — that the option exists, and what type
//! it has — and both are exactly the kind of claim that rots quietly as
//! Hyprland changes. A renamed key or a type that became a float shows up
//! here, not in someone's config.
//!
//! Nothing here writes to the user's files. Settings are pushed with
//! `hyprctl eval`, read back with `hyprctl getoption`, and undone with
//! `hyprctl reload`, which restores every value from the user's own config
//! — verified: a `repeat_delay` set to 601 by eval was back to 600 with
//! `set` false after a reload.

use hyprforge_input::catalog::{Kind, CATALOG};
use hyprforge_input::apply::generate;
use hyprforge_input::{Settings, Value};
use std::process::Command;
use std::sync::{Mutex, MutexGuard};

/// These tests share one compositor and each reloads to undo itself, so two
/// running at once would restore each other's values mid-assertion. Same
/// reason as the shortcuts live suite.
static COMPOSITOR: Mutex<()> = Mutex::new(());

fn compositor() -> MutexGuard<'static, ()> {
    COMPOSITOR.lock().unwrap_or_else(|e| e.into_inner())
}

fn hyprctl(args: &[&str]) -> String {
    let out = Command::new("hyprctl")
        .args(args)
        .output()
        .expect("hyprctl not runnable");
    String::from_utf8_lossy(&out.stdout).to_string()
}

fn reload() {
    hyprctl(&["reload"]);
}

fn getoption(key: &str) -> serde_json::Value {
    let raw = hyprctl(&["getoption", key, "-j"]);
    serde_json::from_str(raw.trim())
        .unwrap_or_else(|_| panic!("getoption {key} returned no JSON: {raw}"))
}

/// Every catalog key exists on the running compositor, with the type the
/// catalog claims. A key Hyprland renamed, or one whose type changed, fails
/// here rather than producing a config error for a user.
#[test]
#[ignore]
fn hyprland_has_every_catalogued_option_with_the_declared_type() {
    let _live = compositor();
    let mut wrong = Vec::new();
    for setting in CATALOG.settings {
        let raw = hyprctl(&["getoption", setting.key, "-j"]);
        if !raw.trim_start().starts_with('{') {
            wrong.push(format!("{}: compositor says {}", setting.key, raw.trim()));
            continue;
        }
        let obj: serde_json::Value = serde_json::from_str(raw.trim()).unwrap();
        let want = setting.kind.hyprctl_field();
        if obj.get(want).is_none() {
            let got: Vec<&str> = obj
                .as_object()
                .unwrap()
                .keys()
                .filter(|k| *k != "option" && *k != "set")
                .map(|k| k.as_str())
                .collect();
            wrong.push(format!(
                "{}: catalog says {want}, compositor says {}",
                setting.key,
                got.join(",")
            ));
        }
    }
    assert!(wrong.is_empty(), "catalog disagrees with Hyprland:\n{}", wrong.join("\n"));
}

/// The generated file loads cleanly with every option set at once — the
/// broadest check that the Lua this crate writes is Lua Hyprland accepts,
/// including the nesting and the number formatting.
#[test]
#[ignore]
fn hyprland_accepts_a_file_setting_every_option() {
    let _live = compositor();
    reload();
    let before = config_errors();
    assert!(
        before.is_empty(),
        "config already has errors, so nothing here could be attributed:\n{before}"
    );

    let mut settings = Settings::default();
    for setting in CATALOG.settings {
        settings.set(setting.key, probe_value(&setting.kind));
    }
    assert_eq!(settings.validate(&CATALOG), vec![], "the probe values must be storable");

    eval_and_assert_clean(&generate(&settings));
    reload();
}

/// A saved setting actually reaches the compositor. Without this the module
/// could generate perfect Lua that Hyprland loads and ignores, and every
/// test above would still pass.
#[test]
#[ignore]
fn a_generated_setting_takes_effect() {
    let _live = compositor();
    reload();

    // Deliberately not a value anything else here uses, and read back
    // before the reload that undoes it.
    let mut settings = Settings::default();
    settings.set("input:repeat_delay", Value::Int(637));
    settings.set("input:touchpad:scroll_factor", Value::Float(1.5));
    eval_and_assert_clean(&generate(&settings));

    assert_eq!(getoption("input:repeat_delay")["int"], 637);
    let factor = getoption("input:touchpad:scroll_factor")["float"]
        .as_f64()
        .unwrap();
    assert!((factor - 1.5).abs() < 1e-6, "got {factor}");

    reload();
    assert_ne!(
        getoption("input:repeat_delay")["int"],
        637,
        "reload should have restored the user's own value"
    );
}

/// The fact the whole design rests on: a later `hl.config` beats an earlier
/// one. If this ever stopped being true, `setup::PLACEMENT` would be
/// backwards and every save would silently do nothing.
#[test]
#[ignore]
fn a_later_config_call_wins_over_an_earlier_one() {
    let _live = compositor();
    reload();

    eval_and_assert_clean("hl.config({ input = { repeat_delay = 611 } })\n");
    assert_eq!(getoption("input:repeat_delay")["int"], 611);
    eval_and_assert_clean("hl.config({ input = { repeat_delay = 622 } })\n");
    assert_eq!(
        getoption("input:repeat_delay")["int"],
        622,
        "the later call must win"
    );

    reload();
}

/// The other half of the design: a call updates only the keys it passes, so
/// the generated overlay leaves everything else to the user's own config.
#[test]
#[ignore]
fn a_partial_config_call_leaves_other_keys_alone() {
    let _live = compositor();
    reload();
    let layout_before = getoption("input:kb_layout")["str"].clone();

    let mut settings = Settings::default();
    settings.set("input:repeat_delay", Value::Int(644));
    eval_and_assert_clean(&generate(&settings));

    assert_eq!(
        getoption("input:kb_layout")["str"],
        layout_before,
        "writing repeat_delay must not disturb the layout"
    );
    reload();
}

/// What [`hyprforge_input::import`] reads has to mean what it thinks it
/// means: `set` false before anything writes the key, true after.
#[test]
#[ignore]
fn the_set_flag_tracks_whether_a_key_was_written() {
    let _live = compositor();
    reload();
    assert_eq!(
        getoption("input:follow_mouse_shrink")["set"],
        false,
        "picked because this machine's config doesn't set it"
    );

    let mut settings = Settings::default();
    settings.set("input:follow_mouse_shrink", Value::Int(3));
    eval_and_assert_clean(&generate(&settings));
    assert_eq!(getoption("input:follow_mouse_shrink")["set"], true);

    reload();
}

/// A value the catalog declares within range that Hyprland nonetheless
/// refuses would be a range this crate got wrong. Every catalogued minimum
/// and maximum is pushed at the compositor.
#[test]
#[ignore]
fn hyprland_accepts_the_extremes_of_every_declared_range() {
    let _live = compositor();
    reload();
    assert!(config_errors().is_empty(), "config already has errors");

    for extreme in [Extreme::Min, Extreme::Max] {
        let mut settings = Settings::default();
        for setting in CATALOG.settings {
            if let Some(v) = range_extreme(&setting.kind, extreme) {
                settings.set(setting.key, v);
            }
        }
        eval_and_assert_clean(&generate(&settings));
    }
    reload();
}

#[derive(Clone, Copy)]
enum Extreme {
    Min,
    Max,
}

fn range_extreme(kind: &Kind, which: Extreme) -> Option<Value> {
    match (kind, which) {
        (Kind::Int { min, .. }, Extreme::Min) => min.map(Value::Int),
        (Kind::Int { max, .. }, Extreme::Max) => max.map(Value::Int),
        (Kind::Float { min, .. }, Extreme::Min) => min.map(Value::Float),
        (Kind::Float { max, .. }, Extreme::Max) => max.map(Value::Float),
        _ => None,
    }
}

/// A value that exercises the option without depending on the hardware
/// attached: never the default, so a setting silently ignored is visible,
/// but always inside the catalogued range.
fn probe_value(kind: &Kind) -> Value {
    match *kind {
        Kind::Bool { default } => Value::Bool(!default),
        Kind::Int { default, min, max } => {
            let bumped = default.saturating_add(1);
            Value::Int(match (min, max) {
                (_, Some(hi)) if bumped > hi => hi,
                (Some(lo), _) if bumped < lo => lo,
                _ => bumped,
            })
        }
        Kind::IntEnum { default, choices } => Value::Int(
            choices
                .iter()
                .map(|(v, _)| *v)
                .find(|v| *v != default)
                .unwrap_or(default),
        ),
        Kind::Float { default, min, max } => {
            let bumped = default + 0.5;
            Value::Float(match (min, max) {
                (_, Some(hi)) if bumped > hi => hi,
                (Some(lo), _) if bumped < lo => lo,
                _ => bumped,
            })
        }
        Kind::Text { default } | Kind::Color { default } => Value::Text(default.to_string()),
        Kind::TextEnum { default, choices } => Value::Text(
            choices
                .iter()
                .find(|c| **c != default)
                .unwrap_or(&default)
                .to_string(),
        ),
    }
}

fn eval_and_assert_clean(lua: &str) {
    let dir = std::env::temp_dir().join("hyprforge-live-input");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("probe.lua");
    std::fs::write(&path, lua).unwrap();

    let out = hyprctl(&["eval", &format!("dofile([[{}]])", path.display())]);
    let errors = config_errors();
    let _ = std::fs::remove_file(&path);

    // A failing probe panics, which would skip the caller's reload and
    // leave the session running whatever this just set — swapped mouse
    // buttons and no focus-follows-mouse, in the broadest test. Undo first,
    // then report.
    if out.to_lowercase().contains("error") || !errors.is_empty() {
        reload();
    }
    assert!(
        !out.to_lowercase().contains("error"),
        "hyprctl eval refused it: {out}\n--- lua ---\n{lua}"
    );
    assert!(
        errors.is_empty(),
        "the compositor reported config errors: {errors}\n--- lua ---\n{lua}"
    );
}

fn config_errors() -> String {
    let out = hyprctl(&["configerrors"]);
    let trimmed = out.trim();
    if trimmed == "no errors" || trimmed.is_empty() {
        String::new()
    } else {
        trimmed.to_string()
    }
}
