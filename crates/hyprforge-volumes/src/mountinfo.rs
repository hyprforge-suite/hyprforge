//! Network filesystems the kernel has mounted, from `/proc/self/mountinfo`.
//!
//! A share mounted by `mount -t cifs`, an fstab line, autofs or `sshfs`
//! never passes through UDisks2 or gvfs, so neither of those can say it
//! is there. The mount table can: one line per mount, and the filesystem
//! type tells a network share from a disk. Pure — [`parse`] takes the
//! text, so every rule here is tested on lines copied from real tables.

use crate::types::{Share, ShareKind};
use std::path::PathBuf;

/// The filesystem types that mean "somewhere across a network".
///
/// An allow-list rather than "anything not backed by a block device":
/// `tmpfs`, `proc`, `overlay` and a dozen other virtual filesystems are
/// not backed by one either, and a sidebar offering `/sys/fs/cgroup` as
/// a server would be absurd. `fuse.gvfsd-fuse` is deliberately absent —
/// it is the one directory gvfs's own shares live under, listed one by
/// one from there instead (see `crate::gvfs`).
pub const NETWORK_TYPES: &[&str] = &[
    "cifs",
    "smb3",
    "nfs",
    "nfs4",
    "fuse.sshfs",
    "fuse.rclone",
    "davfs",
    "fuse.davfs2",
    "afpfs",
    "fuse.afpfs",
    "9p",
];

/// One line of the mount table, the fields this module reads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mount {
    pub mount_point: PathBuf,
    pub fstype: String,
    /// What was mounted: `//server/share`, `host:/export`,
    /// `user@host:/path`.
    pub source: String,
}

/// Every line of `text` that is well formed. A line that is not is
/// skipped rather than failing the whole table — the kernel has never
/// written one, and one bad line must not hide every share.
pub fn parse(text: &str) -> Vec<Mount> {
    text.lines().filter_map(parse_line).collect()
}

/// `36 35 98:0 /mnt1 /mnt2 rw,noatime master:1 - ext3 /dev/root rw,errors=continue`
/// — the fields before ` - ` vary in number (the optional tags), the
/// three after it do not.
fn parse_line(line: &str) -> Option<Mount> {
    let (before, after) = line.split_once(" - ")?;
    let mount_point = before.split(' ').nth(4)?;
    let mut after = after.split(' ');
    let fstype = after.next()?;
    let source = after.next()?;
    Some(Mount {
        mount_point: PathBuf::from(unescape(mount_point)),
        fstype: unescape(fstype),
        source: unescape(source),
    })
}

/// The kernel writes a space in a path as `\040`, a tab as `\011`, a
/// newline as `\012` and a backslash as `\134` — three octal digits after
/// a backslash. Anything else after a backslash is left as it was.
fn unescape(field: &str) -> String {
    let bytes = field.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\\' {
            let digits = bytes.get(i + 1..i + 4).unwrap_or_default();
            if digits.len() == 3 && digits.iter().all(|d| (b'0'..=b'7').contains(d)) {
                let value = (digits[0] - b'0') as u32 * 64 + (digits[1] - b'0') as u32 * 8 + (digits[2] - b'0') as u32;
                if let Ok(byte) = u8::try_from(value) {
                    out.push(byte);
                    i += 4;
                    continue;
                }
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The network shares in `mounts`, as sidebar rows, labelled by the
/// folder they are mounted on — the name the person (or their fstab)
/// chose, which is how a pinned folder is labelled too.
pub fn network_shares(mounts: &[Mount]) -> Vec<Share> {
    let mut shares: Vec<Share> = mounts
        .iter()
        .filter(|m| NETWORK_TYPES.contains(&m.fstype.as_str()))
        .map(|m| Share {
            label: m
                .mount_point
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| m.source.clone()),
            path: m.mount_point.clone(),
            kind: ShareKind::Kernel { fstype: m.fstype.clone() },
        })
        .collect();
    // The same share mounted twice (a bind of it, a namespace's copy)
    // is one row.
    shares.dedup_by(|a, b| a.path == b.path);
    shares
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    /// Lines in the shape `/proc/self/mountinfo` writes them — the first
    /// two copied from the machine this was written on, the network ones
    /// in the same shape with the fields cifs, nfs4 and sshfs fill in.
    const TABLE: &str = "\
25 1 0:22 /@ / rw,relatime shared:1 - btrfs /dev/nvme1n1p2 rw,ssd,space_cache=v2
621 606 0:75 / /run/user/1000/gvfs rw,nosuid,nodev,relatime shared:598 - fuse.gvfsd-fuse gvfsd-fuse rw,user_id=1000,group_id=1000
700 25 0:80 / /mnt/nas\\040music rw,relatime shared:620 - cifs //nas/music rw,vers=3.1.1
701 25 0:81 / /srv/home rw,relatime shared:621 - nfs4 fileserver:/export/home rw,vers=4.2
702 25 0:82 / /home/u/remote rw,nosuid,nodev,relatime shared:622 - fuse.sshfs u@box:/srv rw,user_id=1000
703 25 0:83 / /tmp/x rw shared:623 - tmpfs tmpfs rw
";

    #[test]
    fn only_network_filesystems_become_shares() {
        let shares = network_shares(&parse(TABLE));
        let paths: Vec<_> = shares.iter().map(|s| s.path.to_string_lossy().into_owned()).collect();
        assert_eq!(paths, ["/mnt/nas music", "/srv/home", "/home/u/remote"]);
    }

    /// gvfs's own FUSE root is listed share by share from inside it; the
    /// root itself as a row would be a "gvfs" folder nobody asked for.
    #[test]
    fn the_gvfs_fuse_root_is_not_a_share() {
        let shares = network_shares(&parse(TABLE));
        assert!(!shares.iter().any(|s| s.path.ends_with("gvfs")));
    }

    #[test]
    fn an_escaped_space_in_a_mount_point_is_a_space() {
        let mounts = parse(TABLE);
        assert!(mounts.iter().any(|m| m.mount_point == Path::new("/mnt/nas music")));
        assert_eq!(unescape("a\\134b\\011c"), "a\\b\tc");
        assert_eq!(unescape("trailing\\04"), "trailing\\04", "not three digits: left alone");
    }

    #[test]
    fn a_share_is_labelled_by_the_folder_it_is_mounted_on() {
        let shares = network_shares(&parse(TABLE));
        assert_eq!(shares[0].label, "nas music");
        assert_eq!(shares[0].kind, ShareKind::Kernel { fstype: "cifs".into() });
    }

    #[test]
    fn a_malformed_line_is_skipped_not_fatal() {
        let text = format!("garbage without a separator\n{TABLE}");
        assert_eq!(network_shares(&parse(&text)).len(), 3);
    }
}
