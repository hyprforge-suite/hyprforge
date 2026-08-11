//! Manual test that a real Hyprland accepts the binds we generate. Not run by
//! default — `cargo test --test live_lua -- --ignored`, on a machine running
//! Hyprland.
//!
//! Uses a chord nothing sane binds (SUPER+ALT+CTRL+SHIFT+F13) so evaluating
//! it can't shadow something the user relies on, and it's gone at the next
//! reload either way.
use hyprforge_shortcuts::codegen::generate;
use hyprforge_shortcuts::model::{Action, KeyCombo, Modifier, Shortcut};
use std::process::Command;

fn probe_shortcuts() -> Vec<Shortcut> {
    vec![
        Shortcut {
            name: "hyprforge-live-noarg".to_string(),
            enabled: true,
            combo: KeyCombo {
                mods: vec![Modifier::Super, Modifier::Alt, Modifier::Ctrl, Modifier::Shift],
                key: "F13".to_string(),
            },
            action: Action { dispatcher: "window.close".to_string(), argument: String::new() },
            description: "live probe, no argument".to_string(),
        },
        Shortcut {
            name: "hyprforge-live-arg".to_string(),
            enabled: true,
            combo: KeyCombo {
                mods: vec![Modifier::Super, Modifier::Alt, Modifier::Ctrl, Modifier::Shift],
                key: "F14".to_string(),
            },
            action: Action {
                dispatcher: "exec_cmd".to_string(),
                argument: "[[true]]".to_string(),
            },
            description: "live probe, with argument".to_string(),
        },
    ]
}

#[test]
#[ignore]
fn hyprland_accepts_the_generated_binds() {
    let errors_before = config_errors();
    assert!(
        errors_before.is_empty(),
        "config already has errors, so nothing here could be attributed:\n{errors_before}"
    );

    let dir = std::env::temp_dir().join("hyprforge-live-binds");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("probe.lua");
    let lua = generate(&probe_shortcuts());
    std::fs::write(&path, &lua).unwrap();
    eprintln!("--- generated ---\n{lua}");

    let out = Command::new("hyprctl")
        .arg("eval")
        .arg(format!("dofile([[{}]])", path.display()))
        .output()
        .expect("hyprctl not runnable");
    let body = String::from_utf8_lossy(&out.stdout);
    let body = body.trim();
    eprintln!("eval: {body}");
    assert!(!body.to_lowercase().contains("error"), "Hyprland rejected the binds: {body}");
    assert!(config_errors().is_empty(), "config errors after: {}", config_errors());

    // The description round-trips: what we wrote is what the compositor
    // reports, which is what lets the app find its own binds later.
    let listed = Command::new("hyprctl").arg("binds").arg("-j").output().unwrap();
    let listed = String::from_utf8_lossy(&listed.stdout);
    assert!(
        listed.contains("hyprforge: live probe, no argument"),
        "the description did not reach hyprctl binds"
    );

    let _ = std::fs::remove_file(&path);
}

fn config_errors() -> String {
    let out = Command::new("hyprctl").arg("configerrors").output().expect("hyprctl not runnable");
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}
