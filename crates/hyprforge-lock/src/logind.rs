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
/// Not waiting has a consequence worth naming: on the unlock path this
/// process is about to exit, and a detached thread does not hold it
/// open. The hint is therefore written on a best-effort basis there
/// too — logind clears it when the session ends anyway, so the window
/// where it could be stale is one this cannot make worse.
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

    // By our own PID rather than `auto`. `GetSession("auto")` resolves
    // through `XDG_SESSION_ID`, which a lock screen cannot rely on: it
    // may have been started by an idle daemon or a keybind whose
    // environment is not the session's.
    let manager = zbus::Proxy::new(
        &connection,
        "org.freedesktop.login1",
        "/org/freedesktop/login1",
        "org.freedesktop.login1.Manager",
    )
    .await
    .map_err(|e| e.to_string())?;

    let session: zbus::zvariant::OwnedObjectPath = manager
        .call("GetSessionByPID", &(std::process::id()))
        .await
        .map_err(|e| e.to_string())?;

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
