//! The seams: what a host asks of drives and of servers, so everything
//! above the D-Bus client and the `gio` runner is testable without
//! either.
//!
//! Two traits, not one, for the reason `hyprforge-power` gives for its
//! three: UDisks2 and gvfs are independent services with independent
//! lifetimes. A machine can have one without the other, and nothing that
//! asks about drives should fail because gvfs is missing.

use crate::gadgets::Gadgets;
use crate::types::{Answers, ConnectError, Gvfs, Share, Volume, VolumeError, VolumeId};
use hyprforge_secret::Secret;
use std::path::{Path, PathBuf};
use tokio::sync::mpsc;

/// Drives and the volumes on them.
#[async_trait::async_trait]
pub trait VolumeBackend: Send + Sync {
    /// Every volume worth showing — see [`crate::inventory`] for which.
    async fn volumes(&self) -> Result<Vec<Volume>, VolumeError>;

    /// Mounts `id`, answering where.
    async fn mount(&self, id: &VolumeId) -> Result<PathBuf, VolumeError>;

    /// Unmounts `id` — and when it is the inside of an encrypted
    /// volume, locks that volume again, so the passphrase is needed the
    /// next time as it was this time.
    async fn unmount(&self, id: &VolumeId) -> Result<(), VolumeError>;

    /// Unmounts everything on `id`'s drive, locks every encrypted volume
    /// on it, and lets go of it — powers a stick off, opens a tray,
    /// detaches a disk image.
    async fn eject(&self, id: &VolumeId) -> Result<(), VolumeError>;

    /// Unlocks the encrypted volume `id` with `passphrase` and mounts the
    /// filesystem inside, answering where — the one step a click on a
    /// locked drive means. A passphrase that does not fit is
    /// [`VolumeError::WrongPassphrase`].
    ///
    /// Once unlocked, the volume is listed as what is inside it, under
    /// an id of its own; `id` names the locked container, which is no
    /// longer listed.
    async fn unlock(&self, id: &VolumeId, passphrase: &Secret<String>) -> Result<PathBuf, VolumeError>;

    /// A receiver that gets `()` whenever the volumes may have changed.
    ///
    /// "May have": a signal means *look again*, never "here is the
    /// change", and plugging one stick in sends dozens — so the channel
    /// holds one and drops the rest, and [`crate::settle`] waits for a
    /// burst to finish before anyone reads. It also fires when UDisks2
    /// itself starts or stops, which is how "UDisks2 isn't running"
    /// clears without a restart.
    async fn watch(&self) -> Result<mpsc::Receiver<()>, VolumeError>;
}

/// Network shares: what is mounted, and connecting and disconnecting.
#[async_trait::async_trait]
pub trait ShareBackend: Send + Sync {
    /// Whether gvfs is here, and what it can reach.
    async fn gvfs(&self) -> Gvfs;

    /// Every network share mounted now — gvfs's, and the kernel's.
    async fn shares(&self) -> Vec<Share>;

    /// Mounts `uri`, answering prompts from `answers`; `Some` path to
    /// browse when gvfs gave the share one.
    ///
    /// Cancelled by dropping the future: the `gio` it runs dies with it.
    async fn connect(&self, uri: &str, answers: Answers) -> Result<Option<PathBuf>, ConnectError>;

    async fn disconnect(&self, share: &Share) -> Result<(), String>;

    /// Phones and cameras: what gvfs's MTP and gPhoto2 monitors list,
    /// and what is on USB that no installed backend reads — see
    /// [`crate::gadgets`]. Opening one is [`Self::connect`] on its URI.
    async fn gadgets(&self) -> Gadgets;

    /// Unmounts the phone or camera at `uri`. `Err` is gio's words.
    async fn release(&self, uri: &str) -> Result<(), String>;

    /// Fires whenever gvfs mounts or unmounts something — gvfs's own
    /// signal, so a share mounted by another program arrives too — and
    /// whenever a phone or camera is plugged in or pulled out.
    async fn watch(&self) -> mpsc::Receiver<()>;
}

/// Whether `path` lies on `share`.
pub fn on_share(share: &Share, path: &Path) -> bool {
    path.starts_with(&share.path)
}

#[cfg(feature = "mock")]
pub mod mock {
    //! In-memory backends: drives that mount, refuse and vanish on
    //! cue, and a gvfs that answers from a script.

    use super::*;
    use crate::types::Operation;
    use std::collections::HashMap;
    use std::sync::Mutex;

    #[derive(Default)]
    struct State {
        volumes: Vec<Volume>,
        passphrases: HashMap<VolumeId, String>,
        unavailable: bool,
        failures: HashMap<(Operation, VolumeId), VolumeError>,
        calls: Vec<(Operation, VolumeId)>,
        watchers: Vec<mpsc::Sender<()>>,
    }

    /// A [`VolumeBackend`] over a list held in memory.
    #[derive(Default)]
    pub struct MockVolumes {
        state: Mutex<State>,
    }

    impl MockVolumes {
        pub fn new(volumes: Vec<Volume>) -> Self {
            let mock = MockVolumes::default();
            mock.state().volumes = volumes;
            mock
        }

        fn state(&self) -> std::sync::MutexGuard<'_, State> {
            self.state.lock().unwrap_or_else(|p| p.into_inner())
        }

        /// UDisks2 goes away (`true`) or comes back.
        pub fn set_unavailable(&self, unavailable: bool) {
            self.state().unavailable = unavailable;
            self.poke();
        }

        /// The next `op` on `id` fails with `error`.
        pub fn fail(&self, op: Operation, id: &VolumeId, error: VolumeError) {
            self.state().failures.insert((op, id.clone()), error);
        }

        /// Replaces the list, as plugging or unplugging would, and tells
        /// every watcher.
        pub fn replace(&self, volumes: Vec<Volume>) {
            self.state().volumes = volumes;
            self.poke();
        }

        /// `id` is encrypted, and unlocks with `passphrase`.
        pub fn set_passphrase(&self, id: &VolumeId, passphrase: &str) {
            self.state().passphrases.insert(id.clone(), passphrase.to_string());
        }

        /// What has been asked, in order.
        pub fn calls(&self) -> Vec<(Operation, VolumeId)> {
            self.state().calls.clone()
        }

        fn poke(&self) {
            for tx in &self.state().watchers {
                let _ = tx.try_send(());
            }
        }

        fn act(&self, op: Operation, id: &VolumeId) -> Result<usize, VolumeError> {
            let mut state = self.state();
            state.calls.push((op, id.clone()));
            if state.unavailable {
                return Err(VolumeError::Unavailable);
            }
            if let Some(error) = state.failures.remove(&(op, id.clone())) {
                return Err(error);
            }
            state.volumes.iter().position(|v| &v.id == id).ok_or(VolumeError::Gone)
        }
    }

    #[async_trait::async_trait]
    impl VolumeBackend for MockVolumes {
        async fn volumes(&self) -> Result<Vec<Volume>, VolumeError> {
            let state = self.state();
            if state.unavailable {
                return Err(VolumeError::Unavailable);
            }
            Ok(state.volumes.clone())
        }

        async fn mount(&self, id: &VolumeId) -> Result<PathBuf, VolumeError> {
            let at = self.act(Operation::Mount, id)?;
            let mut state = self.state();
            let volume = &mut state.volumes[at];
            if volume.locked {
                return Err(VolumeError::Locked);
            }
            let point = PathBuf::from("/run/media/mock").join(&volume.label);
            volume.mount_point = Some(point.clone());
            Ok(point)
        }

        /// An encrypted volume locks again, as the real one does.
        async fn unmount(&self, id: &VolumeId) -> Result<(), VolumeError> {
            let at = self.act(Operation::Unmount, id)?;
            let mut state = self.state();
            let encrypted = state.passphrases.contains_key(id);
            let volume = &mut state.volumes[at];
            volume.mount_point = None;
            if encrypted {
                volume.locked = true;
                volume.filesystem = None;
            }
            Ok(())
        }

        async fn eject(&self, id: &VolumeId) -> Result<(), VolumeError> {
            let at = self.act(Operation::Eject, id)?;
            self.state().volumes.remove(at);
            Ok(())
        }

        async fn watch(&self) -> Result<mpsc::Receiver<()>, VolumeError> {
            let (tx, rx) = mpsc::channel(1);
            self.state().watchers.push(tx);
            Ok(rx)
        }

        /// Unlocked in place, keeping its id — simpler than UDisks2's new
        /// object for the inside, and enough for a window's tests, which
        /// ask what the row says and where the tab went.
        async fn unlock(&self, id: &VolumeId, passphrase: &Secret<String>) -> Result<PathBuf, VolumeError> {
            let at = self.act(Operation::Unlock, id)?;
            let mut state = self.state();
            if state.passphrases.get(id).map(String::as_str) != Some(passphrase.expose().as_str()) {
                return Err(VolumeError::WrongPassphrase);
            }
            let volume = &mut state.volumes[at];
            volume.locked = false;
            volume.filesystem = Some("ext4".into());
            let point = PathBuf::from("/run/media/mock").join(&volume.label);
            volume.mount_point = Some(point.clone());
            Ok(point)
        }
    }

    /// A [`ShareBackend`] that answers from what a test set.
    pub struct MockShares {
        pub gvfs: Gvfs,
        shares: Mutex<Vec<Share>>,
        /// What `connect` answers, in order; the last repeats.
        answers: Mutex<Vec<Result<Option<PathBuf>, ConnectError>>>,
        connects: Mutex<Vec<(String, Answers)>>,
        gadgets: Mutex<Gadgets>,
        releases: Mutex<Vec<String>>,
    }

    impl MockShares {
        pub fn new(gvfs: Gvfs, shares: Vec<Share>) -> Self {
            MockShares {
                gvfs,
                shares: Mutex::new(shares),
                answers: Mutex::new(vec![Ok(None)]),
                connects: Mutex::new(Vec::new()),
                gadgets: Mutex::new(Gadgets::default()),
                releases: Mutex::new(Vec::new()),
            }
        }

        /// What [`ShareBackend::gadgets`] answers from now on.
        pub fn set_gadgets(&self, gadgets: Gadgets) {
            *self.gadgets.lock().unwrap() = gadgets;
        }

        /// Every `release` so far.
        pub fn releases(&self) -> Vec<String> {
            self.releases.lock().unwrap().clone()
        }

        pub fn answer(&self, answers: Vec<Result<Option<PathBuf>, ConnectError>>) {
            *self.answers.lock().unwrap() = answers;
        }

        /// Every `connect` so far: the address and what it was answered
        /// with.
        pub fn connects(&self) -> Vec<(String, Answers)> {
            self.connects.lock().unwrap().clone()
        }
    }

    #[async_trait::async_trait]
    impl ShareBackend for MockShares {
        async fn gvfs(&self) -> Gvfs {
            self.gvfs.clone()
        }

        async fn shares(&self) -> Vec<Share> {
            self.shares.lock().unwrap().clone()
        }

        async fn connect(&self, uri: &str, answers: Answers) -> Result<Option<PathBuf>, ConnectError> {
            self.connects.lock().unwrap().push((uri.to_string(), answers));
            let mut queue = self.answers.lock().unwrap();
            if queue.len() > 1 {
                queue.remove(0)
            } else {
                queue.first().cloned().unwrap_or(Ok(None))
            }
        }

        async fn disconnect(&self, share: &Share) -> Result<(), String> {
            self.shares.lock().unwrap().retain(|s| s.path != share.path);
            Ok(())
        }

        async fn gadgets(&self) -> Gadgets {
            self.gadgets.lock().unwrap().clone()
        }

        async fn release(&self, uri: &str) -> Result<(), String> {
            self.releases.lock().unwrap().push(uri.to_string());
            for g in &mut self.gadgets.lock().unwrap().list {
                if g.uri == uri {
                    g.mounted = None;
                }
            }
            Ok(())
        }

        async fn watch(&self) -> mpsc::Receiver<()> {
            mpsc::channel(1).1
        }
    }
}
