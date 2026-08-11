//! Manual test that a real Hyprland accepts what the codegen produces. Not
//! run by default (`cargo test`) — only via `cargo test --test live_lua --
//! --ignored`, and only on a machine actually running Hyprland.
//!
//! This is the only test that can catch a field name we invented. Everything
//! else in this crate asserts on strings we generate ourselves, which cannot
//! tell the difference between a correct rule and a plausible-looking one the
//! compositor silently ignores — the exact failure mode that made the display
//! scale bug take so long to find.
//!
//! It is deliberately read-only with respect to the user's config: it writes
//! only to a temp dir, sources that file into the *running* compositor via
//! `hyprctl keyword`-free means (see below), and never edits `hyprland.lua`.

use hyprforge_windowrules::codegen::generate;
use hyprforge_windowrules::model::{Effects, Matcher, Opacity, Rule, Workspace};
use std::process::Command;

/// Every field the codegen can emit, in one rule set. Add to this whenever a
/// field is added to the model — a field absent here is a field this test
/// can't vouch for.
fn every_supported_field() -> Vec<Rule> {
    vec![
        Rule {
            name: "hyprforge-live-matchers".to_string(),
            enabled: true,
            matcher: Matcher {
                class: Some("hyprforge-live-probe".to_string()),
                title: Some("^probe$".to_string()),
                initial_class: Some("probe".to_string()),
                initial_title: Some("starting".to_string()),
                fullscreen: Some(false),
                floating: Some(true),
                xwayland: Some(false),
                tag: Some("probe-tag".to_string()),
                content: Some("game".to_string()),
            },
            effects: Effects::default(),
        },
        Rule {
            name: "hyprforge-live-effects".to_string(),
            enabled: true,
            matcher: Matcher {
                class: Some("hyprforge-live-probe".to_string()),
                ..Default::default()
            },
            effects: Effects {
                workspace: Workspace { name: "9".to_string(), silent: true },
                tag: Some("+probe-tag".to_string()),
                float: Some(true),
                r#move: Some(["cursor_x-(window_w*0.5)".to_string(), "40".to_string()]),
                size: Some(["60%".to_string(), "480".to_string()]),
                opacity: Opacity {
                    active: Some(0.9),
                    inactive: Some(0.7),
                    fullscreen: Some(1.0),
                    is_override: true,
                },
                border_color: Some("rgb(FF0000)".to_string()),
                no_blur: Some(true),
                rounding: Some(8),
            },
        },
    ]
}

fn config_errors() -> String {
    let out = Command::new("hyprctl")
        .arg("configerrors")
        .output()
        .expect("hyprctl not runnable — is Hyprland running?");
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// Loads a generated file into the running compositor and asks Hyprland
/// whether it had any complaint.
///
/// `hyprctl configerrors` is the compositor's own answer, the same source its
/// error overlay draws on — which is what makes this meaningful where a
/// string assertion isn't.
#[test]
#[ignore]
fn hyprland_accepts_every_field_the_codegen_emits() {
    // Read once: a second call can return something different, and a
    // failure message that re-queries can end up printing nothing at all.
    let preexisting = config_errors();
    assert!(
        preexisting.is_empty(),
        "the config already has errors before this test ran, so nothing here \
         could be attributed to the generated file. `hyprctl reload` clears \
         them:\n{preexisting}"
    );

    let dir = std::env::temp_dir().join("hyprforge-live-lua");
    std::fs::create_dir_all(&dir).expect("could not create temp dir");
    let path = dir.join("probe.lua");
    let lua = generate(&every_supported_field());
    std::fs::write(&path, &lua).expect("could not write probe file");
    eprintln!("--- generated ---\n{lua}");

    // `hyprctl eval` runs a Lua string in the live config context, so the
    // rules are evaluated exactly as a require()d file would be, without
    // touching hyprland.lua. They're gone at the next reload.
    let out = Command::new("hyprctl")
        .arg("eval")
        .arg(format!("dofile({})", lua_quote(&path.display().to_string())))
        .output()
        .expect("hyprctl not runnable");
    let body = String::from_utf8_lossy(&out.stdout);
    let body = body.trim();
    eprintln!("eval: {body}");
    // eval reports a Lua failure in the body, not the exit status — the same
    // asymmetry apply() works around.
    assert!(
        !body.to_lowercase().contains("error"),
        "Hyprland could not evaluate the generated rules: {body}"
    );

    let errors = config_errors();
    assert!(errors.is_empty(), "Hyprland rejected the generated rules:\n{errors}");

    let _ = std::fs::remove_file(&path);
}

fn lua_quote(s: &str) -> String {
    format!("[[{s}]]")
}
