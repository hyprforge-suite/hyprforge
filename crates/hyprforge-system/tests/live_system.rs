//! Manual tests that a real Hyprland agrees with the catalogue.
//! `cargo test --test live_system -- --ignored`, on a machine running
//! Hyprland.
//!
//! This suite found four options the wiki documents and Hyprland 0.56.1
//! doesn't have. Without it they would have shipped as settings that
//! silently do nothing.

use hyprforge_system::apply::generate;
use hyprforge_system::catalog::{Kind, CATALOG};
use hyprforge_core::hlconfig::{Settings, Value};
use std::process::Command;
use std::sync::{Mutex, MutexGuard};

static COMPOSITOR: Mutex<()> = Mutex::new(());

fn compositor() -> MutexGuard<'static, ()> {
    COMPOSITOR.lock().unwrap_or_else(|e| e.into_inner())
}

fn hyprctl(args: &[&str]) -> String {
    String::from_utf8_lossy(
        &Command::new("hyprctl")
            .args(args)
            .output()
            .expect("hyprctl not runnable")
            .stdout,
    )
    .to_string()
}

fn reload() {
    hyprctl(&["reload"]);
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

/// Every catalogued key exists with the type this crate claims.
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

/// The options declared unsupported must still be absent. If one ships,
/// the note telling users it doesn't exist becomes the wrong answer.
#[test]
#[ignore]
fn options_declared_unsupported_are_still_absent() {
    let _live = compositor();
    for (key, _) in CATALOG.unsupported {
        let raw = hyprctl(&["getoption", key, "-j"]);
        assert!(
            !raw.trim_start().starts_with('{'),
            "{key} exists now — it should be catalogued instead of called unreleased: {raw}"
        );
    }
}

/// The generated file loads with every option set at once.
///
/// Deliberately excludes the handful that would leave this machine
/// unusable for the rest of the run — a test that blanks the screen is
/// not a test anyone will run twice.
#[test]
#[ignore]
fn hyprland_accepts_a_file_setting_every_safe_option() {
    let _live = compositor();
    reload();
    let before = config_errors();
    assert!(before.is_empty(), "config already has errors:\n{before}");

    let mut settings = Settings::default();
    for setting in CATALOG.settings {
        if UNSAFE_TO_PROBE.contains(&setting.key) {
            continue;
        }
        settings.set(setting.key, probe_value(&setting.kind));
    }
    assert_eq!(settings.validate(&CATALOG), vec![], "probe values must be storable");

    eval_and_assert_clean(&generate(&settings));
    reload();
}

/// Options this suite writes but never *flips*, because doing so on the
/// machine running the test would make it unusable — a black screen or a
/// compositor that stops rendering can't report a test result.
///
/// They are still catalogued and still type-checked above; only the
/// "set everything at once" probe skips them.
const UNSAFE_TO_PROBE: &[&str] = &[
    // Turning colour management off, or forcing scanout, is where the
    // screen goes black on some drivers.
    "render:cm_enabled",
    "render:direct_scanout",
    "render:xp_mode",
    "render:non_shader_cm",
    "render:use_fp16",
    // Refuses X11 apps outright.
    "xwayland:enabled",
    // Stops the config reloading, which every other test here depends on.
    "misc:disable_autoreload",
];

#[test]
#[ignore]
fn hyprland_accepts_the_extremes_of_every_declared_range() {
    let _live = compositor();
    reload();
    assert!(config_errors().is_empty(), "config already has errors");

    for extreme in [Extreme::Min, Extreme::Max] {
        let mut settings = Settings::default();
        for setting in CATALOG.settings {
            if UNSAFE_TO_PROBE.contains(&setting.key) {
                continue;
            }
            if let Some(v) = range_extreme(&setting.kind, extreme) {
                settings.set(setting.key, v);
            }
        }
        eval_and_assert_clean(&generate(&settings));
    }
    reload();
}

/// A plain colour is written as rgba() and reported as a number, so the
/// round trip is worth pinning against the real thing.
#[test]
#[ignore]
fn a_plain_colour_round_trips_through_hyprland() {
    let _live = compositor();
    reload();
    let mut settings = Settings::default();
    settings.set("misc:background_color", Value::Text("rgba(112233ff)".into()));
    eval_and_assert_clean(&generate(&settings));

    let raw = hyprctl(&["getoption", "misc:background_color", "-j"]);
    let obj: serde_json::Value = serde_json::from_str(raw.trim()).unwrap();
    let live = obj.get("int").and_then(|v| v.as_u64()).unwrap();
    assert_eq!(
        hyprforge_core::hlconfig::import::read_int_color(live),
        Some(Value::Text("rgba(112233ff)".into())),
        "live colour {live} didn't convert back"
    );
    reload();
}

#[derive(Clone, Copy)]
enum Extreme {
    Min,
    Max,
}

fn range_extreme(kind: &Kind, which: Extreme) -> Option<Value> {
    match (kind, which) {
        (Kind::Int { min, .. } | Kind::Gaps { min, .. }, Extreme::Min) => min.map(Value::Int),
        (Kind::Int { max, .. } | Kind::Gaps { max, .. }, Extreme::Max) => max.map(Value::Int),
        (Kind::Float { min, .. }, Extreme::Min) => min.map(Value::Float),
        (Kind::Float { max, .. }, Extreme::Max) => max.map(Value::Float),
        _ => None,
    }
}

fn probe_value(kind: &Kind) -> Value {
    match *kind {
        Kind::Bool { default } => Value::Bool(!default),
        Kind::Int { default, min, max } | Kind::Gaps { default, min, max } => {
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
            let bumped = default + 0.1;
            Value::Float(match (min, max) {
                (_, Some(hi)) if bumped > hi => hi,
                (Some(lo), _) if bumped < lo => lo,
                _ => bumped,
            })
        }
        Kind::Text { default } | Kind::Color { default } | Kind::ColorInt { default } => {
            Value::Text(default.to_string())
        }
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
    let dir = std::env::temp_dir().join("hyprforge-live-system");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("probe.lua");
    std::fs::write(&path, lua).unwrap();

    let out = hyprctl(&["eval", &format!("dofile([[{}]])", path.display())]);
    let errors = config_errors();
    let _ = std::fs::remove_file(&path);

    // Undo before reporting: a failing probe panics, which would skip the
    // caller's reload and leave the session running whatever this set.
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
