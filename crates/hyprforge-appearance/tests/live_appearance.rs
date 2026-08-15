//! Manual tests that a real Hyprland agrees with everything
//! [`hyprforge_appearance::catalog`] claims. Not run by default —
//! `cargo test --test live_appearance -- --ignored`, on a machine running
//! Hyprland.
//!
//! This suite has already earned its place: the wiki documents a
//! `decoration:wobble` subcategory that Hyprland 0.56.1 does not have, and
//! it documents `css_gaps` as taking a string when the compositor's own
//! error says "an integer or a table". A catalog built from the wiki alone
//! would have shipped eight dead settings and a gap control that silently
//! did nothing.
//!
//! Nothing here writes to the user's files. Values are pushed with
//! `hyprctl eval` and undone with `hyprctl reload`, which restores
//! everything from the user's own config.

use hyprforge_appearance::animations::{self, Animation, Animations};
use hyprforge_appearance::apply::generate;
use hyprforge_appearance::catalog::{Kind, CATALOG};
use hyprforge_appearance::storage::Appearance;
use hyprforge_core::hlconfig::{Settings, Value};
use std::process::Command;
use std::sync::{Mutex, MutexGuard};

/// These tests share one compositor and each reloads to undo itself, so
/// two running at once would restore each other's values mid-assertion.
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

/// Every catalog key exists on the running compositor with the type this
/// crate claims. This is what caught `decoration:wobble` not existing.
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

/// The categories this crate declares unsupported must actually be
/// unsupportable — if `decoration:wobble` ever ships, the note telling
/// users it doesn't exist becomes the wrong answer.
#[test]
#[ignore]
fn declared_unsupported_categories_are_still_absent_or_still_untyped() {
    let _live = compositor();
    let raw = hyprctl(&["getoption", "decoration:wobble:enabled", "-j"]);
    assert!(
        !raw.trim_start().starts_with('{'),
        "decoration:wobble exists now — the catalog should carry it instead of \
         calling it unreleased: {raw}"
    );
}

/// The generated file loads cleanly with every option set at once.
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

    eval_and_assert_clean(&generate(&Appearance {
        settings,
        animations: Animations::default(),
    }));
    reload();
}

/// Colours are the type most likely to be written in a form Hyprland
/// refuses, and a refused value takes the whole file with it.
#[test]
#[ignore]
fn hyprland_accepts_the_colour_form_this_crate_writes() {
    let _live = compositor();
    reload();
    assert!(config_errors().is_empty(), "config already has errors");

    let mut settings = Settings::default();
    settings.set("general:col:active_border", Value::Text("rgba(bd93f9ff)".into()));
    settings.set("decoration:shadow:color", Value::Text("rgb(1a1a1a)".into()));
    eval_and_assert_clean(&generate(&Appearance {
        settings,
        animations: Animations::default(),
    }));

    // And it round-trips: what comes back converts to what went in.
    let live = getoption("general:col:active_border")["gradient"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(
        hyprforge_core::hlconfig::import::read_gradient(&live),
        Some(Value::Text("rgba(bd93f9ff)".into())),
        "live gradient {live} didn't convert back"
    );
    reload();
}

/// A gap written as an integer takes effect, and reads back as the four
/// numbers `read_gaps` collapses. The wiki calls this type `css_gaps` and
/// implies a string; the compositor refuses one.
#[test]
#[ignore]
fn a_gap_written_as_an_integer_takes_effect_and_reads_back() {
    let _live = compositor();
    reload();

    let mut settings = Settings::default();
    settings.set("general:gaps_in", Value::Int(7));
    eval_and_assert_clean(&generate(&Appearance {
        settings,
        animations: Animations::default(),
    }));

    let live = getoption("general:gaps_in")["css"].as_str().unwrap().to_string();
    assert_eq!(
        hyprforge_core::hlconfig::import::read_gaps(&live),
        Some(Value::Int(7)),
        "live gaps {live} didn't convert back"
    );
    reload();
}

/// An animation this crate writes actually reaches the compositor, and a
/// later call for the same leaf wins — the fact that lets animations share
/// the settings' placement instead of needing their own.
#[test]
#[ignore]
fn a_later_animation_call_overrides_the_same_leaf() {
    let _live = compositor();
    reload();

    let write = |speed: f64, bezier: &str| {
        let mut animations = Animations::default();
        animations.set(
            "windows",
            Animation {
                enabled: true,
                speed,
                bezier: bezier.to_string(),
                style: String::new(),
            },
        );
        eval_and_assert_clean(&generate(&Appearance {
            settings: Settings::default(),
            animations,
        }));
    };

    write(4.79, "easeOutQuint");
    assert_eq!(live_speed("windows"), Some(4.79));
    write(9.99, "linear");
    assert_eq!(live_speed("windows"), Some(9.99), "the later call must win");

    reload();
}

/// The leaf names and curve names this crate offers come from the
/// compositor rather than a hardcoded list, so this checks the parsing
/// against the real thing.
#[test]
#[ignore]
fn live_animations_and_curves_parse() {
    let _live = compositor();
    reload();
    let (leaves, curves) = animations::live().expect("hyprctl animations");
    assert!(leaves.len() > 20, "expected the full leaf list, got {}", leaves.len());
    assert!(
        leaves.iter().all(|l| !l.leaf.starts_with("__internal")),
        "internal leaves must be filtered out"
    );
    assert!(
        curves.iter().any(|c| c.name == "default"),
        "the built-in curve should always exist"
    );
}

fn live_speed(leaf: &str) -> Option<f64> {
    let (leaves, _) = animations::parse_live(&hyprctl(&["animations", "-j"]));
    leaves.iter().find(|l| l.leaf == leaf).map(|l| l.animation.speed)
}

/// A value the catalog declares within range that Hyprland refuses would
/// be a range this crate got wrong.
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
        eval_and_assert_clean(&generate(&Appearance {
            settings,
            animations: Animations::default(),
        }));
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
        (Kind::Int { min, .. } | Kind::Gaps { min, .. }, Extreme::Min) => min.map(Value::Int),
        (Kind::Int { max, .. } | Kind::Gaps { max, .. }, Extreme::Max) => max.map(Value::Int),
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
    let dir = std::env::temp_dir().join("hyprforge-live-appearance");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("probe.lua");
    std::fs::write(&path, lua).unwrap();

    let out = hyprctl(&["eval", &format!("dofile([[{}]])", path.display())]);
    let errors = config_errors();
    let _ = std::fs::remove_file(&path);

    // A failing probe panics, which would skip the caller's reload and
    // leave the session running whatever this just set. Undo first, then
    // report.
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
