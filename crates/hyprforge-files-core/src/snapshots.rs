//! Previous versions of a file, from snapper's btrfs snapshots.
//!
//! A machine with snapper keeps copies of a subvolume — `/home`, hourly
//! — as read-only snapshots under `<subvolume>/.snapshots/<N>/snapshot/`,
//! each a whole tree as it was. So an older copy of `~/notes.txt` is at
//! `/home/.snapshots/<N>/snapshot/<you>/notes.txt` for every `N` that
//! has it, and when that snapshot was taken is in `<N>/info.xml`.
//!
//! Most of those copies are the same file: a document changed twice a
//! week is identical in all but a few of a week's 168 hourly snapshots.
//! So candidates are **collapsed by (modified time, size)** into the
//! versions there actually were, each named by the newest snapshot that
//! holds it, and one identical to the file as it is now is not offered
//! at all — "restore it to what it already is" is not a version.
//!
//! This module is pure: which config a path falls under, where its copy
//! would be in snapshot `N`, what an `info.xml` says, and the collapsing.
//! Listing snapshots and `stat`ing candidates is the host's, bounded.
//!
//! Reading `.snapshots` needs snapper to allow this user — Settings → Set
//! up → "Previous versions of your files" — and without that a host
//! reports [`Unreadable`], never an empty list: "you have no previous
//! versions" and "you cannot see them" are different answers.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// One snapper config: the subvolume it snapshots.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    pub name: String,
    pub subvolume: PathBuf,
}

impl Config {
    /// The config in `text` (`/etc/snapper/configs/<name>`), by its
    /// `SUBVOLUME="…"` line; `None` without one.
    pub fn parse(name: &str, text: &str) -> Option<Config> {
        let subvolume = text
            .lines()
            .rev()
            .find_map(|l| l.trim().strip_prefix("SUBVOLUME="))?
            .trim()
            .trim_matches('"');
        (!subvolume.is_empty()).then(|| Config { name: name.to_string(), subvolume: PathBuf::from(subvolume) })
    }

    /// Where its snapshots are.
    pub fn snapshots(&self) -> PathBuf {
        self.subvolume.join(".snapshots")
    }

    /// Where `relative` — a path under the subvolume — is in snapshot
    /// `num`.
    pub fn copy_in(&self, num: u32, relative: &Path) -> PathBuf {
        self.snapshots().join(num.to_string()).join("snapshot").join(relative)
    }
}

/// The config `path` falls under — the one with the longest subvolume
/// that holds it, so `/home/x` is `home`'s even though `/` holds it too
/// — and `path` relative to that subvolume. `None` for a path in no
/// snapshotted subvolume, or one already inside `.snapshots`.
pub fn config_for<'a>(path: &Path, configs: &'a [Config]) -> Option<(&'a Config, PathBuf)> {
    let (config, relative) = configs
        .iter()
        .filter_map(|c| path.strip_prefix(&c.subvolume).ok().map(|rel| (c, rel)))
        .max_by_key(|(c, _)| c.subvolume.components().count())?;
    if relative.starts_with(".snapshots") {
        return None;
    }
    Some((config, relative.to_path_buf()))
}

/// What `<N>/info.xml` says about snapshot `N`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Info {
    pub num: u32,
    /// When it was taken, as snapper writes it: `YYYY-MM-DD HH:MM:SS`,
    /// UTC.
    pub date: String,
    /// "timeline", "boot", or whatever whoever took it called it.
    pub description: String,
}

/// The tags snapper writes; anything else in the file is ignored, and a
/// file missing `num` or `date` is not a snapshot this can name.
pub fn parse_info(xml: &str) -> Option<Info> {
    let tag = |name: &str| -> Option<String> {
        let open = format!("<{name}>");
        let close = format!("</{name}>");
        let start = xml.find(&open)? + open.len();
        let end = start + xml[start..].find(&close)?;
        Some(unescape(xml[start..end].trim()))
    };
    Some(Info { num: tag("num")?.parse().ok()?, date: tag("date")?, description: tag("description").unwrap_or_default() })
}

fn unescape(text: &str) -> String {
    text.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&apos;", "'").replace("&amp;", "&")
}

/// One snapshot's copy of the file, as found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    pub info: Info,
    pub path: PathBuf,
    pub modified: Option<SystemTime>,
    pub size: u64,
}

/// A version there was: the newest snapshot holding it, and how many
/// snapshots in a row held the same.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Version {
    pub newest: Found,
    pub snapshots: usize,
}

/// `found` — every snapshot's copy, in any order — collapsed into the
/// versions there were, newest first, leaving out any identical to the
/// file as it is `now` (its modified time and size).
pub fn versions(mut found: Vec<Found>, now: Option<(Option<SystemTime>, u64)>) -> Vec<Version> {
    found.sort_by_key(|f| std::cmp::Reverse(f.info.num));
    let mut out: Vec<Version> = Vec::new();
    for copy in found {
        let same = |v: &Version| v.newest.modified == copy.modified && v.newest.size == copy.size;
        match out.last_mut() {
            Some(last) if same(last) => last.snapshots += 1,
            _ => out.push(Version { newest: copy, snapshots: 1 }),
        }
    }
    if let Some((modified, size)) = now {
        out.retain(|v| !(v.newest.modified == modified && v.newest.size == size));
    }
    out
}

/// Why there is no list to show.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Unreadable {
    /// No snapper config covers this path.
    NotSnapshotted,
    /// snapper does not let this user read the snapshots — Set up's item.
    NotAllowed,
    /// Something else, in words.
    Failed(String),
}

impl Unreadable {
    /// The sentence the window shows.
    pub fn message(&self) -> String {
        match self {
            Unreadable::NotSnapshotted => "This isn't anywhere snapper keeps snapshots of.".to_string(),
            Unreadable::NotAllowed => "Your snapshots can't be read yet — Settings → Set up → \u{201C}Previous versions of your files\u{201D} allows it.".to_string(),
            Unreadable::Failed(why) => why.clone(),
        }
    }
}

/// The name a restored copy gets beside the file, so restoring never
/// replaces anything: `notes (from 2026-10-05 14.00).txt`. The date is
/// the snapshot's, minutes only; `:` is left out of names for the
/// programs that choke on it.
pub fn restored_name(name: &str, date: &str) -> String {
    let short: String = date.chars().take(16).collect::<String>().replace(':', ".");
    let (stem, ext) = match name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() => (stem, Some(ext)),
        _ => (name, None),
    };
    match ext {
        Some(ext) => format!("{stem} (from {short}).{ext}"),
        None => format!("{stem} (from {short})"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn configs() -> Vec<Config> {
        vec![
            Config { name: "root".into(), subvolume: PathBuf::from("/") },
            Config { name: "home".into(), subvolume: PathBuf::from("/home") },
        ]
    }

    #[test]
    fn a_path_belongs_to_the_deepest_config_that_holds_it() {
        let configs = configs();
        let (config, rel) = config_for(Path::new("/home/alex/notes.txt"), &configs).unwrap();
        assert_eq!(config.name, "home");
        assert_eq!(rel, PathBuf::from("alex/notes.txt"));
        assert_eq!(config_for(Path::new("/etc/hosts"), &configs).unwrap().0.name, "root");
        assert!(config_for(Path::new("/home/.snapshots/3/snapshot/x"), &configs).is_none(), "not a snapshot of a snapshot");
    }

    #[test]
    fn a_copy_in_a_snapshot_is_under_its_number() {
        let home = &configs()[1];
        assert_eq!(home.copy_in(42, Path::new("alex/notes.txt")), PathBuf::from("/home/.snapshots/42/snapshot/alex/notes.txt"));
    }

    #[test]
    fn the_config_file_names_its_subvolume() {
        let home = Config::parse("home", "SUBVOLUME=\"/home\"\nFSTYPE=\"btrfs\"\n").unwrap();
        assert_eq!(home.subvolume, PathBuf::from("/home"));
        assert!(Config::parse("x", "FSTYPE=\"btrfs\"\n").is_none());
    }

    fn found(num: u32, modified: u64, size: u64) -> Found {
        Found {
            info: Info { num, date: format!("2026-10-0{} 10:00:00", num % 9), description: "timeline".into() },
            path: PathBuf::from(format!("/home/.snapshots/{num}/snapshot/a")),
            modified: Some(SystemTime::UNIX_EPOCH + Duration::from_secs(modified)),
            size,
        }
    }

    /// 168 hourly copies of a file changed twice are three versions, not
    /// 168 rows.
    #[test]
    fn identical_copies_in_a_row_are_one_version_named_by_the_newest() {
        let list = vec![found(1, 100, 5), found(2, 100, 5), found(3, 200, 7), found(4, 200, 7), found(5, 300, 9)];
        let v = versions(list, None);
        assert_eq!(v.iter().map(|v| (v.newest.info.num, v.snapshots)).collect::<Vec<_>>(), [(5, 1), (4, 2), (2, 2)]);
    }

    #[test]
    fn a_copy_identical_to_the_file_now_is_not_a_version() {
        let now = Some((Some(SystemTime::UNIX_EPOCH + Duration::from_secs(300)), 9));
        let v = versions(vec![found(4, 200, 7), found(5, 300, 9)], now);
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].newest.info.num, 4);
    }

    #[test]
    fn a_restored_copy_never_takes_the_files_own_name() {
        assert_eq!(restored_name("notes.txt", "2026-10-05 14:00:00"), "notes (from 2026-10-05 14.00).txt");
        assert_eq!(restored_name("Makefile", "2026-10-05 14:00:00"), "Makefile (from 2026-10-05 14.00)");
        assert_eq!(restored_name(".bashrc", "2026-10-05 14:00:00"), ".bashrc (from 2026-10-05 14.00)");
    }

    /// Written from snapper's documented format; replaced by a real
    /// `info.xml` once one could be read — see the module doc.
    #[test]
    fn info_xml_says_the_number_the_date_and_the_description() {
        let xml = "<?xml version=\"1.0\"?>\n<snapshot>\n  <type>single</type>\n  <num>42</num>\n  <date>2026-10-05 14:00:01</date>\n  <description>timeline</description>\n  <cleanup>timeline</cleanup>\n</snapshot>\n";
        assert_eq!(parse_info(xml), Some(Info { num: 42, date: "2026-10-05 14:00:01".into(), description: "timeline".into() }));
        assert_eq!(parse_info("<snapshot><num>x</num></snapshot>"), None);
    }
}
