//! Keep awake that outlives the window that turned it on.
//!
//! An inhibit lasts exactly as long as its file descriptor is open — see
//! [`crate::backend`]'s module doc — and the descriptor belongs to the
//! process that took it. Held by the Settings window, keep awake ended
//! the moment the window closed; held by the tray daemon, it ended
//! whenever the daemon restarted after an upgrade. Both looked like "keep
//! awake does nothing", because in each case nothing said the lock was
//! gone; the machine simply slept on its next idle timeout.
//!
//! So the descriptor gets a process of its own: a *holder*, started
//! detached by whichever program turned keep awake on. It is that
//! program's own executable run with [`HOLDER_ARG`], so nothing has to be
//! installed beside it, and it does one thing — take the inhibit, then
//! wait. Turning keep awake off stops it, which closes the descriptor.
//!
//! Nothing records the holder's pid. logind already lists every inhibitor
//! with its pid and its *who*, so the holder names itself [`HOLDER_WHO`]
//! and is found there — by Settings, by the tray, by a second copy of
//! either — which is also what makes every one of them agree on whether
//! keep awake is on. A pid is never signalled on the strength of that
//! list alone: it must belong to this user and its command line must
//! carry [`HOLDER_ARG`] ([`holders_to_stop`]), so a recycled pid or
//! another program that happened to pick the same name is left alone.

use crate::backend::InhibitBackend;
use crate::logind::LogindBackend;
use crate::types::{InhibitError, InhibitorInfo, WhatSet};
use std::os::unix::fs::MetadataExt;
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::time::Duration;

/// The *who* a holder gives logind, and how it is found again.
pub const HOLDER_WHO: &str = "Hyprforge keep awake";

/// The argument that makes a Hyprforge binary a holder instead of itself.
/// A program that offers keep awake checks for it first thing in `main`
/// and calls [`hold`].
pub const HOLDER_ARG: &str = "--hold-keep-awake";

const WHY: &str = "Keep awake is on";

/// How long a holder gets to show up in logind's list, or to leave it.
/// Taking an inhibit is one bus call; a holder that has not appeared in
/// this long failed to start or to take it.
const SETTLE: Duration = Duration::from_secs(3);
const POLL: Duration = Duration::from_millis(100);

/// Of these inhibitors, the pids that are this user's keep-awake holders
/// and safe to stop. `uid` is ours; `cmdline` reads a process's command
/// line, NUL-separated as `/proc/<pid>/cmdline` is, or `None` if it is
/// gone.
pub fn holders_to_stop(
    inhibitors: &[InhibitorInfo],
    uid: u32,
    cmdline: impl Fn(u32) -> Option<Vec<u8>>,
) -> Vec<u32> {
    inhibitors
        .iter()
        .filter(|i| i.who == HOLDER_WHO && i.uid == uid)
        .filter(|i| {
            cmdline(i.pid).is_some_and(|raw| raw.split(|b| *b == 0).any(|arg| arg == HOLDER_ARG.as_bytes()))
        })
        .map(|i| i.pid)
        .collect()
}

/// Whether one of these is this user's holder.
pub fn is_on(inhibitors: &[InhibitorInfo], uid: u32) -> bool {
    inhibitors.iter().any(|i| i.who == HOLDER_WHO && i.uid == uid)
}

/// Inhibitors that are not this user's keep awake — what a screen lists
/// under "also keeping this machine awake".
pub fn others(inhibitors: Vec<InhibitorInfo>, uid: u32) -> Vec<InhibitorInfo> {
    inhibitors.into_iter().filter(|i| !(i.who == HOLDER_WHO && i.uid == uid)).collect()
}

/// How a holder is started: in a transient systemd scope of its own when
/// the user has a systemd manager, directly otherwise.
///
/// A scope, because a child stays in its parent's cgroup, and the tray
/// runs as a systemd user service with the default
/// `KillMode=control-group`: restarting the tray — which
/// `./hyprforge --install` does whenever its binary changes — killed every
/// process in that cgroup, the holder with it. A new process group does
/// not leave the cgroup; `systemd-run --scope` does. `--collect` so a
/// holder that is stopped leaves no unit behind.
fn holder_command(program: &std::path::Path) -> std::process::Command {
    if std::path::Path::new("/run/systemd/system").exists() && which("systemd-run") {
        let mut command = std::process::Command::new("systemd-run");
        command
            .args(["--user", "--scope", "--quiet", "--collect", "--unit"])
            .arg(format!("hyprforge-keep-awake-{}", std::process::id()))
            .arg("--")
            .arg(program)
            .arg(HOLDER_ARG);
        command
    } else {
        let mut command = std::process::Command::new(program);
        command.arg(HOLDER_ARG);
        command
    }
}

/// Whether `name` is a program on `$PATH`.
fn which(name: &str) -> bool {
    std::env::var_os("PATH")
        .is_some_and(|path| std::env::split_paths(&path).any(|dir| dir.join(name).is_file()))
}

fn our_uid() -> u32 {
    std::fs::metadata("/proc/self").map(|m| m.uid()).unwrap_or(u32::MAX)
}

fn cmdline(pid: u32) -> Option<Vec<u8>> {
    std::fs::read(format!("/proc/{pid}/cmdline")).ok()
}

/// Keep awake as a detached holder process — an [`InhibitBackend`], so a
/// screen written against the trait uses it unchanged.
pub struct DetachedBackend {
    /// The program to run as the holder: the caller's own executable.
    program: PathBuf,
    /// For listing inhibitors. Connected on first use, so constructing
    /// this costs nothing on a machine where it is never asked.
    logind: tokio::sync::Mutex<Option<std::sync::Arc<LogindBackend>>>,
}

impl DetachedBackend {
    /// Holders are started as `program --hold-keep-awake`.
    pub fn new(program: PathBuf) -> DetachedBackend {
        DetachedBackend { program, logind: tokio::sync::Mutex::new(None) }
    }

    /// The running executable as the holder — what every caller wants.
    pub fn this_binary() -> std::io::Result<DetachedBackend> {
        Ok(DetachedBackend::new(std::env::current_exe()?))
    }

    async fn logind(&self) -> Result<std::sync::Arc<LogindBackend>, InhibitError> {
        let mut slot = self.logind.lock().await;
        if let Some(backend) = slot.as_ref() {
            return Ok(backend.clone());
        }
        let backend = std::sync::Arc::new(LogindBackend::connect().await?);
        *slot = Some(backend.clone());
        Ok(backend)
    }

    /// Polls until keep awake reads `want`, or [`SETTLE`] passes.
    async fn settle(&self, want: bool) -> Result<bool, InhibitError> {
        let logind = self.logind().await?;
        let deadline = tokio::time::Instant::now() + SETTLE;
        loop {
            if is_on(&logind.list_inhibitors().await?, our_uid()) == want {
                return Ok(true);
            }
            if tokio::time::Instant::now() >= deadline {
                return Ok(false);
            }
            tokio::time::sleep(POLL).await;
        }
    }
}

#[async_trait::async_trait]
impl InhibitBackend for DetachedBackend {
    /// Starts a holder, unless one is already running, and waits until
    /// logind lists it. `who` and `why` are ignored: the holder names
    /// itself [`HOLDER_WHO`], because that name is how it is found again.
    async fn take(&self, _what: WhatSet, _who: &str, _why: &str) -> Result<(), InhibitError> {
        if self.held().await?.is_some() {
            return Ok(());
        }
        let mut child = holder_command(&self.program)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            // Its own process group: a Ctrl+C in the terminal that
            // started Settings must not take keep awake down with it.
            .process_group(0)
            .spawn()
            .map_err(|e| InhibitError::Refused(format!("keep awake could not start: {e}")))?;
        // Reaped when it ends, so a long-lived starter (the tray) never
        // collects zombies. The holder does not end with its starter.
        std::thread::spawn(move || {
            let _ = child.wait();
        });
        if self.settle(true).await? {
            Ok(())
        } else {
            Err(InhibitError::Refused("keep awake started but never took hold".to_string()))
        }
    }

    /// Stops every holder of this user's, wherever it was started from.
    async fn release(&self) -> Result<(), InhibitError> {
        let logind = self.logind().await?;
        let pids = holders_to_stop(&logind.list_inhibitors().await?, our_uid(), cmdline);
        for pid in pids {
            let mut kill = std::process::Command::new("kill");
            kill.args(["-TERM", &pid.to_string()]);
            if let Err(e) = hyprforge_process::output(&mut kill, hyprforge_process::TIMEOUT) {
                return Err(InhibitError::Refused(format!("keep awake could not be stopped: {e}")));
            }
        }
        if self.settle(false).await? {
            Ok(())
        } else {
            Err(InhibitError::Refused("keep awake is still on after being stopped".to_string()))
        }
    }

    async fn held(&self) -> Result<Option<WhatSet>, InhibitError> {
        let on = is_on(&self.logind().await?.list_inhibitors().await?, our_uid());
        Ok(on.then(WhatSet::keep_awake))
    }

    async fn list_inhibitors(&self) -> Result<Vec<InhibitorInfo>, InhibitError> {
        self.logind().await?.list_inhibitors().await
    }
}

/// The holder itself: take the inhibit and keep it until stopped. Called
/// from `main` when [`HOLDER_ARG`] is the first argument; returns only if
/// the inhibit could not be taken.
pub async fn hold() -> Result<(), InhibitError> {
    let logind = LogindBackend::connect().await?;
    logind.take(WhatSet::keep_awake(), HOLDER_WHO, WHY).await?;
    // Stopped by SIGTERM, whose default action ends the process; the
    // kernel closes the descriptor, and logind ends the inhibit.
    std::future::pending::<()>().await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inhibitor(who: &str, uid: u32, pid: u32) -> InhibitorInfo {
        InhibitorInfo {
            what: "sleep:idle".into(),
            who: who.into(),
            why: "x".into(),
            mode: "block".into(),
            uid,
            pid,
        }
    }

    fn holder_cmdline() -> Vec<u8> {
        b"/usr/bin/hyprforge-trayd\0--hold-keep-awake\0".to_vec()
    }

    #[test]
    fn only_this_users_holders_are_stopped() {
        let list = [inhibitor(HOLDER_WHO, 1000, 10), inhibitor(HOLDER_WHO, 1001, 11)];
        assert_eq!(holders_to_stop(&list, 1000, |_| Some(holder_cmdline())), vec![10]);
    }

    #[test]
    fn a_process_that_merely_borrowed_the_name_is_left_alone() {
        // Same name, same user — but not started as a holder: a recycled
        // pid, or someone else's program.
        let list = [inhibitor(HOLDER_WHO, 1000, 10)];
        assert!(holders_to_stop(&list, 1000, |_| Some(b"/usr/bin/firefox\0".to_vec())).is_empty());
        assert!(holders_to_stop(&list, 1000, |_| None).is_empty(), "a pid already gone");
    }

    #[test]
    fn the_flag_must_be_a_whole_argument_not_a_substring() {
        let list = [inhibitor(HOLDER_WHO, 1000, 10)];
        let sneaky = b"/bin/sh\0-c\0echo --hold-keep-awake-please\0".to_vec();
        assert!(holders_to_stop(&list, 1000, |_| Some(sneaky.clone())).is_empty());
    }

    #[test]
    fn other_holders_are_never_stopped() {
        let list = [inhibitor("NetworkManager", 0, 1), inhibitor("hypridle", 1000, 2)];
        assert!(holders_to_stop(&list, 1000, |_| Some(holder_cmdline())).is_empty());
    }

    #[test]
    fn keep_awake_is_on_when_this_users_holder_is_listed() {
        assert!(is_on(&[inhibitor(HOLDER_WHO, 1000, 10)], 1000));
        assert!(!is_on(&[inhibitor(HOLDER_WHO, 1001, 10)], 1000), "another user's is not ours");
        assert!(!is_on(&[inhibitor("hypridle", 1000, 2)], 1000));
    }

    #[test]
    fn the_holder_is_not_listed_among_the_others() {
        let list = vec![inhibitor(HOLDER_WHO, 1000, 10), inhibitor("hypridle", 1000, 2)];
        let rest = others(list, 1000);
        assert_eq!(rest.len(), 1);
        assert_eq!(rest[0].who, "hypridle");
    }
}
