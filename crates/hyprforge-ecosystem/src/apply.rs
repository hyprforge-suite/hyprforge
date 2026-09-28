//! Writing a generated config and getting the daemon to notice.
//!
//! The write half is identical for all four: render, make sure the
//! user's own config sources it, write atomically. The *notice* half is
//! not, and pretending otherwise would be the "reports a save that did
//! nothing" failure this project keeps meeting:
//!
//! | Daemon | How a change lands |
//! |---|---|
//! | hyprpaper | `hyprctl hyprpaper wallpaper` — immediate |
//! | hyprsunset | `hyprctl hyprsunset temperature` — immediate |
//! | hypridle | no IPC at all; must be restarted |
//! | xdg-desktop-portal-hyprland | read at startup; restarted only on request |
//!
//! So [`Applied`] reports what actually happened rather than returning
//! `()`, and the caller shows the difference.

use crate::{idle, portal, sunset, wallpaper};
use hyprforge_core::hyprlang;
use std::path::Path;
use std::process::Command;

#[derive(Debug, thiserror::Error)]
pub enum ApplyError {
    #[error("couldn't write {path}: {source}")]
    Write {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("couldn't update {path}: {source}")]
    Setup {
        path: String,
        #[source]
        source: hyprforge_core::lua_setup::SetupError,
    },
}

/// What reached the running daemon, if anything.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Applied {
    /// Saved and live.
    Live,
    /// Saved, and the daemon refused some of it. Carries what was
    /// rejected — a push that failed while the app said "applied" is
    /// exactly the silent-no-op this project keeps finding.
    PartlyRefused(Vec<String>),
    /// Saved, but the daemon won't pick it up until it restarts.
    NeedsRestart,
    /// Saved; the daemon isn't running, so there was nothing to tell.
    DaemonNotRunning,
    /// Saved, but whether the daemon is running could not be established —
    /// `pgrep` didn't start, or didn't answer inside the timeout.
    ///
    /// Deliberately not folded into [`Applied::DaemonNotRunning`]. "There
    /// was nothing to tell" and "we couldn't find out whether there was"
    /// are different facts, and reporting the second as the first tells the
    /// user their change had nothing to reach when it may have had a daemon
    /// sitting right there, ignoring the settings they just saved.
    DaemonUnknown,
}

/// Writes `contents` to `generated` and makes sure `target` sources it.
fn write_and_source(generated: &Path, target: &Path, contents: &str) -> Result<(), ApplyError> {
    hyprforge_core::paths::write_atomic(generated, contents).map_err(|source| ApplyError::Write {
        path: generated.display().to_string(),
        source,
    })?;
    // After the generated file exists, never before: a `source =` line
    // pointing at a missing file is an error the daemon reports on its
    // next read, and the ordering is the same rule the Lua modules follow.
    hyprlang::install(target, &hyprlang::source_line(generated)).map_err(|source| {
        ApplyError::Setup {
            path: target.display().to_string(),
            source,
        }
    })?;
    Ok(())
}

/// Whether `name` is running: `Some(false)` for "checked, it isn't",
/// `None` for "couldn't check".
///
/// `pgrep -x` exits 1 when it finds nothing, which is an answer. An `Err`
/// from `command::output` is not — it means the check itself failed to
/// start or timed out, and collapsing that into `false` is the same
/// mistake as reading an unparseable config file as an empty one.
fn running(name: &str) -> Option<bool> {
    hyprforge_core::command::output(
        Command::new("pgrep").args(["-x", name]),
        hyprforge_core::command::TIMEOUT,
    )
    .map(|o| o.status.success())
    .ok()
}

fn hyprctl(args: &[&str]) -> bool {
    hyprforge_core::command::output(
        Command::new("hyprctl").args(args),
        hyprforge_core::command::TIMEOUT,
    )
        .map(|o| {
            let body = String::from_utf8_lossy(&o.stdout).to_lowercase();
            o.status.success() && !body.contains("error") && !body.contains("invalid")
        })
        .unwrap_or(false)
}

/// Writes the wallpaper config and pushes every entry to hyprpaper.
///
/// Each entry is pushed individually because that is the only request
/// hyprpaper 0.8.4 takes — there is no "reload the config" — so the live
/// state is built by replaying what was just written.
pub fn wallpapers(
    generated: &Path,
    target: &Path,
    settings: &wallpaper::Settings,
) -> Result<Applied, ApplyError> {
    write_and_source(generated, target, &wallpaper::generate(settings))?;
    match running("hyprpaper") {
        Some(true) => {}
        Some(false) => return Ok(Applied::DaemonNotRunning),
        None => return Ok(Applied::DaemonUnknown),
    }
    let skip: Vec<usize> = settings.invalid().into_iter().map(|(i, _)| i).collect();
    let mut refused = Vec::new();
    for (i, entry) in settings.entries.iter().enumerate() {
        if skip.contains(&i) {
            continue;
        }
        let request = format!(
            "{},{},{}",
            entry.monitor.trim(),
            wallpaper::expand_tilde(&entry.path),
            entry.fit_mode
        );
        // Checked, not fired and forgotten: hyprpaper refuses an image it
        // can't read, and reporting "applied" for a wallpaper that never
        // changed is the failure this project keeps meeting.
        if !hyprctl(&["hyprpaper", "wallpaper", &request]) {
            refused.push(if entry.is_fallback() {
                entry.path.clone()
            } else {
                format!("{} on {}", entry.path, entry.monitor.trim())
            });
        }
    }
    if refused.is_empty() {
        Ok(Applied::Live)
    } else {
        Ok(Applied::PartlyRefused(refused))
    }
}

/// Writes the temperature schedule and pushes the profile that should be
/// active right now.
///
/// `now_minutes` is passed in rather than read from the clock so the
/// choice is testable — picking the wrong profile means the screen
/// changes colour to something the schedule doesn't call for.
pub fn temperature(
    generated: &Path,
    target: &Path,
    settings: &sunset::Settings,
    now_minutes: u32,
) -> Result<Applied, ApplyError> {
    write_and_source(generated, target, &sunset::generate(settings))?;
    match running("hyprsunset") {
        Some(true) => {}
        Some(false) => return Ok(Applied::DaemonNotRunning),
        None => return Ok(Applied::DaemonUnknown),
    }
    let mut refused = Vec::new();
    if let Some(profile) = active_profile(settings, now_minutes) {
        let ok = if profile.identity {
            hyprctl(&["hyprsunset", "identity"])
        } else {
            hyprctl(&["hyprsunset", "temperature", &profile.temperature.to_string()])
        };
        if !ok {
            refused.push(format!("{}K at {}", profile.temperature, profile.time));
        }
        // Gamma is a percentage over IPC and a multiplier in the config.
        if !hyprctl(&["hyprsunset", "gamma", &format!("{}", (profile.gamma * 100.0).round())]) {
            refused.push(format!("brightness {}", profile.gamma));
        }
    }
    if refused.is_empty() {
        Ok(Applied::Live)
    } else {
        Ok(Applied::PartlyRefused(refused))
    }
}

/// The profile hyprsunset would be holding at `now_minutes`.
///
/// The one whose time has most recently passed — and if none has (every
/// profile is later today), the **last** one, because the schedule wraps
/// around midnight. Getting that wrap wrong leaves the morning showing
/// the daytime profile when the user only configured an evening one.
pub fn active_profile(settings: &sunset::Settings, now_minutes: u32) -> Option<&sunset::Profile> {
    // `written` and not `profiles`: a profile the generator refused is not
    // part of the schedule hyprsunset was given, so pushing it live would
    // apply something no file describes and nothing can explain.
    let mut timed: Vec<(u32, &sunset::Profile)> = settings
        .written()
        .into_iter()
        .filter_map(|p| Some((sunset::parse_time(&p.time)?, p)))
        .collect();
    timed.sort_by_key(|(minutes, _)| *minutes);
    timed
        .iter()
        .rev()
        .find(|(minutes, _)| *minutes <= now_minutes)
        .or_else(|| timed.last())
        .map(|(_, p)| *p)
}

/// Writes the idle config.
///
/// Never restarts hypridle. It is the process that locks the session and
/// blanks the screen, and restarting it out from under a user — who might
/// be mid-sentence, or relying on it not to lock — is not something a
/// save button should do silently. The caller reports [`Applied::NeedsRestart`]
/// and lets them choose.
pub fn idle(
    generated: &Path,
    target: &Path,
    settings: &idle::Settings,
) -> Result<Applied, ApplyError> {
    write_and_source(generated, target, &idle::generate(settings))?;
    match running("hypridle") {
        Some(true) => {}
        Some(false) => return Ok(Applied::DaemonNotRunning),
        None => return Ok(Applied::DaemonUnknown),
    }
    Ok(Applied::NeedsRestart)
}

/// Restarts hypridle, on explicit request.
///
/// `pkill` then respawn, because it is started from `hl.exec_cmd` in the
/// user's Hyprland config rather than by systemd — there is no unit to
/// restart. Detached so it outlives the settings app.
pub fn restart_idle() -> Result<(), std::io::Error> {
    // Bounded like every other subprocess: a pkill that never returns
    // would hold up restarting the daemon it was clearing the way for.
    let _ = hyprforge_core::command::output(
        Command::new("pkill").args(["-x", "hypridle"]),
        hyprforge_core::command::TIMEOUT,
    );
    Command::new("hypridle")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()?;
    Ok(())
}

/// Writes the screen-sharing config.
///
/// Never restarts the portal. It reads `xdph.conf` only at startup, so a
/// change needs one — but restarting it drops any screen share in
/// progress, and a save button that can end someone's call mid-sentence
/// is not a trade this app makes for them. Same reasoning as
/// [`idle`](idle()): report [`Applied::NeedsRestart`] and let them choose.
pub fn portal(
    generated: &Path,
    target: &Path,
    settings: &portal::Settings,
) -> Result<Applied, ApplyError> {
    write_and_source(generated, target, &portal::generate(settings))?;
    // Unlike the other three this is a systemd user unit, so "running" is
    // a question for systemd rather than `pgrep`.
    match portal_running() {
        Some(true) => Ok(Applied::NeedsRestart),
        Some(false) => Ok(Applied::DaemonNotRunning),
        None => Ok(Applied::DaemonUnknown),
    }
}

/// Whether the portal unit is active: `Some(false)` for "checked, it
/// isn't", `None` for "couldn't check".
fn portal_running() -> Option<bool> {
    hyprforge_core::command::output(
        Command::new("systemctl")
            .args(["--user", "is-active", "--quiet", PORTAL_UNIT]),
        hyprforge_core::command::TIMEOUT,
    )
    .map(|o| o.status.success())
    .ok()
}

const PORTAL_UNIT: &str = "xdg-desktop-portal-hyprland.service";

/// Restarts the screen-sharing portal, on explicit request.
///
/// `systemctl --user restart`, not `pkill`: unlike hypridle this one is a
/// systemd user unit, and killing it would have the unit restart it under
/// systemd's own policy rather than ours.
pub fn restart_portal() -> Result<(), String> {
    let output = hyprforge_core::command::output(
        Command::new("systemctl").args(["--user", "restart", PORTAL_UNIT]),
        hyprforge_core::command::TIMEOUT,
    )
    .map_err(|e| e.to_string())?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stderr = stderr.trim();
    Err(if stderr.is_empty() {
        format!("systemctl couldn't restart {PORTAL_UNIT}")
    } else {
        stderr.to_string()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn schedule(times: &[(&str, i64)]) -> sunset::Settings {
        sunset::Settings {
            max_gamma: None,
            profiles: times
                .iter()
                .map(|(time, temperature)| sunset::Profile {
                    time: (*time).to_string(),
                    temperature: *temperature,
                    ..sunset::Profile::default()
                })
                .collect(),
        }
    }

    #[test]
    fn the_active_profile_is_the_most_recently_started_one() {
        let s = schedule(&[("00:00", 6500), ("12:00", 6000), ("21:00", 4000)]);
        assert_eq!(active_profile(&s, 0).unwrap().temperature, 6500);
        assert_eq!(active_profile(&s, 11 * 60).unwrap().temperature, 6500);
        assert_eq!(active_profile(&s, 12 * 60).unwrap().temperature, 6000);
        assert_eq!(active_profile(&s, 23 * 60).unwrap().temperature, 4000);
    }

    /// The schedule wraps: before the first profile of the day, the one
    /// still holding is the *last*. Getting this wrong shows the daytime
    /// profile all morning to someone who only configured an evening one.
    #[test]
    fn before_the_first_profile_the_last_one_is_still_holding() {
        let s = schedule(&[("08:00", 6500), ("21:00", 4000)]);
        assert_eq!(active_profile(&s, 0).unwrap().temperature, 4000);
        assert_eq!(active_profile(&s, 7 * 60).unwrap().temperature, 4000);
    }

    /// File order isn't schedule order — hyprsunset goes by time.
    #[test]
    fn profiles_out_of_order_still_resolve_by_time() {
        let s = schedule(&[("21:00", 4000), ("00:00", 6500), ("12:00", 6000)]);
        assert_eq!(active_profile(&s, 13 * 60).unwrap().temperature, 6000);
    }

    #[test]
    fn an_unparseable_time_is_left_out_of_the_schedule() {
        let s = schedule(&[("9:5", 3000), ("00:00", 6500)]);
        assert_eq!(active_profile(&s, 12 * 60).unwrap().temperature, 6500);
    }

    /// `gamma = 0` is rejected by `invalid()` because it blacks the screen
    /// out. It therefore never reaches the config file — but `temperature`
    /// also pushes the active profile to hyprsunset over IPC, and that path
    /// used to read `profiles` directly. So a profile too broken to write
    /// still turned the screen off, and stayed off until hyprsunset was
    /// restarted, with nothing on disk to explain it.
    #[test]
    fn a_profile_too_invalid_to_write_is_never_pushed_live() {
        let mut s = schedule(&[("00:00", 6500)]);
        s.profiles[0].gamma = 0.0;
        assert!(!s.invalid().is_empty(), "gamma 0 must be rejected to begin with");
        assert!(
            active_profile(&s, 12 * 60).is_none(),
            "a profile the generator refused must not be pushed over IPC"
        );
    }

    /// Dropping the invalid profile is not enough on its own: whichever
    /// valid profile it was shadowing has to take over, including across
    /// the midnight wrap. Otherwise fixing the black screen would leave the
    /// schedule showing the wrong profile instead.
    #[test]
    fn an_invalid_profile_does_not_shadow_the_valid_one_before_it() {
        let mut s = schedule(&[("06:00", 6500), ("20:00", 4000)]);
        s.profiles[1].temperature = 500; // below MIN_TEMPERATURE

        // 21:00 would be the evening profile, but it cannot be written.
        assert_eq!(active_profile(&s, 21 * 60).unwrap().temperature, 6500);
        // And before the first valid profile the day still wraps to the
        // last one that *is* valid, not to the rejected one.
        assert_eq!(active_profile(&s, 2 * 60).unwrap().temperature, 6500);
    }

    /// `pgrep -x` exiting 1 is an answer: nothing is running. `pgrep`
    /// failing to start, or timing out, is not — and reporting it as
    /// "the daemon isn't running" tells the user their change had nothing
    /// to reach when a daemon may have been sitting right there ignoring
    /// it. The two must stay distinguishable all the way to the screen.
    #[test]
    fn not_running_and_could_not_tell_are_different_answers() {
        assert_ne!(Applied::DaemonNotRunning, Applied::DaemonUnknown);
    }

    /// A process certain not to exist answers `Some(false)`, not `None`:
    /// the check ran and found nothing, which is knowledge.
    #[test]
    fn a_daemon_that_is_definitely_absent_reads_as_absent_not_unknown() {
        assert_eq!(running("hyprforge-no-such-daemon-xyz"), Some(false));
    }

    #[test]
    fn an_empty_schedule_has_no_active_profile() {
        assert!(active_profile(&sunset::Settings::default(), 600).is_none());
    }

    /// The generated file must exist before anything sources it — a
    /// `source =` line pointing at nothing is an error the daemon
    /// reports on its next read.
    #[test]
    fn writing_creates_the_file_before_sourcing_it() {
        let dir = tempfile::tempdir().unwrap();
        let generated = dir.path().join("gen/wallpaper.conf");
        let target = dir.path().join("hyprpaper.conf");
        std::fs::write(&target, "# mine\n").unwrap();

        write_and_source(&generated, &target, "# generated\n").unwrap();
        assert!(generated.exists());
        let conf = std::fs::read_to_string(&target).unwrap();
        assert!(conf.starts_with("# mine\n"), "{conf}");
        assert!(conf.contains(&format!("source = {}", generated.display())), "{conf}");
    }

    #[test]
    fn writing_twice_does_not_source_twice() {
        let dir = tempfile::tempdir().unwrap();
        let generated = dir.path().join("gen.conf");
        let target = dir.path().join("hyprpaper.conf");
        write_and_source(&generated, &target, "# a\n").unwrap();
        write_and_source(&generated, &target, "# b\n").unwrap();
        let conf = std::fs::read_to_string(&target).unwrap();
        assert_eq!(conf.matches("source =").count(), 1, "{conf}");
        assert_eq!(std::fs::read_to_string(&generated).unwrap(), "# b\n");
    }
}
