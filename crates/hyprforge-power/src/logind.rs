//! The real [`InhibitBackend`], talking to `systemd-logind` on the system
//! bus.
//!
//! Everything here is D-Bus mechanics and the one piece of state this
//! whole crate exists to manage correctly: the file descriptor
//! `Inhibit` hands back. See [`crate::backend`] for why that descriptor
//! is never allowed to be a bare return value.

use crate::backend::InhibitBackend;
use crate::types::{InhibitError, InhibitorInfo, WhatSet};
use std::sync::Mutex;
use std::time::Duration;
use zbus::zvariant::OwnedFd;
use zbus::Connection;

/// Every call here is a property-free method call against a daemon that
/// is either up (systemd, always running) or not answering at all — there
/// is no hardware or network wait analogous to NetworkManager's
/// activation, so one bound covers everything.
pub const TIMEOUT: Duration = Duration::from_secs(5);

/// `"block"` prevents sleep/idle outright; `"delay"` only postpones it
/// briefly for cleanup. This crate's "keep awake" toggle wants the
/// former — a delay would still let the machine sleep a few seconds
/// later regardless of the toggle.
const MODE_BLOCK: &str = "block";

#[zbus::proxy(
    interface = "org.freedesktop.login1.Manager",
    default_service = "org.freedesktop.login1",
    default_path = "/org/freedesktop/login1"
)]
trait LoginManagerDbus {
    /// Returns a file descriptor. The inhibit lasts exactly as long as
    /// that descriptor stays open — see [`crate::backend`].
    fn inhibit(&self, what: &str, who: &str, why: &str, mode: &str) -> zbus::Result<OwnedFd>;

    /// `(what, who, why, mode, uid, pid)` for every inhibitor currently
    /// held, ours included.
    #[allow(clippy::type_complexity)]
    fn list_inhibitors(&self) -> zbus::Result<Vec<(String, String, String, String, u32, u32)>>;
}

/// Bounds a call and turns a D-Bus failure into something a screen can
/// say out loud.
async fn bounded<T>(
    limit: Duration,
    call: impl std::future::Future<Output = zbus::Result<T>>,
) -> Result<T, InhibitError> {
    match tokio::time::timeout(limit, call).await {
        Err(_elapsed) => Err(InhibitError::TimedOut(limit)),
        Ok(Ok(value)) => Ok(value),
        Ok(Err(e)) => Err(classify(e)),
    }
}

/// Which failures mean "logind isn't there" and which mean "logind said
/// no". Collapsing the two would put "nothing is inhibiting" on screen
/// for a bus that simply is not answering — the mistake this project
/// keeps finding in every D-Bus-backed module. Everything unrecognised
/// stays [`InhibitError::Refused`] carrying the original text, because a
/// message that was not anticipated is still more use than one invented.
fn classify(e: zbus::Error) -> InhibitError {
    if let zbus::Error::MethodError(name, _, _) = &e {
        let name = name.as_str();
        if name.ends_with(".ServiceUnknown") || name.ends_with(".NameHasNoOwner") {
            return InhibitError::Unavailable;
        }
        return InhibitError::Refused(e.to_string());
    }
    match e {
        // No bus to talk to at all.
        zbus::Error::Address(_) | zbus::Error::InputOutput(_) => InhibitError::Unavailable,
        other => InhibitError::Refused(other.to_string()),
    }
}

/// `systemd-logind`, as this crate uses it.
pub struct LogindBackend {
    connection: Connection,
    /// The inhibit this backend currently holds, if any: the file
    /// descriptor `Inhibit` returned, kept open, plus what it covers.
    /// Dropping the descriptor is what releases the lock, so this field
    /// existing and being populated *is* the inhibit — there is nothing
    /// else to it.
    held: Mutex<Option<(OwnedFd, WhatSet)>>,
}

impl LogindBackend {
    /// Opens the system bus.
    ///
    /// Failing here is [`InhibitError::Unavailable`] rather than a panic:
    /// a machine where the system bus cannot be reached is a state the
    /// screen has to describe, not crash on.
    pub async fn connect() -> Result<Self, InhibitError> {
        let connection = tokio::time::timeout(TIMEOUT, Connection::system())
            .await
            .map_err(|_| InhibitError::TimedOut(TIMEOUT))?
            .map_err(classify)?;
        Ok(LogindBackend { connection, held: Mutex::new(None) })
    }

    async fn manager(&self) -> Result<LoginManagerDbusProxy<'_>, InhibitError> {
        bounded(TIMEOUT, LoginManagerDbusProxy::new(&self.connection)).await
    }
}

#[async_trait::async_trait]
impl InhibitBackend for LogindBackend {
    async fn take(&self, what: WhatSet, who: &str, why: &str) -> Result<(), InhibitError> {
        // Already holding one: leave it alone. Taking a second lock here
        // would mean this backend now holds two descriptors for one
        // logical "keep awake" state, and a caller that releases once
        // would find the machine still inhibited by the other.
        if self.held.lock().unwrap().is_some() {
            return Ok(());
        }

        let manager = self.manager().await?;
        let fd = bounded(TIMEOUT, manager.inhibit(&what.to_arg(), who, why, MODE_BLOCK)).await?;
        *self.held.lock().unwrap() = Some((fd, what));
        Ok(())
    }

    async fn release(&self) -> Result<(), InhibitError> {
        // Dropping the `OwnedFd` closes it, which is what actually tells
        // logind the inhibit is over — there is no separate "release"
        // method to call on the bus.
        *self.held.lock().unwrap() = None;
        Ok(())
    }

    async fn held(&self) -> Result<Option<WhatSet>, InhibitError> {
        Ok(self.held.lock().unwrap().as_ref().map(|(_, what)| what.clone()))
    }

    async fn list_inhibitors(&self) -> Result<Vec<InhibitorInfo>, InhibitError> {
        let manager = self.manager().await?;
        let rows = bounded(TIMEOUT, manager.list_inhibitors()).await?;
        Ok(rows
            .into_iter()
            .map(|(what, who, why, mode, uid, pid)| InhibitorInfo { what, who, why, mode, uid, pid })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn block_is_the_mode_this_backend_always_asks_for() {
        // `"delay"` only postpones sleep briefly, which would still let
        // the machine sleep seconds after a "keep awake" toggle turned
        // on — pinned here so nobody swaps it in by habit from a
        // different logind example.
        assert_eq!(MODE_BLOCK, "block");
    }
}
