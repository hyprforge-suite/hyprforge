//! Which Hyprland a live test may talk to, and putting it back afterwards.
//!
//! Hyprforge's live suites ask a real compositor whether it agrees with
//! the option catalogues. About thirty of those tests *change* the
//! compositor to find out: every option flipped, every range at both
//! ends, binds on F13–F24, window rules, gestures. Until 2026-10-11 they
//! ran against whatever `hyprctl` reached, which was the session the
//! person running them was working in. Six full runs in one evening
//! left that session slow, then frozen, until a re-login — `hyprctl
//! reload` puts back the config file, not the renderer state a flipped
//! `render:` option leaves behind.
//!
//! So there are two doors, and a test has to choose one:
//!
//! - [`Hyprland::read_only`] for a test that only asks (`getoption`,
//!   `binds -j`, `instances`). It talks to the current instance, or to
//!   the disposable one when `tools/hf test disposable` is running.
//! - [`Hyprland::disposable`] for a test that changes anything. It
//!   refuses — panics, so the run is red rather than quietly green —
//!   unless `HYPRFORGE_TEST_HYPRLAND` names an instance that is itself a
//!   Wayland client, i.e. nested inside another compositor. A session
//!   compositor started from a TTY has no `WAYLAND_DISPLAY` in its
//!   environment; a nested one cannot start without it. That check
//!   needs nothing recorded in advance and cannot be satisfied by the
//!   session by accident.
//!
//! The [`Disposable`] it returns reloads the instance when dropped, and
//! a drop runs during a panic's unwind too — so an assertion failing
//! between an `eval` and the test's own reload no longer leaves the
//! probe values live. That was true of three suites.
//!
//! Every `hyprctl` here names its instance explicitly with `--instance`
//! and is bounded (`hyprforge_process::output`), because a nested
//! compositor that wedges must fail a test, not hang the run.

use std::path::PathBuf;
use std::process::Command;
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

/// The instance signature `tools/hf` exports for the disposable tier.
pub const DISPOSABLE_ENV: &str = "HYPRFORGE_TEST_HYPRLAND";

/// The session's own signature, recorded by `tools/hf` before it starts
/// anything. A second line of defence: the nesting check alone already
/// refuses the session.
pub const SESSION_ENV: &str = "HYPRFORGE_SESSION_SIGNATURE";

/// A bound, not a tuning: a reload of a large config on a busy machine
/// takes well under a second, and a compositor that has not answered in
/// thirty has stopped answering.
const HYPRCTL_TIMEOUT: Duration = Duration::from_secs(30);

/// Serialises tests within one test binary under plain `cargo test`.
/// nextest runs each test in its own process and serialises with its
/// `compositor` test group instead; this lock costs nothing there.
static COMPOSITOR: Mutex<()> = Mutex::new(());

fn lock() -> MutexGuard<'static, ()> {
    COMPOSITOR.lock().unwrap_or_else(|e| e.into_inner())
}

/// A Hyprland instance a test may talk to.
#[derive(Debug, Clone)]
pub struct Hyprland {
    /// `None` means "whatever `hyprctl` reaches without being told",
    /// which is `HYPRLAND_INSTANCE_SIGNATURE`.
    instance: Option<String>,
}

/// A read-only handle; holds the in-process lock for the test's life.
pub struct ReadOnly {
    hyprland: Hyprland,
    _lock: MutexGuard<'static, ()>,
}

/// A disposable instance. Reloads it on drop, panicking or not.
pub struct Disposable {
    hyprland: Hyprland,
    _lock: MutexGuard<'static, ()>,
}

impl Hyprland {
    /// For a test that only asks. Targets the disposable instance when
    /// one is exported, so the same read-only tests can run inside
    /// `hf test disposable`; otherwise the current one.
    pub fn read_only() -> ReadOnly {
        let instance = std::env::var(DISPOSABLE_ENV).ok().filter(|s| !s.is_empty());
        ReadOnly {
            hyprland: Hyprland { instance },
            _lock: lock(),
        }
    }

    /// For a test that changes anything. Panics unless
    /// `HYPRFORGE_TEST_HYPRLAND` names a nested instance; see the crate
    /// docs for why that is the check.
    pub fn disposable() -> Disposable {
        let guard = lock();
        let wanted = std::env::var(DISPOSABLE_ENV).ok();
        let session = std::env::var(SESSION_ENV).ok();
        let instances = run_hyprctl(None, &["instances", "-j"]);
        match vet(wanted.as_deref(), session.as_deref(), &instances, environ_of) {
            Ok(instance) => Disposable {
                hyprland: Hyprland { instance: Some(instance) },
                _lock: guard,
            },
            Err(why) => panic!(
                "this test changes Hyprland's state, so it runs only against a \
                 disposable nested instance — `tools/hf test disposable` starts one.\n\
                 refused: {why}"
            ),
        }
    }

    /// `hyprctl <args>` against this instance; stdout, lossily decoded.
    /// A hyprctl that cannot start or does not answer panics: every
    /// caller is a test, and an empty string would read as an answer.
    pub fn hyprctl(&self, args: &[&str]) -> String {
        run_hyprctl(self.instance.as_deref(), args)
    }

    /// `hyprctl reload`.
    pub fn reload(&self) {
        self.hyprctl(&["reload"]);
    }

    /// `hyprctl configerrors`, with "no errors" read as empty.
    pub fn config_errors(&self) -> String {
        let out = self.hyprctl(&["configerrors"]);
        let trimmed = out.trim();
        if trimmed == "no errors" || trimmed.is_empty() {
            String::new()
        } else {
            trimmed.to_string()
        }
    }

    /// This instance's signature, if it was named.
    pub fn signature(&self) -> Option<&str> {
        self.instance.as_deref()
    }

    /// Where a companion daemon (`hyprpaper`, `hyprsunset`) keeps its
    /// socket for this instance. Each instance has its own directory,
    /// so a daemon started under the disposable instance's signature
    /// never touches the session's — the hazard CLAUDE.md names about a
    /// second hyprpaper is a second one under the *same* signature.
    pub fn daemon_socket(&self, daemon: &str) -> Option<PathBuf> {
        let sig = match &self.instance {
            Some(s) => s.clone(),
            None => std::env::var("HYPRLAND_INSTANCE_SIGNATURE").ok()?,
        };
        let runtime = std::env::var_os("XDG_RUNTIME_DIR")?;
        Some(PathBuf::from(runtime).join("hypr").join(sig).join(format!(".{daemon}.sock")))
    }

    /// Whether `daemon` answers on this instance's socket. Replaces a
    /// machine-wide `pgrep -x`, which sees the session's daemon while
    /// `hyprctl` is talking to another instance.
    pub fn daemon_running(&self, daemon: &str) -> bool {
        self.daemon_socket(daemon)
            .is_some_and(|p| std::os::unix::net::UnixStream::connect(p).is_ok())
    }
}

/// The instance a suite's free helpers talk to: the disposable one when
/// exported, otherwise whatever `hyprctl` reaches. The same rule
/// [`Hyprland::read_only`] uses — and a test that changes anything has
/// already been vetted by [`Hyprland::disposable`] before it gets here,
/// because that is the first line of every such test.
fn current() -> Hyprland {
    Hyprland {
        instance: std::env::var(DISPOSABLE_ENV).ok().filter(|s| !s.is_empty()),
    }
}

/// `hyprctl <args>` against the current instance. For suites whose
/// helpers are free functions rather than methods on a handle.
pub fn hyprctl(args: &[&str]) -> String {
    current().hyprctl(args)
}

/// `hyprctl reload` against the current instance.
pub fn reload() {
    current().reload()
}

/// `hyprctl configerrors` against the current instance, "no errors" read
/// as empty.
pub fn config_errors() -> String {
    current().config_errors()
}

impl std::ops::Deref for ReadOnly {
    type Target = Hyprland;
    fn deref(&self) -> &Hyprland {
        &self.hyprland
    }
}

impl std::ops::Deref for Disposable {
    type Target = Hyprland;
    fn deref(&self) -> &Hyprland {
        &self.hyprland
    }
}

impl Drop for Disposable {
    fn drop(&mut self) {
        // Not `self.reload()`: that panics on a hyprctl that will not
        // answer, and a panic inside a drop during an unwind aborts the
        // whole binary, taking the real failure's message with it.
        let _ = output(self.hyprland.instance.as_deref(), &["reload"]);
    }
}

fn output(instance: Option<&str>, args: &[&str]) -> std::io::Result<std::process::Output> {
    let mut cmd = Command::new("hyprctl");
    if let Some(sig) = instance {
        cmd.args(["--instance", sig]);
    }
    cmd.args(args);
    hyprforge_process::output(&mut cmd, HYPRCTL_TIMEOUT)
}

fn run_hyprctl(instance: Option<&str>, args: &[&str]) -> String {
    match output(instance, args) {
        Ok(out) => String::from_utf8_lossy(&out.stdout).into_owned(),
        Err(e) => panic!("hyprctl {} failed: {e}", args.join(" ")),
    }
}

/// The value of `WAYLAND_DISPLAY` in a process's environment, or `None`
/// when it has none (or the process is gone).
fn environ_of(pid: u64) -> Option<Option<String>> {
    let raw = std::fs::read(format!("/proc/{pid}/environ")).ok()?;
    Some(
        raw.split(|b| *b == 0)
            .filter_map(|kv| std::str::from_utf8(kv).ok())
            .find_map(|kv| kv.strip_prefix("WAYLAND_DISPLAY=").map(str::to_string)),
    )
}

/// Decide whether `wanted` may be treated as disposable. Pure apart from
/// `environ`, so every refusal is pinned by a unit test without a
/// compositor.
fn vet(
    wanted: Option<&str>,
    session: Option<&str>,
    instances_json: &str,
    environ: impl Fn(u64) -> Option<Option<String>>,
) -> Result<String, String> {
    let wanted = match wanted {
        Some(w) if !w.is_empty() => w,
        _ => return Err(format!("{DISPOSABLE_ENV} is not set")),
    };
    if session == Some(wanted) {
        return Err(format!("{DISPOSABLE_ENV} names the session ({SESSION_ENV})"));
    }
    let list: serde_json::Value = serde_json::from_str(instances_json.trim())
        .map_err(|e| format!("`hyprctl instances -j` was not JSON: {e}"))?;
    let entry = list
        .as_array()
        .into_iter()
        .flatten()
        .find(|i| i.get("instance").and_then(|v| v.as_str()) == Some(wanted))
        .ok_or_else(|| format!("no running Hyprland instance has signature {wanted}"))?;
    let pid = entry
        .get("pid")
        .and_then(|v| v.as_u64())
        .ok_or_else(|| "the instance has no pid".to_string())?;
    match environ(pid) {
        None => Err(format!("cannot read /proc/{pid}/environ")),
        Some(None) => Err(format!(
            "instance {wanted} (pid {pid}) has no WAYLAND_DISPLAY: it is driving \
             hardware, which makes it a session, not a nested instance"
        )),
        Some(Some(_parent)) => Ok(wanted.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TWO: &str = r#"[
        {"instance": "session_sig", "pid": 10, "wl_socket": "wayland-1"},
        {"instance": "nested_sig", "pid": 20, "wl_socket": "wayland-2"}
    ]"#;

    fn env(pid: u64) -> Option<Option<String>> {
        match pid {
            10 => Some(None),
            20 => Some(Some("wayland-1".into())),
            _ => None,
        }
    }

    #[test]
    fn nothing_exported_means_no_disposable_instance() {
        assert!(vet(None, None, TWO, env).unwrap_err().contains("not set"));
        assert!(vet(Some(""), None, TWO, env).is_err());
    }

    #[test]
    fn the_session_is_refused_even_when_named_outright() {
        let why = vet(Some("session_sig"), None, TWO, env).unwrap_err();
        assert!(why.contains("no WAYLAND_DISPLAY"), "{why}");
    }

    #[test]
    fn the_recorded_session_signature_is_refused_before_anything_is_asked() {
        // Even a nested-looking instance, if it is the one hf recorded as
        // the session, is refused.
        let why = vet(Some("nested_sig"), Some("nested_sig"), TWO, env).unwrap_err();
        assert!(why.contains("names the session"), "{why}");
    }

    #[test]
    fn a_nested_instance_is_accepted() {
        assert_eq!(vet(Some("nested_sig"), Some("session_sig"), TWO, env).unwrap(), "nested_sig");
    }

    #[test]
    fn an_instance_that_is_not_running_is_refused() {
        assert!(vet(Some("gone"), None, TWO, env).unwrap_err().contains("no running"));
    }

    #[test]
    fn an_unreadable_environment_is_a_refusal_not_a_pass() {
        let one = r#"[{"instance": "x", "pid": 99, "wl_socket": "wayland-3"}]"#;
        assert!(vet(Some("x"), None, one, env).unwrap_err().contains("environ"));
    }

    #[test]
    fn garbage_from_hyprctl_is_a_refusal() {
        assert!(vet(Some("x"), None, "hyprctl: not running", env).is_err());
    }
}
