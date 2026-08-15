//! The daemons themselves parse what this crate generates.
//!
//! Not run by default — `cargo test --test generated_configs_parse --
//! --ignored`, on a machine with the daemons installed. They do **not**
//! need to be running: `--config` is read before anything else happens,
//! and hyprpaper's EGL initialisation fails harmlessly in a second
//! instance long after the config has been parsed.
//!
//! This closes the gap the other suites leave. `live_ecosystem.rs` checks
//! that `hyprctl` accepts the *requests* this crate sends, and the unit
//! tests check the generated text against this crate's own idea of the
//! format. Neither has ever asked the daemon whether it can actually read
//! the file — and a misspelled key is invisible until then, because:
//!
//! ```text
//! [ERR] Config has errors:
//! Config error … config option <listener:this_is_not_a_key> does not exist.
//! Proceeding ignoring faulty entries
//! ```
//!
//! **It exits 0.** A wrong key isn't a failure to the daemon; it is
//! silently dropped, and the setting simply never happens. So the exit
//! status is useless and the marker in the output is the only signal —
//! which is exactly the "silently does nothing" class this project keeps
//! finding.

use hyprforge_ecosystem::{idle, wallpaper};
use std::process::Command;

fn is_running(daemon: &str) -> bool {
    Command::new("pgrep")
        .args(["-x", daemon])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// What hyprlang prints when it rejects a **key or its syntax**. Matched
/// on the message rather than the exit code, which is 0 either way.
///
/// Deliberately narrower than "did it print `Config has errors`". hypridle
/// answers a config with no listeners in it with:
///
/// ```text
/// [ERR] Config has errors:
/// No rules configured
/// ```
///
/// which is a complaint about the config as a whole, not about anything
/// this crate wrote — and it is only reachable by removing every listener,
/// where hypridle having nothing to do is the correct outcome. Treating it
/// as a failure would make the empty-file case unfixable without writing a
/// listener nobody asked for.
const ERROR_MARKERS: &[&str] = &["does not exist", "config error in file"];

struct Parsed {
    output: String,
}

impl Parsed {
    fn complained(&self) -> bool {
        let lower = self.output.to_lowercase();
        ERROR_MARKERS.iter().any(|m| lower.contains(m))
    }
}

/// Runs `daemon --config <path>` briefly and captures what it said.
///
/// `None` when the daemon isn't installed **or is already running**, so
/// the suite skips rather than fails.
///
/// The running check is not caution, it is required. A second hyprpaper
/// takes over the IPC socket, and when it exits the socket is gone —
/// leaving the original process alive but unreachable, so every
/// `hyprctl hyprpaper` afterwards answers "failed to connect". That
/// happened once here and needed a manual restart to undo.
fn parse_with(daemon: &str, contents: &str) -> Option<Parsed> {
    if !Command::new("which").arg(daemon).output().ok()?.status.success() {
        return None;
    }
    // Only hyprpaper. A second hypridle just tracks idle on its own for
    // the two seconds it lives, with a timeout far too long to fire.
    if daemon == "hyprpaper" && is_running(daemon) {
        eprintln!(
            "hyprpaper is running — skipping. A second instance takes over its IPC \
             socket, and when it exits the socket is gone, leaving the original \
             alive but unreachable."
        );
        return None;
    }
    let dir = tempfile::tempdir().ok()?;
    let path = dir.path().join("generated.conf");
    std::fs::write(&path, contents).ok()?;

    // `timeout` rather than a manual kill: these are daemons and will
    // otherwise sit there. One second is far longer than parsing takes.
    let out = Command::new("timeout")
        .arg("2")
        .arg(daemon)
        .arg("--config")
        .arg(&path)
        .output()
        .ok()?;
    Some(Parsed {
        output: format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        ),
    })
}

/// Every wallpaper option this crate can write, in one file.
fn maximal_wallpapers() -> wallpaper::Settings {
    let mut settings = wallpaper::Settings {
        splash: Some(false),
        splash_offset: Some(20.0),
        splash_opacity: Some(0.8),
        ..Default::default()
    };
    // A real directory, so the cycling options are actually emitted —
    // they're skipped for a single image, and a test that didn't emit
    // them wouldn't be checking them.
    settings.entries.push(wallpaper::Entry {
        monitor: String::new(),
        path: std::env::temp_dir().to_string_lossy().to_string(),
        fit_mode: wallpaper::FitMode::Contain,
        timeout: Some(60),
        random_order: true,
        recursive: true,
    });
    settings
}

/// Every idle option this crate can write, in one file.
///
/// Commands are all `true` — a real program that does nothing — so a
/// stray execution during the parse can't have an effect.
fn maximal_idle() -> idle::Settings {
    let mut settings = idle::Settings::default();
    settings.general.lock_cmd = "true".into();
    settings.general.unlock_cmd = "true".into();
    settings.general.before_sleep_cmd = "true".into();
    settings.general.after_sleep_cmd = "true".into();
    settings.general.on_lock_cmd = "true".into();
    settings.general.on_unlock_cmd = "true".into();
    settings.general.ignore_dbus_inhibit = true;
    settings.general.ignore_systemd_inhibit = true;
    settings.general.ignore_wayland_inhibit = true;
    settings.listeners.push(idle::Listener {
        // Long enough that it cannot fire during the test.
        timeout: 99_999,
        on_timeout: "true".into(),
        on_resume: "true".into(),
        ignore_inhibit: true,
    });
    settings
}

#[test]
#[ignore]
fn hyprpaper_parses_every_wallpaper_option_this_crate_writes() {
    let contents = wallpaper::generate(&maximal_wallpapers());
    let Some(parsed) = parse_with("hyprpaper", &contents) else {
        // `parse_with` has already said why.
        return;
    };
    assert!(
        !parsed.complained(),
        "hyprpaper rejected the generated config:\n{}\n--- config ---\n{contents}",
        parsed.output
    );
}

#[test]
#[ignore]
fn hypridle_parses_every_idle_option_this_crate_writes() {
    let contents = idle::generate(&maximal_idle());
    let Some(parsed) = parse_with("hypridle", &contents) else {
        // `parse_with` has already said why.
        return;
    };
    assert!(
        !parsed.complained(),
        "hypridle rejected the generated config:\n{}\n--- config ---\n{contents}",
        parsed.output
    );
}

/// The negative control, and the reason the two tests above mean
/// anything.
///
/// A validation test that cannot fail is worthless: if a daemon stopped
/// reporting config errors, or the markers changed wording, the checks
/// above would pass forever while catching nothing. This feeds each
/// daemon a key that definitely doesn't exist and insists it complains.
#[test]
#[ignore]
fn a_misspelled_key_is_actually_detected() {
    let cases = [
        (
            "hypridle",
            "listener {\n    timeout = 99999\n    definitely_not_a_key = true\n}\n",
        ),
        (
            "hyprpaper",
            "wallpaper {\n    monitor =\n    definitely_not_a_key = true\n}\n",
        ),
    ];
    let mut checked = 0;
    for (daemon, contents) in cases {
        let Some(parsed) = parse_with(daemon, contents) else {
            continue;
        };
        assert!(
            parsed.complained(),
            "{daemon} accepted a key that doesn't exist, so the parse checks \
             above can no longer fail and are proving nothing:\n{}",
            parsed.output
        );
        checked += 1;
    }
    // hyprpaper is skipped whenever it's running, so this can legitimately
    // check only hypridle. Zero is still a failure: it would mean the
    // suite is proving nothing at all.
    assert!(
        checked > 0,
        "no daemon was available, so the parse checks are unverified — stop \
         hyprpaper, or install the daemons, and run this again"
    );
}

/// The empty file every module writes at setup, before anything is
/// configured, has to parse too — it is what the user's config sources
/// from the moment the screen is first opened.
#[test]
#[ignore]
fn the_empty_generated_files_parse() {
    for (daemon, contents) in [
        ("hyprpaper", wallpaper::generate(&wallpaper::Settings::default())),
        ("hypridle", idle::generate(&idle::Settings::default())),
    ] {
        let Some(parsed) = parse_with(daemon, &contents) else {
            continue;
        };
        assert!(
            !parsed.complained(),
            "{daemon} rejected the empty generated config:\n{}\n--- config ---\n{contents}",
            parsed.output
        );
    }
}
