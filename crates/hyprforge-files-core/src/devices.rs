//! What the sidebar's **Devices** and **Remote** sections know: the
//! drives that can be mounted, the network shares that are, and what is
//! happening to each right now.
//!
//! Plain data and pure decisions over it. The talking — to UDisks2 and
//! gvfs — is the host's, through `hyprforge-volumes`; the browser does no
//! I/O (see [`crate::browser`]). So the shape is the Pinned section's:
//! the *window* owns one [`Devices`], changes it as answers arrive, and
//! hands every tab the same copy through
//! [`Message::DevicesChanged`](crate::browser::Message::DevicesChanged).
//! A click on a drive comes back out as an [`Ask`] in
//! [`Outcome::Devices`](crate::browser::Outcome::Devices) for the host
//! to carry out.
//!
//! The window's, not the tab's, because a mount started from one tab is
//! the same drive in every other: its "Mounting…" has to show
//! everywhere, and two tabs must never both ask to mount it.

pub use hyprforge_volumes::{Gvfs, Operation, Share, ShareKind, Volume, VolumeError, VolumeId};
use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// The drives, as last heard.
#[derive(Debug, Clone, Default, PartialEq)]
pub enum Listing {
    /// Nobody has asked — a host that does not show drives (or has not
    /// heard back yet). The section is absent rather than claiming
    /// there are none.
    #[default]
    NotAsked,
    /// UDisks2 could not be asked: the sentence says why. A state with
    /// a message, never an empty list — a machine whose disk service is
    /// down has not had every drive unplugged.
    Unavailable(String),
    Listed(Vec<Volume>),
}

/// Everything the Devices and Remote sections draw from.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Devices {
    pub volumes: Listing,
    /// What is being done to which volume, until it is done.
    pub busy: BTreeMap<VolumeId, Operation>,
    /// Network shares mounted now.
    pub shares: Vec<Share>,
    /// Shares a disconnect is under way for.
    pub disconnecting: BTreeSet<PathBuf>,
    /// Whether this host changes the session's mounts beyond opening a
    /// drive: the Remote section's "Connect to Server…" and a mounted
    /// drive's eject mark. The window does; the open/save dialog does
    /// not, because a dialog's job is a file — it still mounts a drive
    /// that is clicked, since a stick you cannot open is one you cannot
    /// save to. Data the host sets, so the view needs no idea which host
    /// it is in.
    pub manages_mounts: bool,
}

/// What a click in those sections asks the browser for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeviceMessage {
    /// A drive's row: go there, mounting it first when it is not.
    Open(VolumeId),
    /// The eject mark on a mounted drive's row.
    Eject(VolumeId),
}

/// What the host is asked to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ask {
    /// Mount it — and when `open`, go there in the tab that asked once
    /// it is mounted, which is what clicking an unmounted drive means.
    Mount { id: VolumeId, open: bool },
    Unmount(VolumeId),
    Eject(VolumeId),
    /// Disconnect the share mounted at this path.
    Disconnect(PathBuf),
}

/// What a drive's own action would act on — see
/// [`crate::action::ActionContext::device`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DeviceTarget {
    /// There is a drive to act on at all.
    pub volume: bool,
    pub mounted: bool,
    pub can_eject: bool,
    /// Something is already being done to it.
    pub busy: bool,
    pub locked: bool,
    /// There is a network share to act on.
    pub share: bool,
}

impl Devices {
    /// Every volume listed, or none when there is no list.
    pub fn volumes(&self) -> &[Volume] {
        match &self.volumes {
            Listing::Listed(v) => v,
            _ => &[],
        }
    }

    pub fn volume(&self, id: &VolumeId) -> Option<&Volume> {
        self.volumes().iter().find(|v| &v.id == id)
    }

    /// The volume a sidebar row's target names — its device file when it
    /// is not mounted, its mount point when it is.
    pub fn volume_named(&self, target: &Path) -> Option<&Volume> {
        self.volumes()
            .iter()
            .find(|v| v.device == target || v.mount_point.as_deref() == Some(target))
    }

    /// The volume `path` is on, deepest mount first — what Unmount from
    /// the command palette means while you are browsing a stick.
    pub fn volume_holding(&self, path: &Path) -> Option<&Volume> {
        hyprforge_volumes::inventory::mounted_at(self.volumes(), path)
    }

    /// The share `path` is on.
    pub fn share_holding(&self, path: &Path) -> Option<&Share> {
        self.shares
            .iter()
            .filter(|s| path.starts_with(&s.path))
            .max_by_key(|s| s.path.components().count())
    }

    /// What the drive actions would act on: the menu's own row when there
    /// is one, else whatever the folder in view is on.
    pub fn target(&self, target: Option<&Path>, current_dir: &Path) -> DeviceTarget {
        let volume = match target {
            Some(t) => self.volume_named(t),
            None => self.volume_holding(current_dir),
        };
        let share = match target {
            Some(t) => self.shares.iter().any(|s| s.path == t),
            None => self.share_holding(current_dir).is_some(),
        };
        match volume {
            Some(v) => DeviceTarget {
                volume: true,
                mounted: v.is_mounted(),
                can_eject: v.detach.is_some(),
                busy: self.busy.contains_key(&v.id),
                locked: v.locked,
                share,
            },
            None => DeviceTarget { share, ..DeviceTarget::default() },
        }
    }

    /// A new list from UDisks2. An operation still running keeps its
    /// state, and one on a volume that has gone is forgotten.
    pub fn listed(&mut self, result: Result<Vec<Volume>, VolumeError>) {
        self.volumes = match result {
            Ok(volumes) => {
                self.busy.retain(|id, _| volumes.iter().any(|v| &v.id == id));
                Listing::Listed(volumes)
            }
            Err(VolumeError::Unavailable) => {
                Listing::Unavailable("UDisks2 isn't running, so drives can't be listed.".to_string())
            }
            Err(e) => Listing::Unavailable(format!("Couldn't list drives: {e}")),
        };
    }

    /// Marks `id` as having `op` done to it. `false` when something is
    /// already being done to it — a second mount of the same stick is
    /// not a request worth sending.
    pub fn begin(&mut self, id: &VolumeId, op: Operation) -> bool {
        if self.busy.contains_key(id) {
            return false;
        }
        self.busy.insert(id.clone(), op);
        true
    }

    /// `op` on `id` ended. On success the row changes now rather than
    /// when UDisks2's signal comes round — a mounted stick that still
    /// read "not mounted" for a third of a second after the folder
    /// opened would be a contradiction on screen.
    pub fn finish(&mut self, id: &VolumeId, op: Operation, result: &Result<Option<PathBuf>, VolumeError>) {
        self.busy.remove(id);
        let Listing::Listed(volumes) = &mut self.volumes else { return };
        match (op, result) {
            (Operation::Mount, Ok(Some(point))) => {
                if let Some(v) = volumes.iter_mut().find(|v| &v.id == id) {
                    v.mount_point = Some(point.clone());
                }
            }
            (Operation::Unmount, Ok(_)) => {
                if let Some(v) = volumes.iter_mut().find(|v| &v.id == id) {
                    v.mount_point = None;
                }
            }
            (Operation::Eject, Ok(_)) => volumes.retain(|v| &v.id != id),
            _ => {}
        }
    }

    /// The icon theme names the sections want — asked for once, like
    /// every other icon.
    pub fn icon_names(&self) -> Vec<&'static str> {
        let mut names: Vec<&'static str> = self.volumes().iter().map(Volume::icon_name).collect();
        if !self.shares.is_empty() {
            names.push("folder-remote");
        }
        if self.manages_mounts {
            names.push(CONNECT_ICON);
        }
        names.sort_unstable();
        names.dedup();
        names
    }
}

/// The icon for "Connect to Server…".
pub const CONNECT_ICON: &str = "network-server";

#[cfg(test)]
mod tests {
    use super::*;
    use hyprforge_volumes::{Detach, VolumeKind};

    fn stick(label: &str, mounted: Option<&str>) -> Volume {
        Volume {
            id: VolumeId(format!("/b/{label}")),
            device: PathBuf::from(format!("/dev/{label}")),
            label: label.to_string(),
            size: 1,
            filesystem: Some("vfat".into()),
            mount_point: mounted.map(PathBuf::from),
            kind: VolumeKind::Removable,
            detach: Some(Detach::PowerOff),
            locked: false,
        }
    }

    fn devices(volumes: Vec<Volume>) -> Devices {
        Devices { volumes: Listing::Listed(volumes), ..Devices::default() }
    }

    /// UDisks2 down is a sentence, never an empty list.
    #[test]
    fn a_service_that_is_not_running_is_not_an_empty_list() {
        let mut d = Devices::default();
        d.listed(Err(VolumeError::Unavailable));
        assert!(matches!(&d.volumes, Listing::Unavailable(s) if s.contains("isn't running")));
        d.listed(Ok(vec![]));
        assert_eq!(d.volumes, Listing::Listed(vec![]));
    }

    #[test]
    fn a_volume_already_busy_is_not_asked_twice() {
        let mut d = devices(vec![stick("A", None)]);
        let id = VolumeId("/b/A".into());
        assert!(d.begin(&id, Operation::Mount));
        assert!(!d.begin(&id, Operation::Mount));
        d.finish(&id, Operation::Mount, &Ok(Some("/run/media/u/A".into())));
        assert!(d.busy.is_empty());
        assert_eq!(d.volume(&id).unwrap().mount_point.as_deref(), Some(Path::new("/run/media/u/A")));
    }

    #[test]
    fn a_failure_clears_the_busy_state_and_changes_nothing_else() {
        let mut d = devices(vec![stick("A", Some("/m/A"))]);
        let id = VolumeId("/b/A".into());
        d.begin(&id, Operation::Unmount);
        d.finish(&id, Operation::Unmount, &Err(VolumeError::Busy("x".into())));
        assert!(d.busy.is_empty());
        assert!(d.volume(&id).unwrap().is_mounted(), "a refused unmount leaves it mounted");
    }

    #[test]
    fn an_ejected_volume_leaves_the_list_at_once() {
        let mut d = devices(vec![stick("A", Some("/m/A")), stick("B", None)]);
        d.finish(&VolumeId("/b/A".into()), Operation::Eject, &Ok(None));
        assert_eq!(d.volumes().len(), 1);
    }

    /// A stick unplugged while it was being mounted takes its "Mounting…"
    /// with it, rather than leaving it on a row that is gone.
    #[test]
    fn busy_states_for_volumes_that_vanished_are_forgotten() {
        let mut d = devices(vec![stick("A", None)]);
        d.begin(&VolumeId("/b/A".into()), Operation::Mount);
        d.listed(Ok(vec![]));
        assert!(d.busy.is_empty());
    }

    #[test]
    fn the_drive_actions_act_on_the_row_or_on_where_you_are() {
        let d = devices(vec![stick("A", Some("/run/media/u/A")), stick("B", None)]);
        let row = d.target(Some(Path::new("/dev/B")), Path::new("/home/u"));
        assert!(row.volume && !row.mounted);
        let here = d.target(None, Path::new("/run/media/u/A/photos"));
        assert!(here.volume && here.mounted && here.can_eject);
        assert_eq!(d.target(None, Path::new("/home/u")), DeviceTarget::default());
    }

    #[test]
    fn a_share_is_found_by_any_path_under_it() {
        let d = Devices {
            shares: vec![Share {
                label: "u@box".into(),
                path: "/run/user/1/gvfs/sftp:host=box".into(),
                kind: ShareKind::Gvfs { scheme: "sftp".into() },
            }],
            ..Devices::default()
        };
        assert!(d.share_holding(Path::new("/run/user/1/gvfs/sftp:host=box/srv")).is_some());
        assert!(d.target(None, Path::new("/run/user/1/gvfs/sftp:host=box/srv")).share);
        assert!(!d.target(None, Path::new("/home")).share);
    }
}
