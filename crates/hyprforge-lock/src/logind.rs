//! Telling logind the session is locked.
//!
//! `LockedHint` is the property the rest of the system reads to find out
//! whether a session is locked — it is what `loginctl show-session`
//! reports, and what anything outside the compositor has to go on.
//! Nothing else can set it on our behalf: only the process holding the
//! `ext-session-lock` knows when the lock is actually up and when it has
//! been released. hypridle owns the *other* direction (it listens for
//! logind's `Lock` signal and runs `lock_cmd`), and that split is why
//! this module only ever writes.
//!
//! **Advisory, and never load-bearing.** Every call here is best-effort,
//! bounded, and off the event loop. A lock screen that failed to lock
//! because a bus call hung would be strictly worse than a locked screen
//! with a stale hint — the session is secured by the Wayland protocol,
//! not by this. So the failure mode is a wrong `loginctl` reading, which
//! is the mildest one available.
//!
//! Setting it is unprivileged: logind allows a session to set its own
//! hint, which is the one thing we want to do.

use std::time::Duration;

/// How long a hint may take. Deliberately short: this is a local socket
/// and a single method call, so anything slower than this is a bus that
/// is not answering, and the answer does not matter enough to wait.
const TIMEOUT: Duration = Duration::from_secs(2);

/// Sets `LockedHint` on our own session, in the background.
///
/// Returns immediately. The call happens on a detached thread with its
/// own runtime, because the lock screen's loop is calloop and must not
/// acquire a tokio runtime — and because the *point* is that the screen
/// never waits for this.
///
/// Use [`clear_locked_hint`] on the way out instead. A detached thread
/// does not hold the process open, and the session outlives this
/// process — so a fire-and-forget clear loses the race with exit and
/// leaves `LockedHint=yes` on an unlocked session, which is worse than
/// never having set it. Measured, not reasoned about: the first version
/// cleared it this way and the hint stayed `yes`.
pub fn set_locked_hint(locked: bool) {
    std::thread::spawn(move || {
        let runtime = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
            Ok(runtime) => runtime,
            Err(e) => {
                eprintln!("couldn't tell logind the session is locked: {e}");
                return;
            }
        };
        if let Err(e) = runtime.block_on(async {
            tokio::time::timeout(TIMEOUT, write_hint(locked))
                .await
                .unwrap_or_else(|_| Err("timed out".into()))
        }) {
            // Worth one line, never more. The lock itself is unaffected,
            // so this is information for someone wondering why
            // `loginctl` disagrees with the screen in front of them.
            eprintln!("couldn't tell logind the session is locked: {e}");
        }
    });
}

/// The call itself: find our own session, set the hint on it.
async fn write_hint(locked: bool) -> Result<(), String> {
    let connection = zbus::Connection::system().await.map_err(|e| e.to_string())?;

    let manager = zbus::Proxy::new(
        &connection,
        "org.freedesktop.login1",
        "/org/freedesktop/login1",
        "org.freedesktop.login1.Manager",
    )
    .await
    .map_err(|e| e.to_string())?;

    // Two ways to name our own session, tried in the order that
    // actually works rather than the order that sounds safest.
    //
    // `GetSessionByPID` was the first choice here, on the reasoning that
    // a lock screen launched by an idle daemon or a keybind cannot trust
    // its environment. Measured, that is exactly backwards: under uwsm
    // every app lands in `app.slice`, not the session scope, so logind
    // answers `NoSessionForPID` for anything the compositor started —
    // which is every way a lock screen is ever launched here. The PID
    // path is kept as the fallback for a session that does use scopes.
    //
    // `auto` resolves through `XDG_SESSION_ID`, and if that is unset it
    // resolves to the caller's session anyway, so it degrades to the
    // same answer rather than to a wrong one.
    let session: zbus::zvariant::OwnedObjectPath = match manager.call("GetSession", &("auto")).await
    {
        Ok(session) => session,
        Err(by_name) => manager
            .call("GetSessionByPID", &(std::process::id()))
            .await
            .map_err(|by_pid| format!("no session for this process ({by_name}; {by_pid})"))?,
    };

    let proxy = zbus::Proxy::new(
        &connection,
        "org.freedesktop.login1",
        session,
        "org.freedesktop.login1.Session",
    )
    .await
    .map_err(|e| e.to_string())?;

    proxy.call::<_, _, ()>("SetLockedHint", &(locked)).await.map_err(|e| e.to_string())
}

/// Clears the hint and waits for it, for the unlock path.
///
/// Blocking, unlike [`set_locked_hint`], because this is the last thing
/// the process does and there is nothing left to keep responsive. The
/// same bound applies: a bus that will not answer must not stop a lock
/// screen exiting, and the session is already unlocked by the time this
/// matters.
pub fn clear_locked_hint() {
    let done = std::thread::spawn(|| {
        let runtime = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
            Ok(runtime) => runtime,
            Err(e) => {
                eprintln!("couldn't tell logind the session is unlocked: {e}");
                return;
            }
        };
        if let Err(e) = runtime.block_on(async {
            tokio::time::timeout(TIMEOUT, write_hint(false))
                .await
                .unwrap_or_else(|_| Err("timed out".into()))
        }) {
            eprintln!("couldn't tell logind the session is unlocked: {e}");
        }
    });
    // Joining is the point. If the thread is wedged past its own bound
    // the join still returns once it finishes; a panic there is nothing
    // to act on at this stage.
    let _ = done.join();
}
