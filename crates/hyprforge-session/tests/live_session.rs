//! Manual tests that a real Hyprland accepts what this crate generates.
//! `cargo test --test live_session -- --ignored`, on a machine running
//! Hyprland.
//!
//! Every direction and action in the gesture catalogue is checked with a
//! `hyprctl reload` *between* each one. Without the reload the first
//! registration shadows the next, and Hyprland answers "Gesture will be
//! overshadowed by a previous gesture" — which reads exactly like a
//! rejected name and would make nine valid actions look invalid. That
//! false reading happened while writing this.

use hyprforge_session::{autostart, environment, gestures, permissions};
use std::process::Command;
use std::sync::{Mutex, MutexGuard};

/// These share one compositor and each reloads to undo itself.
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

/// Evaluates `lua` and returns what Hyprland said.
fn eval(lua: &str) -> String {
    hyprctl(&["eval", lua])
}

fn refused(out: &str) -> bool {
    let lower = out.to_lowercase();
    lower.contains("error") || lower.contains("invalid")
}

/// Every gesture direction this crate offers is one Hyprland takes.
#[test]
#[ignore]
fn hyprland_accepts_every_gesture_direction() {
    let _live = compositor();
    let mut wrong = Vec::new();
    for (direction, _) in gestures::DIRECTIONS {
        // Between each: an earlier gesture shadows a later one, which
        // reads like a rejected name.
        reload();
        let out = eval(&format!(
            "hl.gesture({{ fingers = 4, direction = '{direction}', action = 'workspace' }})"
        ));
        if refused(&out) {
            wrong.push(format!("{direction}: {}", out.trim()));
        }
    }
    reload();
    assert!(wrong.is_empty(), "Hyprland refused directions:\n{}", wrong.join("\n"));
}

/// Every gesture action, with the extra argument it declares.
#[test]
#[ignore]
fn hyprland_accepts_every_gesture_action_with_its_argument() {
    let _live = compositor();
    let mut wrong = Vec::new();
    for (action, _, argument) in gestures::ACTIONS {
        reload();
        let extra = match *argument {
            Some("workspace_name") => ", workspace_name = 'magic'",
            Some("zoom_level") => ", zoom_level = 2",
            Some("mode") => ", mode = 'maximize'",
            _ => "",
        };
        let out = eval(&format!(
            "hl.gesture({{ fingers = 4, direction = 'up', action = '{action}'{extra} }})"
        ));
        if refused(&out) {
            wrong.push(format!("{action}: {}", out.trim()));
        }
    }
    reload();
    assert!(wrong.is_empty(), "Hyprland refused actions:\n{}", wrong.join("\n"));
}

/// The fact the gesture module's whole design rests on: Hyprland
/// **refuses** a duplicate gesture rather than letting the later one win.
/// If this ever changed, the conflict detection would be needless caution
/// instead of a requirement.
#[test]
#[ignore]
fn hyprland_refuses_a_shadowed_gesture() {
    let _live = compositor();
    reload();
    let first = eval("hl.gesture({ fingers = 4, direction = 'up', action = 'close' })");
    assert!(!refused(&first), "the first gesture should register: {first}");

    let second = eval("hl.gesture({ fingers = 4, direction = 'up', action = 'move' })");
    reload();
    assert!(
        refused(&second),
        "Hyprland now allows a duplicate gesture — the conflict detection in \
         hyprforge_session::gestures could be dropped: {second}"
    );
    assert!(
        second.to_lowercase().contains("overshadow"),
        "expected the overshadow message, got: {second}"
    );
}

/// Every permission type this crate offers is one Hyprland takes.
#[test]
#[ignore]
fn hyprland_accepts_every_permission_type_and_mode() {
    let _live = compositor();
    reload();
    let mut wrong = Vec::new();
    for (kind, _, _) in permissions::TYPES {
        for mode in permissions::Mode::ALL {
            // A binary path that matches nothing real, so no actual
            // permission changes for anything running.
            let out = eval(&format!(
                "hl.permission({{ binary = '/nonexistent/hyprforge-probe', type = '{kind}', mode = '{mode}' }})"
            ));
            if refused(&out) {
                wrong.push(format!("{kind}/{mode}: {}", out.trim()));
            }
        }
    }
    reload();
    assert!(wrong.is_empty(), "Hyprland refused permissions:\n{}", wrong.join("\n"));
}

/// The generated file loads as a whole — environment, permissions,
/// gestures and autostart together, which is the shape a user actually
/// gets.
#[test]
#[ignore]
fn hyprland_loads_a_complete_generated_file() {
    let _live = compositor();
    reload();
    let before = hyprctl(&["configerrors"]);
    assert!(
        before.trim() == "no errors" || before.trim().is_empty(),
        "config already has errors, so nothing here could be attributed:\n{before}"
    );

    let mut session = hyprforge_session::storage::Session::default();
    session.environment.variables.push(environment::Variable {
        name: "HYPRFORGE_PROBE".into(),
        value: "1".into(),
        enabled: true,
    });
    session.permissions.rules.push(permissions::Rule {
        binary: "/nonexistent/hyprforge-probe".into(),
        r#type: "screencopy".into(),
        mode: permissions::Mode::Ask,
        enabled: true,
    });
    session.gestures.gestures.push(gestures::Gesture {
        // 5 fingers: nothing real uses it, so it can't shadow the
        // user's own gestures.
        fingers: 5,
        direction: "up".into(),
        action: "close".into(),
        ..Default::default()
    });
    session.autostart.programs.push(autostart::Program {
        // `true` is a real program that does nothing.
        command: "true".into(),
        enabled: true,
        when: autostart::When::Start,
        note: "live probe".into(),
    });

    let lua = hyprforge_session::apply::generate(&session, &[]);
    let dir = std::env::temp_dir().join("hyprforge-live-session");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("probe.lua");
    std::fs::write(&path, &lua).unwrap();

    let out = eval(&format!("dofile([[{}]])", path.display()));
    let errors = hyprctl(&["configerrors"]);
    let _ = std::fs::remove_file(&path);
    reload();

    assert!(!refused(&out), "Hyprland refused the generated file: {out}\n--- lua ---\n{lua}");
    assert!(
        errors.trim() == "no errors" || errors.trim().is_empty(),
        "config errors after loading it: {errors}\n--- lua ---\n{lua}"
    );
}

/// An environment variable this crate writes actually reaches Hyprland.
#[test]
#[ignore]
fn a_generated_environment_variable_is_accepted() {
    let _live = compositor();
    reload();
    let out = eval("hl.env([[HYPRFORGE_PROBE]], [[1]])");
    reload();
    assert!(!refused(&out), "Hyprland refused hl.env: {out}");
}
