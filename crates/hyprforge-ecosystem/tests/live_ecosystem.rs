//! Manual tests that the real daemons accept what this crate generates.
//! Not run by default — `cargo test --test live_ecosystem -- --ignored`,
//! on a machine running them.
//!
//! Nothing here writes to the user's config files. Generated text is
//! checked against the daemons' own parsers by writing it to a temp file
//! and sourcing nothing, and live behaviour is exercised through the same
//! `hyprctl` requests the app uses.
//!
//! The tests that *change* a daemon — the wallpaper in every fit mode,
//! the screen temperature at both ends of its range — take the
//! disposable door (`hyprforge_testkit`), and `tools/hf test disposable`
//! starts a hyprpaper and a hyprsunset under the nested instance's
//! signature for them. Each instance has its own socket directory, so
//! those never touch the session's daemons. Run against the session,
//! the fit-mode test left the wallpaper on `cover` whatever it had been,
//! because hyprpaper has no way to report the mode it is using.

use hyprforge_ecosystem::{apply, sunset, wallpaper};
use hyprforge_testkit::{hyprctl, Hyprland};

/// Whether `daemon` answers on the targeted instance's own socket — not a
/// machine-wide `pgrep`, which sees the session's daemon while `hyprctl`
/// is talking to the nested instance. Says so in the marker `tools/hf`
/// and `tools/hf` report, because a test that returns early prints `ok`.
fn running(live: &Hyprland, daemon: &str) -> bool {
    let up = live.daemon_running(daemon);
    if !up {
        eprintln!("HYPRFORGE-SKIP: {daemon} isn't running on this instance");
    }
    up
}

/// The wallpaper request this crate builds is one hyprpaper accepts, in
/// every fit mode. A refused request is a wallpaper that silently doesn't
/// change.
#[test]
#[ignore]
fn hyprpaper_accepts_every_fit_mode_this_crate_writes() {
    let live = Hyprland::disposable();
    if !running(&live, "hyprpaper") {
        return;
    }
    // One line per monitor. Splitting the whole answer at its first colon
    // took every later line into the "path" — invisible on a one-monitor
    // session, and hyprpaper's "bad path" on the nested instance, which
    // has its window's output and a headless one.
    let active = hyprctl(&["hyprpaper", "listactive"]);
    let first = active.lines().find(|l| l.contains(':')).unwrap_or_default();
    let Some((monitor, path)) = first.split_once(':') else {
        eprintln!("HYPRFORGE-SKIP: no active wallpaper to probe with");
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
    // hyprpaper cannot say which mode it was in, so this is a guess —
    // harmless only because the instance is a disposable one.
    hyprctl(&["hyprpaper", "wallpaper", &format!("{monitor},{path},cover")]);
}

/// `listactive` is the shape this crate reads monitors and paths from,
/// and it is not in `hyprctl hyprpaper --help` — so a version that drops
/// it should fail here rather than silently reporting nothing.
#[test]
#[ignore]
fn hyprpaper_still_answers_listactive() {
    let live = Hyprland::read_only();
    if !running(&live, "hyprpaper") {
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
    let live = Hyprland::disposable();
    if !running(&live, "hyprsunset") {
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
    let live = Hyprland::disposable();
    if !running(&live, "hyprsunset") {
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
    let _live = Hyprland::read_only();
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

    // The path deliberately doesn't exist, so a running hyprpaper refuses
    // it — and that refusal has to be reported rather than dressed up as
    // success. This is the assertion that would have caught the app
    // saying "applied" for a wallpaper that never changed.
    let applied = apply::wallpapers(&generated, &target, &settings).unwrap();
    match applied {
        apply::Applied::PartlyRefused(refused) => {
            assert!(
                refused.iter().any(|r| r.contains("nonexistent")),
                "the refusal must name what was rejected: {refused:?}"
            );
        }
        apply::Applied::DaemonNotRunning => {}
        other => panic!("hyprpaper took a path that doesn't exist: {other:?}"),
    }

    let text = std::fs::read_to_string(&generated).unwrap();
    assert!(text.contains("wallpaper {"), "{text}");
    assert!(text.contains("fit_mode = contain"), "{text}");
    assert!(text.contains("splash = false"), "{text}");
    assert!(
        std::fs::read_to_string(&target).unwrap().contains("source ="),
        "the target must source the generated file"
    );
}
