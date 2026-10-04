//! The plain data this crate hands out, and the sentences it says when
//! something does not work.
//!
//! Nothing here talks to anything. A [`Volume`] is what a sidebar row
//! needs to know about a drive, a [`Share`] the same for a server, and
//! the errors keep apart the failures a person has to act on differently
//! — the busy drive, the policy that said no, the service that is not
//! running — because each one gets its own sentence ([`sentence`],
//! [`ConnectError`]'s `Display`) and collapsing two of them would put the
//! wrong advice on screen.

use std::path::PathBuf;
use std::time::Duration;

/// Which volume, across one listing and the next: UDisks2's object path
/// for its block device (`/org/freedesktop/UDisks2/block_devices/sdb1`).
///
/// Not the device file. `/dev/sdb1` is reused by whatever is plugged in
/// next, so an answer about "sdb1" arriving after a swap would land on
/// the wrong stick; the object path is just as reused, but it is what
/// every call takes, and a vanished one fails with "no such object"
/// rather than acting on a stranger.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct VolumeId(pub String);

impl VolumeId {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// What sort of thing a volume is on — which picks its icon and whether
/// "Eject" means anything.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VolumeKind {
    /// A USB stick, a card, an external disk: anything on a drive that
    /// says it can be removed or powered off.
    Removable,
    /// A CD, DVD or Blu-ray.
    Optical,
    /// A disk inside the machine that the system nonetheless offers —
    /// UDisks2 says it is not a system disk.
    Internal,
    /// A disk image this user attached (`udisksctl loop-setup`).
    Loop,
}

/// How "Eject" lets go of a volume, once everything on it is unmounted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Detach {
    /// Power the drive down, so it can be pulled out — a USB stick.
    PowerOff,
    /// Open the tray — an optical drive.
    Eject,
    /// Detach the disk image — a loop device. The image file stays.
    Loop,
}

/// One mountable thing, as a sidebar shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Volume {
    pub id: VolumeId,
    /// `/dev/sdb1`. Shown nowhere, but it is the honest name of an
    /// unmounted volume — the one path it has.
    pub device: PathBuf,
    /// What to call it — see [`crate::inventory::label`].
    pub label: String,
    /// Bytes.
    pub size: u64,
    /// `vfat`, `ntfs`, `ext4`. `None` for an encrypted container, whose
    /// filesystem cannot be known until it is unlocked.
    pub filesystem: Option<String>,
    /// Where it is mounted, or `None` when it is not. A volume mounted in
    /// several places is shown at the first, which is where UDisks2
    /// mounted it.
    pub mount_point: Option<PathBuf>,
    pub kind: VolumeKind,
    /// How Eject lets go of it; `None` when it cannot — an internal disk.
    pub detach: Option<Detach>,
    /// An encrypted container (LUKS, BitLocker) that is not unlocked.
    /// Mounting one means unlocking it first, which this crate does not
    /// do yet — see [`VolumeError::Locked`].
    pub locked: bool,
}

impl Volume {
    pub fn is_mounted(&self) -> bool {
        self.mount_point.is_some()
    }

    /// The freedesktop icon name for it — the names every icon theme
    /// that draws devices uses.
    pub fn icon_name(&self) -> &'static str {
        match self.kind {
            VolumeKind::Optical => "media-optical",
            VolumeKind::Removable => "drive-removable-media",
            VolumeKind::Internal | VolumeKind::Loop => "drive-harddisk",
        }
    }
}

/// Something done to a volume, for a "doing it" state and a sentence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Operation {
    Mount,
    Unmount,
    Eject,
}

impl Operation {
    /// What a row says while it is happening.
    pub fn doing(self) -> &'static str {
        match self {
            Operation::Mount => "Mounting\u{2026}",
            Operation::Unmount => "Unmounting\u{2026}",
            Operation::Eject => "Ejecting\u{2026}",
        }
    }

    fn verb(self) -> &'static str {
        match self {
            Operation::Mount => "open",
            Operation::Unmount => "unmount",
            Operation::Eject => "eject",
        }
    }
}

/// Why something done to a volume did not happen.
///
/// Each variant is a different thing for a person to do about it, which
/// is the test for whether two failures may share one: "close what is
/// open on it" and "you are not allowed" must never read alike.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum VolumeError {
    /// UDisks2 is not on the system bus, or there is no system bus.
    /// Never an empty list — a machine whose disk service is down has
    /// not had every drive unplugged.
    #[error("UDisks2 isn't running")]
    Unavailable,
    /// It did not answer within the bound. The operation may still
    /// finish; this side stopped waiting.
    #[error("UDisks2 didn't answer within {0:?}")]
    TimedOut(Duration),
    /// Something has a file open on it, or a shell is standing in it.
    #[error("the volume is busy: {0}")]
    Busy(String),
    /// The system's policy (polkit) said no.
    #[error("not authorized: {0}")]
    NotAuthorized(String),
    /// A password was asked for and the prompt was dismissed.
    #[error("the authentication prompt was dismissed")]
    Dismissed,
    /// It is not there any more — unplugged while this was happening.
    #[error("the volume is no longer there")]
    Gone,
    /// An encrypted container: unlocking is not built.
    #[error("the volume is encrypted")]
    Locked,
    /// UDisks2 refused for another reason, in its own words.
    #[error("{0}")]
    Refused(String),
}

/// What to tell a person about `error` while doing `op` to `label`.
///
/// One function, so every host says the same thing about the same
/// failure, and so the wording is a thing a test can hold still.
pub fn sentence(op: Operation, label: &str, error: &VolumeError) -> String {
    let name = format!("\u{201C}{label}\u{201D}");
    match error {
        VolumeError::Unavailable => {
            format!("Couldn't {} {name}: UDisks2, the system's disk service, isn't running.", op.verb())
        }
        VolumeError::TimedOut(limit) => match op {
            // An unmount that is slow is nearly always a stick still
            // being written to, and that is the one thing a person
            // must not be told is safe.
            Operation::Unmount | Operation::Eject => format!(
                "{name} is taking a long time to finish writing — wait before unplugging it. (Stopped waiting after {}s.)",
                limit.as_secs()
            ),
            Operation::Mount => format!("{name} didn't open within {}s.", limit.as_secs()),
        },
        VolumeError::Busy(_) => format!(
            "{name} is in use — close the files and terminals open on it, then {} it again.",
            op.verb()
        ),
        VolumeError::NotAuthorized(_) => {
            format!("You aren't allowed to {} {name} — the system's policy said no.", op.verb())
        }
        VolumeError::Dismissed => format!("Didn't {} {name}: the password prompt was cancelled.", op.verb()),
        VolumeError::Gone => format!("{name} is no longer connected."),
        VolumeError::Locked => {
            format!("{name} is encrypted, and unlocking encrypted drives isn't built yet.")
        }
        VolumeError::Refused(why) => format!("Couldn't {} {name}: {why}", op.verb()),
    }
}

/// How a server is reached.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShareKind {
    /// Mounted by gvfs and reached through its FUSE directory
    /// (`/run/user/1000/gvfs/…`). `scheme` is what the address began
    /// with — `smb`, `sftp`.
    Gvfs { scheme: String },
    /// Mounted by the kernel or a FUSE program — `cifs`, `nfs4`,
    /// `fuse.sshfs` — as `/proc/self/mountinfo` lists it.
    Kernel { fstype: String },
}

/// A network share that is mounted, as a sidebar shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Share {
    pub label: String,
    /// Where its files are — a real directory every program can open.
    pub path: PathBuf,
    pub kind: ShareKind,
}

impl Share {
    /// The freedesktop icon name for a share: a folder somewhere else.
    pub fn icon_name(&self) -> &'static str {
        "folder-remote"
    }
}

/// Whether gvfs is here to connect to servers with, and what it can
/// reach.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Gvfs {
    /// `schemes` are what an address may begin with — every backend
    /// gvfs has installed, by `Scheme=`, `Type=` and `SchemeAliases=`.
    Available { schemes: Vec<String> },
    /// Why not, as a sentence for the dialog — never a hidden feature.
    Absent(String),
}

/// Why connecting to a server did not end in a mounted share.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ConnectError {
    /// gvfs, or its `gio` command, is not installed.
    #[error("{0}")]
    Absent(String),
    /// What was typed is not an address.
    #[error("“{0}” isn't a server address. Try something like smb://server/share or sftp://user@host.")]
    NotAnAddress(String),
    /// gvfs has no backend for this kind of address on this machine.
    #[error("This machine's gvfs can't reach {scheme}:// addresses — the backend for them isn't installed.")]
    Unsupported { scheme: String },
    /// The server wants a name and password; ask, and try again.
    #[error("{}", .0.message)]
    NeedsLogin(Login),
    /// The server asked a question (an unknown host key, say); show its
    /// choices, and try again with one.
    #[error("{}", .0.message)]
    Question(Question),
    /// No answer within the bound.
    #[error("The server didn't answer within {}s.", .0.as_secs())]
    TimedOut(Duration),
    /// gio said no, in its own words — "Connection refused".
    #[error("{0}")]
    Failed(String),
}

/// A request for credentials, as the server put it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Login {
    /// The server's own words: "Password required for share x on y".
    pub message: String,
    /// The name gio offered, if it offered one.
    pub user: Option<String>,
    /// The domain gio offered — only Windows shares ask for one.
    pub domain: Option<String>,
    /// Whether it asks for a domain at all.
    pub asks_domain: bool,
    /// Whether this follows answers that were refused — the dialog says
    /// "that didn't work" rather than asking afresh as if nothing
    /// happened.
    pub retry: bool,
}

/// A question from the server, with the choices it offered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Question {
    pub message: String,
    pub choices: Vec<String>,
}

/// What to answer when gio asks. Built by the connect dialog from what
/// the person typed; consumed by one connection attempt.
///
/// `Debug` is safe to print: the password is a
/// [`hyprforge_secret::Secret`], which renders its length.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Answers {
    pub user: Option<String>,
    pub domain: Option<String>,
    pub password: Option<hyprforge_secret::Secret<String>>,
    /// Which of a question's choices, counted from 0.
    pub choice: Option<usize>,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A busy drive and a refused one want different things done about
    /// them; the sentence is the only place that difference reaches a
    /// person.
    #[test]
    fn a_busy_volume_and_a_forbidden_one_never_read_alike() {
        let busy = sentence(Operation::Unmount, "STICK", &VolumeError::Busy("target is busy".into()));
        let denied = sentence(Operation::Unmount, "STICK", &VolumeError::NotAuthorized("no".into()));
        assert!(busy.contains("in use"), "{busy}");
        assert!(denied.contains("aren't allowed"), "{denied}");
        assert_ne!(busy, denied);
    }

    /// The one timeout that must not sound like success: an unmount
    /// that has not finished means the stick is still being written.
    #[test]
    fn a_slow_unmount_never_says_it_is_safe_to_unplug() {
        let slow = sentence(Operation::Eject, "STICK", &VolumeError::TimedOut(Duration::from_secs(300)));
        assert!(slow.contains("wait before unplugging"), "{slow}");
        assert!(!slow.to_lowercase().contains("safe to"), "{slow}");
    }

    #[test]
    fn a_service_that_is_not_running_is_named_as_such() {
        let s = sentence(Operation::Mount, "STICK", &VolumeError::Unavailable);
        assert!(s.contains("UDisks2") && s.contains("isn't running"), "{s}");
    }

    /// The password's `Debug` is its length — `Answers` reaches a log
    /// only through `?answers`, and that must not carry it.
    #[test]
    fn answers_never_print_the_password() {
        let answers = Answers {
            user: Some("bob".into()),
            password: Some(hyprforge_secret::Secret::new("hunter2".to_string())),
            ..Answers::default()
        };
        let printed = format!("{answers:?}");
        assert!(!printed.contains("hunter2"), "{printed}");
    }

    #[test]
    fn a_removable_volume_has_a_removable_icon() {
        let volume = Volume {
            id: VolumeId("/x".into()),
            device: "/dev/sdb1".into(),
            label: "S".into(),
            size: 1,
            filesystem: Some("vfat".into()),
            mount_point: None,
            kind: VolumeKind::Removable,
            detach: Some(Detach::PowerOff),
            locked: false,
        };
        assert_eq!(volume.icon_name(), "drive-removable-media");
        assert!(!volume.is_mounted());
    }
}
