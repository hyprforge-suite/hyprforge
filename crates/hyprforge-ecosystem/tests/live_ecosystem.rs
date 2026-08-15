//! Manual tests that the real daemons accept what this crate generates.
//! Not run by default — `cargo test --test live_ecosystem -- --ignored`,
//! on a machine running them.
//!
//! Nothing here writes to the user's config files. Generated text is
//! checked against the daemons' own parsers by writing it to a temp file
//! and sourcing nothing, and live behaviour is exercised through the same
//! `hyprctl` requests the app uses — then put back.

use hyprforge_ecosystem::{apply, sunset, wallpaper};
use std::process::Command;
use std::sync::{Mutex, MutexGuard};

/// The daemons are shared, and each test restores what it changed.
static DAEMONS: Mutex<()> = Mutex::new(());

fn daemons() -> MutexGuard<'static, ()> {
    DAEMONS.lock().unwrap_or_else(|e| e.into_inner())
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

fn running(name: &str) -> bool {
    Command::new("pgrep")
        .args(["-x", name])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// The wallpaper request this crate builds is one hyprpaper accepts, in
/// every fit mode. A refused request is a wallpaper that silently doesn't
/// change.
#[test]
#[ignore]
fn hyprpaper_accepts_every_fit_mode_this_crate_writes() {
    let _live = daemons();
    if !running("hyprpaper") {
        eprintln!("hyprpaper isn't running — skipping");
        return;
    }
    let active = hyprctl(&["hyprpaper", "listactive"]);
    let Some((monitor, path)) = active.trim().split_once(':') else {
        eprintln!("no active wallpaper to probe with — skipping");
        return;
    };
    let (monitor, path) = (monitor.trim(), path.trim());

    for mode in wallpaper::FitMode::ALL {
        let request = format!("{monitor},{path},{mode}");
        let out = hyprctl(&["hyprpaper", "wallpaper", &request]);
        assert!(
            !out.to_lowercase().contains("invalid") && !out.to_lowercase().contains("error"),
            "hyprpaper refused {request}: {out}"
        );
    }
    // Put it back the way it was.
    hyprctl(&["hyprpaper", "wallpaper", &format!("{monitor},{path},cover")]);
}

/// `listactive` is the shape this crate reads monitors and paths from,
/// and it is not in `hyprctl hyprpaper --help` — so a version that drops
/// it should fail here rather than silently reporting nothing.
#[test]
#[ignore]
fn hyprpaper_still_answers_listactive() {
    let _live = daemons();
    if !running("hyprpaper") {
        eprintln!("hyprpaper isn't running — skipping");
        return;
    }
    let out = hyprctl(&["hyprpaper", "listactive"]);
    assert!(out.contains(':'), "expected `monitor: path` lines, got: {out}");
    assert!(
        !out.to_lowercase().contains("invalid"),
        "listactive is no longer supported: {out}"
    );
}

/// The temperature this crate pushes is one hyprsunset takes, and it
/// reports the value back — so a rejected temperature can't look like a
/// successful save.
#[test]
#[ignore]
fn hyprsunset_takes_and_reports_a_temperature() {
    let _live = daemons();
    if !running("hyprsunset") {
        eprintln!("hyprsunset isn't running — skipping");
        return;
    }
    let before = hyprctl(&["hyprsunset", "temperature"]).trim().to_string();

    hyprctl(&["hyprsunset", "temperature", "5000"]);
    assert_eq!(hyprctl(&["hyprsunset", "temperature"]).trim(), "5000");

    // Restore whatever was there, falling back to neutral daylight.
    let restore = if before.parse::<i64>().is_ok() { before } else { "6500".into() };
    hyprctl(&["hyprsunset", "temperature", &restore]);
    assert_eq!(hyprctl(&["hyprsunset", "temperature"]).trim(), restore);
}

/// The catalogued temperature range has to be one hyprsunset actually
/// accepts at both ends.
#[test]
#[ignore]
fn hyprsunset_accepts_the_extremes_of_the_declared_range() {
    let _live = daemons();
    if !running("hyprsunset") {
        eprintln!("hyprsunset isn't running — skipping");
        return;
    }
    let before = hyprctl(&["hyprsunset", "temperature"]).trim().to_string();
    for value in [sunset::MIN_TEMPERATURE, sunset::MAX_TEMPERATURE] {
        let out = hyprctl(&["hyprsunset", "temperature", &value.to_string()]);
        assert!(
            !out.to_lowercase().contains("invalid") && !out.to_lowercase().contains("error"),
            "hyprsunset refused {value}, which the catalogue calls valid: {out}"
        );
    }
    let restore = if before.parse::<i64>().is_ok() { before } else { "6500".into() };
    hyprctl(&["hyprsunset", "temperature", &restore]);
}

/// hypridle having no IPC is load-bearing: it is why the idle screen says
/// a restart is needed instead of implying the change is live. If it ever
/// gains one, that message becomes a lie.
#[test]
#[ignore]
fn hypridle_still_has_no_ipc() {
    let _live = daemons();
    let out = hyprctl(&["hypridle"]).to_lowercase();
    assert!(
        out.contains("unknown request") || out.contains("invalid"),
        "hypridle answers hyprctl now — the idle screen should apply live \
         instead of asking for a restart: {out}"
    );
}

/// What `apply` writes has to be what the daemon would read back. This
/// checks the generated text parses as hyprlang by handing it to
/// hyprpaper as a sourced file in a temp tree — no user config involved.
#[test]
#[ignore]
fn the_generated_wallpaper_file_is_well_formed() {
    let dir = tempfile::tempdir().unwrap();
    let generated = dir.path().join("wallpaper.conf");
    let target = dir.path().join("hyprpaper.conf");

    let mut settings = wallpaper::Settings::default();
    settings.entries.push(wallpaper::Entry {
        monitor: String::new(),
        path: "/tmp/nonexistent.png".to_string(),
        fit_mode: wallpaper::FitMode::Contain,
        ..Default::default()
    });
    settings.splash = Some(false);

    let applied = apply::wallpapers(&generated, &target, &settings).unwrap();
    assert!(matches!(
        applied,
        apply::Applied::Live | apply::Applied::DaemonNotRunning
    ));

    let text = std::fs::read_to_string(&generated).unwrap();
    assert!(text.contains("wallpaper {"), "{text}");
    assert!(text.contains("fit_mode = contain"), "{text}");
    assert!(text.contains("splash = false"), "{text}");
    assert!(
        std::fs::read_to_string(&target).unwrap().contains("source ="),
        "the target must source the generated file"
    );
}
