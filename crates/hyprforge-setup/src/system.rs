//! Everything setup asks of another process, behind one seam.
//!
//! The trait came before the real implementation, the order CLAUDE.md
//! asks of a D-Bus-backed module, and for the same reason: the machine
//! running the tests has no guaranteed systemd user instance, no Hyprland
//! to `reload` and no session bus — and must never have its real services
//! enabled or its real binds reloaded by a test. `mock::MockSystem` (feature `mock`) stands in
//! for all of it.
//!
//! Files are not behind the seam. They go through [`crate::Env`]'s paths,
//! which a test points at a temp directory; only the things a temp
//! directory cannot fake are here.
//!
//! Every method of [`RealSystem`] that runs a program is bounded by
//! [`hyprforge_process::TIMEOUT`]. A `systemctl` that never answers is the
//! same problem, from here, as one that is not installed.

use hyprforge_shortcuts::binds::LiveBind;
use std::path::{Path, PathBuf};
use std::process::Command;

/// What `systemctl --user is-enabled` says about a unit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UnitState {
    /// Linked to start with the session — per-user, *or* for every user
    /// by a package's `systemctl --global preset`. The two read the same
    /// here and both count as done; only how to turn them off differs,
    /// which is why [`crate::turn_off_for_me`] masks rather than disables.
    Enabled,
    /// Installed and not linked.
    Disabled,
    /// Masked: nothing can start it until it is unmasked.
    Masked,
    /// No unit file by that name — the package is not installed.
    NotFound,
    /// Anything else systemd answers (`static`, `indirect`, `generated`,
    /// …), carried verbatim so a message can name it.
    Other(String),
}

impl UnitState {
    /// Reads `is-enabled`'s first word. Its exit status is no help — it is
    /// non-zero for `disabled`, `masked` and `not-found` alike — so the
    /// word is the answer.
    pub fn parse(word: &str) -> UnitState {
        match word.trim() {
            "enabled" | "enabled-runtime" | "alias" | "linked" | "linked-runtime" => {
                UnitState::Enabled
            }
            "disabled" => UnitState::Disabled,
            "masked" | "masked-runtime" => UnitState::Masked,
            "not-found" => UnitState::NotFound,
            other => UnitState::Other(other.to_string()),
        }
    }
}

/// A generated Lua file to write and have Hyprland load.
///
/// Owned rather than borrowed like `hyprforge_core::apply_lua::GeneratedFile`,
/// so the mock can keep what it was asked to write.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Generated {
    pub path: PathBuf,
    pub contents: String,
    /// What to leave if Hyprland refuses and there was no previous file.
    pub empty: String,
    /// Names the file in a rejection: `"shortcuts"`, `"rules"`.
    pub subject: &'static str,
}

/// How a generated file landed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Loaded {
    /// Written, reloaded, and Hyprland had no complaint.
    Live,
    /// Written, but Hyprland could not be asked — it is not running, or
    /// `hyprctl` is not installed. The file is kept: the next start reads
    /// it, and throwing away something nothing refused would be its own
    /// bug (`apply_lua` documents the same choice).
    NotReloaded(String),
}

/// What setup asks of the system. See the module doc.
pub trait System {
    /// Where `name` is on `$PATH`, if anywhere.
    fn find_binary(&self, name: &str) -> Option<PathBuf>;
    /// `systemctl --user is-enabled <unit>`. `Err` when the question could
    /// not be asked at all.
    fn unit_state(&self, unit: &str) -> Result<UnitState, String>;
    /// `systemctl --user is-active <unit>`.
    fn unit_active(&self, unit: &str) -> Result<bool, String>;
    /// `systemctl --user enable --now <unit>`.
    fn enable_now(&self, unit: &str) -> Result<(), String>;
    /// `systemctl --user disable --now <unit>`. Removes only the
    /// per-user link: a unit a package enabled for every user stays
    /// enabled, which is why [`System::mask_now`] exists.
    fn disable_now(&self, unit: &str) -> Result<(), String>;
    /// `systemctl --user mask --now <unit>` — the only per-user off switch
    /// for a unit enabled globally.
    fn mask_now(&self, unit: &str) -> Result<(), String>;
    /// `systemctl --user unmask <unit>`.
    fn unmask(&self, unit: &str) -> Result<(), String>;
    /// `systemctl --user try-restart <unit>`: restarts it only if it was
    /// running, so a change never starts something nobody started.
    fn try_restart(&self, unit: &str) -> Result<(), String>;
    /// Whether a process called `name` is running. `None` when that could
    /// not be found out — never folded into "no".
    fn process_running(&self, name: &str) -> Option<bool>;
    /// `hyprctl binds -j`.
    fn live_binds(&self) -> Result<Vec<LiveBind>, String>;
    /// Writes `file`, reloads Hyprland and rolls back if it refuses —
    /// `hyprforge_core::apply_lua::apply`. `Err` only for a refusal or a
    /// failed write, with the file already restored.
    fn apply_lua(&self, file: &Generated) -> Result<Loaded, String>;
    /// Restarts hypridle — only ever called when it is already running.
    fn restart_idle(&self) -> Result<(), String>;
    /// Asks the session bus to re-read its service files.
    fn reload_session_bus(&self) -> Result<(), String>;
}

/// The real system: `systemctl --user`, `pgrep`, `hyprctl`, `busctl`.
#[derive(Debug, Default, Clone, Copy)]
pub struct RealSystem;

/// Runs a program with the shared bound and returns its output, or what
/// went wrong in words.
fn run(program: &str, args: &[&str]) -> Result<std::process::Output, String> {
    hyprforge_process::output(Command::new(program).args(args), hyprforge_process::TIMEOUT)
        .map_err(|e| format!("couldn't run {program}: {e}"))
}

/// A `systemctl --user` verb that either worked or says why not.
fn systemctl(args: &[&str]) -> Result<(), String> {
    let mut full = vec!["--user"];
    full.extend_from_slice(args);
    let out = run("systemctl", &full)?;
    if out.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
    Err(if stderr.is_empty() {
        format!("systemctl --user {} failed", args.join(" "))
    } else {
        stderr
    })
}

fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    path.metadata().is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

impl System for RealSystem {
    fn find_binary(&self, name: &str) -> Option<PathBuf> {
        let path = std::env::var_os("PATH")?;
        std::env::split_paths(&path).map(|dir| dir.join(name)).find(|p| is_executable(p))
    }

    fn unit_state(&self, unit: &str) -> Result<UnitState, String> {
        let out = run("systemctl", &["--user", "is-enabled", unit])?;
        let stdout = String::from_utf8_lossy(&out.stdout);
        if let Some(word) = stdout.split_whitespace().next() {
            return Ok(UnitState::parse(word));
        }
        // Older systemd said "not found" on stderr rather than stdout.
        let stderr = String::from_utf8_lossy(&out.stderr);
        if stderr.contains("No such file") || stderr.contains("not found") {
            return Ok(UnitState::NotFound);
        }
        Err(format!("systemctl --user is-enabled {unit} said nothing: {}", stderr.trim()))
    }

    fn unit_active(&self, unit: &str) -> Result<bool, String> {
        // `is-active` exits 0 for active and non-zero for every other
        // answer, including a unit that does not exist — all of which are
        // "not running" here. Only failing to ask is an error.
        Ok(run("systemctl", &["--user", "is-active", "--quiet", unit])?.status.success())
    }

    fn enable_now(&self, unit: &str) -> Result<(), String> {
        systemctl(&["enable", "--now", unit])
    }

    fn disable_now(&self, unit: &str) -> Result<(), String> {
        systemctl(&["disable", "--now", unit])
    }

    fn mask_now(&self, unit: &str) -> Result<(), String> {
        systemctl(&["mask", "--now", unit])
    }

    fn unmask(&self, unit: &str) -> Result<(), String> {
        systemctl(&["unmask", unit])
    }

    fn try_restart(&self, unit: &str) -> Result<(), String> {
        systemctl(&["try-restart", unit])
    }

    fn process_running(&self, name: &str) -> Option<bool> {
        // `pgrep -x`, never `-f` (which matches the shell running it).
        // Every name asked about here fits in /proc's 15 characters.
        debug_assert!(name.len() <= 15, "pgrep -x cannot see {name}: comm is truncated at 15");
        run("pgrep", &["-x", name]).ok().map(|o| o.status.success())
    }

    fn live_binds(&self) -> Result<Vec<LiveBind>, String> {
        hyprforge_shortcuts::binds::list_binds().map_err(|e| e.to_string())
    }

    fn apply_lua(&self, file: &Generated) -> Result<Loaded, String> {
        use hyprforge_core::apply_lua::{apply, ApplyError, GeneratedFile};
        match apply(GeneratedFile {
            path: &file.path,
            contents: &file.contents,
            empty: &file.empty,
            subject: file.subject,
        }) {
            Ok(()) => Ok(Loaded::Live),
            // Not running, or no hyprctl: the file is written and nothing
            // refused it. `apply` already rolled back if the compositor
            // gained an error, which would have come back as rejected.
            Err(e @ (ApplyError::Spawn(_) | ApplyError::ReloadFailed { .. })) => {
                Ok(Loaded::NotReloaded(e.to_string()))
            }
            Err(e) => Err(e.to_string()),
        }
    }

    fn restart_idle(&self) -> Result<(), String> {
        hyprforge_ecosystem::apply::restart_idle().map_err(|e| e.to_string())
    }

    fn reload_session_bus(&self) -> Result<(), String> {
        // The same call the installer's `reload_session_bus` makes.
        let out = run(
            "busctl",
            &[
                "--user",
                "call",
                "org.freedesktop.DBus",
                "/org/freedesktop/DBus",
                "org.freedesktop.DBus",
                "ReloadConfig",
            ],
        )?;
        if out.status.success() {
            Ok(())
        } else {
            Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The words `systemctl --user is-enabled` prints, as measured on this
    /// machine: `not-found` on stdout, exit 4, for a unit that is not there.
    #[test]
    fn is_enabled_words_map_to_states() {
        assert_eq!(UnitState::parse("enabled\n"), UnitState::Enabled);
        assert_eq!(UnitState::parse("disabled"), UnitState::Disabled);
        assert_eq!(UnitState::parse("masked"), UnitState::Masked);
        assert_eq!(UnitState::parse("not-found"), UnitState::NotFound);
        assert_eq!(UnitState::parse("static"), UnitState::Other("static".into()));
    }
}
