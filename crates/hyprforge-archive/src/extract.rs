//! Deciding what an extraction will write, and then writing it.
//!
//! Split in two on purpose. [`plan`] is a pure function over an
//! [`Index`] — which members a selection brings, what each one is called
//! at the destination, and whether any of them would land outside it —
//! and it is where every rule worth testing lives. [`run`] does the I/O
//! and knows almost nothing.
//!
//! That split is also what lets the mock backend extract through exactly
//! the same code as the real ones, so the two cannot disagree about the
//! rules.
//!
//! # The guard that must not be clever
//!
//! An archive is a list of filenames supplied by whoever built it, and
//! `../../../etc/cron.d/x` is a filename. [`crate::model::normalise`]
//! already drops a member like that when the index is built, and this
//! module checks again anyway, against the *destination* this time. Two
//! independent guards rather than one, because the cost of being wrong
//! here is writing an arbitrary file to an arbitrary place, and because
//! the first guard exists to make a tree drawable while this one exists
//! to keep a write inside a directory — related, but not the same
//! question, and a future change to either has no business quietly
//! disabling the other.

use crate::backend::{Advance, Collision, ExtractReport, ExtractRequest, Flow, Progress};
use crate::error::{ArchiveError, Result};
use crate::model::{normalise, Index};
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};

/// One member, and where it is going.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanItem {
    pub member: String,
    pub is_dir: bool,
    pub size: u64,
    pub mode: Option<u32>,
    pub link_target: Option<String>,
    /// The absolute destination path, already checked to be inside the
    /// destination directory.
    pub dest: PathBuf,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Plan {
    pub items: Vec<PlanItem>,
    pub bytes: u64,
}

impl Plan {
    pub fn files(&self) -> usize {
        self.items.iter().filter(|i| !i.is_dir).count()
    }
}

/// Works out everything an extraction would write.
///
/// Fails as a whole on an unsafe member rather than skipping it: an
/// archive that tries to write outside the folder someone chose is not a
/// normal archive with one bad row in it, and half-extracting one leaves
/// somebody deciding whether to trust the other half.
pub fn plan(index: &Index, request: &ExtractRequest, archive: &Path) -> Result<Plan> {
    let selected = select(index, &request.members, archive)?;

    let mut items = Vec::new();
    let mut bytes = 0u64;
    for member in selected {
        let Some(relative) = strip(&member.path, request.strip_prefix.as_deref()) else {
            // The stripped prefix itself — the folder someone asked to
            // extract the *contents* of. It has no destination of its
            // own; the destination directory is standing in for it.
            continue;
        };
        let dest = safe_join(&request.dest, &relative).ok_or_else(|| {
            ArchiveError::UnsafeMemberPath {
                archive: archive.to_path_buf(),
                member: member.path.clone(),
            }
        })?;
        if !member.is_dir {
            bytes += member.size;
        }
        items.push(PlanItem {
            member: member.path.clone(),
            is_dir: member.is_dir,
            size: member.size,
            mode: member.mode,
            link_target: member.link_target.clone(),
            dest,
        });
    }

    // Parents before children, so a directory exists before anything is
    // written into it — true of a plain lexicographic sort over
    // `/`-separated paths, since a prefix sorts before anything
    // extending it.
    items.sort_by(|a, b| a.member.cmp(&b.member));
    Ok(Plan { items, bytes })
}

/// The members a selection covers.
///
/// A directory brings everything under it. An empty selection is
/// everything in the archive — "Extract Here" on the archive itself.
fn select<'a>(
    index: &'a Index,
    wanted: &[String],
    archive: &Path,
) -> Result<Vec<&'a crate::model::Member>> {
    if wanted.is_empty() {
        return Ok(index.members().iter().collect());
    }

    let mut chosen: Vec<&crate::model::Member> = Vec::new();
    let mut seen: std::collections::HashSet<&str> = std::collections::HashSet::new();
    for raw in wanted {
        let path = normalise(raw).ok_or_else(|| ArchiveError::MemberNotFound {
            archive: archive.to_path_buf(),
            member: raw.clone(),
        })?;
        let member = index.get(&path).ok_or_else(|| ArchiveError::MemberNotFound {
            archive: archive.to_path_buf(),
            member: raw.clone(),
        })?;
        if seen.insert(&member.path) {
            chosen.push(member);
        }
        if member.is_dir {
            let under = format!("{path}/");
            for candidate in index.members() {
                if candidate.path.starts_with(&under) && seen.insert(&candidate.path) {
                    chosen.push(candidate);
                }
            }
        }
    }
    Ok(chosen)
}

/// `member` with `prefix` removed from the front.
///
/// `None` when `member` *is* the prefix — it has no path left, and is
/// the directory being extracted the contents of rather than a thing to
/// write. A member outside the prefix entirely keeps its whole path;
/// that only happens when a caller strips a prefix from a selection that
/// does not share one, which is theirs to mean.
fn strip(member: &str, prefix: Option<&str>) -> Option<String> {
    let Some(prefix) = prefix.and_then(normalise) else {
        return Some(member.to_string());
    };
    if member == prefix {
        return None;
    }
    match member.strip_prefix(&format!("{prefix}/")) {
        Some(rest) => Some(rest.to_string()),
        None => Some(member.to_string()),
    }
}

/// `dest` joined with `relative`, or `None` if the result would not be
/// inside `dest`.
///
/// Lexical, and that is the point: `std::fs::canonicalize` cannot be
/// used here because the path does not exist yet, and the variants that
/// resolve what *does* exist would follow a symlink an earlier member of
/// the same archive just planted. Refusing every `..`, every root and
/// every prefix outright needs nothing to exist and cannot be walked
/// around.
fn safe_join(dest: &Path, relative: &str) -> Option<PathBuf> {
    let relative = Path::new(relative);
    for component in relative.components() {
        match component {
            Component::Normal(_) => {}
            // `.` cannot appear after `normalise`, and the rest are the
            // escape: an absolute path, a Windows prefix, or a climb.
            _ => return None,
        }
    }
    let joined = dest.join(relative);
    joined.starts_with(dest).then_some(joined)
}

/// Carries out a [`Plan`], one member at a time.
///
/// A struct rather than one loop, because the two kinds of archive want
/// to drive it from opposite ends. A zip or a 7z can be *pulled* from —
/// ask for a member by name and it is there — and extracts through
/// [`run`], which walks the plan in order. A tar has no index at all, so
/// asking it for members one at a time re-reads (and re-decompresses)
/// the whole stream per member, turning an extraction into an O(n²) one;
/// it *pushes* instead, walking its own entries once and handing each to
/// [`Run::item`] as it passes.
///
/// Both go through the same writer, so the collision rule, the symlink
/// refusal, the permission masking and the "one bad member is a row in
/// the report, not the end of the extraction" rule cannot differ between
/// the format that pulls and the format that pushes.
pub struct Run<'a> {
    request: &'a ExtractRequest,
    report: ExtractReport,
    files_total: usize,
    bytes_total: u64,
}

impl<'a> Run<'a> {
    pub fn new(plan: &Plan, request: &'a ExtractRequest) -> Result<Run<'a>> {
        std::fs::create_dir_all(&request.dest)
            .map_err(|e| ArchiveError::io(request.dest.clone(), e))?;
        Ok(Run {
            request,
            report: ExtractReport::default(),
            files_total: plan.files(),
            bytes_total: plan.bytes,
        })
    }

    /// Writes one planned member. `open` supplies its contents, and is
    /// only called if the member is actually going to be written — so a
    /// collision that resolves to "skip" costs no decompression at all.
    pub fn item<'r>(
        &mut self,
        item: &PlanItem,
        progress: &mut dyn Progress,
        open: impl FnOnce() -> Result<Box<dyn Read + 'r>>,
    ) -> Result<()> {
        if progress.advance(Advance {
            member: &item.member,
            files_done: self.report.files,
            files_total: self.files_total,
            bytes_done: self.report.bytes,
            bytes_total: self.bytes_total,
        }) == Flow::Cancel
        {
            return Err(ArchiveError::Cancelled);
        }

        if item.is_dir {
            match std::fs::create_dir_all(&item.dest) {
                Ok(()) => self.report.dirs += 1,
                Err(e) => self.report.failed.push((item.member.clone(), e.to_string())),
            }
            return Ok(());
        }

        let Some(dest) = resolve_collision(&item.dest, self.request.collision) else {
            self.report.skipped.push(item.member.clone());
            return Ok(());
        };

        // The member's own parent, not the destination root: a member
        // whose directories the archive never listed still has to land
        // somewhere. `plan` sorts parents first, so this is usually
        // already there — and a tar pushing entries in its own order is
        // exactly the case where it is not.
        if let Some(parent) = dest.parent() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                self.report.failed.push((item.member.clone(), e.to_string()));
                return Ok(());
            }
        }

        if let Some(target) = &item.link_target {
            match write_link(target, &dest) {
                Ok(()) => self.report.files += 1,
                Err(why) => self.report.failed.push((item.member.clone(), why)),
            }
            return Ok(());
        }

        // One member's failure is a row in `failed`, never the end of
        // the extraction — the rule `StdBackend::read_dir` follows for a
        // directory entry it cannot describe. A cancel is the exception:
        // it is the person's answer, not this member's problem.
        match open().and_then(|mut source| write_file(&dest, &mut source, item.mode)) {
            Ok(written) => {
                self.report.files += 1;
                self.report.bytes += written;
            }
            Err(e) if e.cancelled() => return Err(e),
            Err(e) => self.report.failed.push((item.member.clone(), e.to_string())),
        }
        Ok(())
    }

    pub fn finish(self) -> ExtractReport {
        self.report
    }
}

/// Carries out a whole [`Plan`] by pulling each member in turn — the
/// random-access path. See [`Run`] for why a tar does not use this.
pub fn run(
    plan: &Plan,
    request: &ExtractRequest,
    progress: &mut dyn Progress,
    read: impl Fn(&str) -> Result<Vec<u8>>,
) -> Result<ExtractReport> {
    let mut run = Run::new(plan, request)?;
    for item in &plan.items {
        run.item(item, progress, || {
            let bytes = read(&item.member)?;
            Ok(Box::new(std::io::Cursor::new(bytes)) as Box<dyn Read>)
        })?;
    }
    Ok(run.finish())
}

/// Where this member actually gets written, given what is already
/// there. `None` means skip it.
fn resolve_collision(dest: &Path, collision: Collision) -> Option<PathBuf> {
    // `symlink_metadata`, so a *broken* symlink sitting at the
    // destination still counts as something being in the way. Following
    // it would report "nothing there", and the write would then go
    // through the link to wherever it points — which is the same
    // escape the path guard above exists to prevent, arriving through
    // the destination directory instead of through the archive.
    if std::fs::symlink_metadata(dest).is_err() {
        return Some(dest.to_path_buf());
    }
    match collision {
        Collision::Overwrite => Some(dest.to_path_buf()),
        Collision::Skip => None,
        Collision::Rename => Some(free_name(dest)),
    }
}

/// `notes.txt` -> `notes (1).txt`, counting up until nothing is there.
fn free_name(dest: &Path) -> PathBuf {
    let parent = dest.parent().unwrap_or(Path::new("."));
    let name = dest.file_name().unwrap_or_default().to_string_lossy().into_owned();
    // Split on the *first* dot after the stem so `archive.tar.gz`
    // becomes `archive (1).tar.gz` rather than `archive.tar (1).gz`.
    let (stem, extension) = match name.find('.') {
        // A leading dot is the whole name of a dotfile, not an
        // extension: `.bashrc` is `.bashrc (1)`.
        Some(0) | None => (name.as_str(), ""),
        Some(cut) => (&name[..cut], &name[cut..]),
    };
    for n in 1u32.. {
        let candidate = parent.join(format!("{stem} ({n}){extension}"));
        if std::fs::symlink_metadata(&candidate).is_err() {
            return candidate;
        }
    }
    unreachable!("the loop returns as soon as a name is free")
}

/// Streams `source` into a new file at `dest`.
///
/// Streamed rather than read into a `Vec` first, because the size of a
/// member is whatever the archive says it is: a disk image inside a
/// tarball is a perfectly ordinary thing to extract and must not need to
/// fit in memory to do it. CLAUDE.md's rule about asking what a change
/// allocates rather than only whether it works — the lock screen's
/// 296MB wallpaper, in a different shape.
fn write_file(dest: &Path, source: &mut dyn Read, mode: Option<u32>) -> Result<u64> {
    let file = std::fs::File::create(dest).map_err(|e| ArchiveError::io(dest, e))?;
    let mut file = std::io::BufWriter::new(file);
    let written = std::io::copy(source, &mut file).map_err(|e| ArchiveError::io(dest, e))?;
    file.flush().map_err(|e| ArchiveError::io(dest, e))?;
    drop(file);

    // Only the permission bits, and only the ones a normal umask would
    // allow anyway: setuid and setgid out of an archive are a way to
    // leave a root shell in someone's Downloads folder, and no file
    // manager's extract button should be able to create one.
    #[cfg(unix)]
    if let Some(mode) = mode {
        use std::os::unix::fs::PermissionsExt;
        let safe = mode & 0o777;
        let _ = std::fs::set_permissions(dest, std::fs::Permissions::from_mode(safe));
    }
    #[cfg(not(unix))]
    let _ = mode;

    Ok(written)
}

/// Writes a symlink member, refusing one that points out of the
/// extraction.
///
/// An absolute target, or a relative one that climbs past the
/// destination, is the second half of the traversal attack: the member
/// itself lands safely inside, and the *next* member is written through
/// it to wherever it points. Refused as a failed row, so the rest of the
/// archive still extracts and the reason is reported.
fn write_link(target: &str, dest: &Path) -> std::result::Result<(), String> {
    let relative = Path::new(target);
    if relative.is_absolute() {
        return Err(format!("points outside the folder, at {target}"));
    }
    if normalise(target).is_none() {
        return Err(format!("points outside the folder, at {target}"));
    }
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(relative, dest).map_err(|e| e.to_string())
    }
    #[cfg(not(unix))]
    {
        let _ = dest;
        Err("symlinks aren't supported here".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::NoProgress;
    use crate::model::Member;

    fn request(dest: &Path) -> ExtractRequest {
        ExtractRequest {
            members: Vec::new(),
            dest: dest.to_path_buf(),
            strip_prefix: None,
            collision: Collision::Overwrite,
        }
    }

    #[test]
    fn selecting_a_directory_brings_everything_under_it() {
        let index = Index::build(vec![
            Member::file("docs/guide.txt", 10),
            Member::file("docs/deep/more.txt", 20),
            Member::file("elsewhere.txt", 30),
        ]);
        let mut req = request(Path::new("/out"));
        req.members = vec!["docs".to_string()];

        let plan = plan(&index, &req, Path::new("/a.zip")).unwrap();
        let members: Vec<&str> = plan.items.iter().map(|i| i.member.as_str()).collect();
        assert_eq!(members, ["docs", "docs/deep", "docs/deep/more.txt", "docs/guide.txt"]);
        assert!(!members.contains(&"elsewhere.txt"));
        assert_eq!(plan.bytes, 30, "only the two files under docs");
    }

    #[test]
    fn stripping_a_prefix_extracts_a_folders_contents_rather_than_the_folder() {
        let index = Index::build(vec![Member::file("docs/guide.txt", 10)]);
        let mut req = request(Path::new("/out"));
        req.members = vec!["docs".to_string()];
        req.strip_prefix = Some("docs".to_string());

        let plan = plan(&index, &req, Path::new("/a.zip")).unwrap();
        let dests: Vec<&Path> = plan.items.iter().map(|i| i.dest.as_path()).collect();
        assert_eq!(dests, [Path::new("/out/guide.txt")], "the folder itself has no destination");
    }

    #[test]
    fn a_plan_lists_parents_before_the_things_inside_them() {
        let index = Index::build(vec![
            Member::file("z/deep/last.txt", 1),
            Member::file("a.txt", 1),
        ]);
        let plan = plan(&index, &request(Path::new("/out")), Path::new("/a.zip")).unwrap();
        let members: Vec<&str> = plan.items.iter().map(|i| i.member.as_str()).collect();
        let z = members.iter().position(|m| *m == "z").unwrap();
        let deep = members.iter().position(|m| *m == "z/deep").unwrap();
        let last = members.iter().position(|m| *m == "z/deep/last.txt").unwrap();
        assert!(z < deep && deep < last);
    }

    /// The guard that must hold even though `normalise` already dropped
    /// the member — see the module doc on why there are two.
    #[test]
    fn nothing_can_be_planned_outside_the_destination() {
        assert_eq!(safe_join(Path::new("/out"), "../etc/passwd"), None);
        assert_eq!(safe_join(Path::new("/out"), "/etc/passwd"), None);
        assert_eq!(
            safe_join(Path::new("/out"), "docs/guide.txt"),
            Some(PathBuf::from("/out/docs/guide.txt"))
        );
    }

    #[test]
    fn a_name_that_climbs_out_never_reaches_a_listing_in_the_first_place() {
        let index = Index::build(vec![Member::file("../../etc/passwd", 1)]);
        assert_eq!(index.members().len(), 0);
    }

    #[test]
    fn a_double_extension_keeps_both_halves_when_renamed_around_a_collision() {
        let dir = tempfile::tempdir().unwrap();
        let taken = dir.path().join("archive.tar.gz");
        std::fs::write(&taken, b"x").unwrap();
        assert_eq!(free_name(&taken), dir.path().join("archive (1).tar.gz"));
    }

    #[test]
    fn a_dotfile_is_renamed_after_its_whole_name_not_inside_it() {
        let dir = tempfile::tempdir().unwrap();
        let taken = dir.path().join(".bashrc");
        std::fs::write(&taken, b"x").unwrap();
        assert_eq!(free_name(&taken), dir.path().join(".bashrc (1)"));
    }

    #[test]
    fn skipping_a_collision_reports_what_it_left_alone() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("out");
        std::fs::create_dir(&out).unwrap();
        std::fs::write(out.join("notes.txt"), b"mine").unwrap();

        let index = Index::build(vec![Member::file("notes.txt", 5)]);
        let mut req = request(&out);
        req.collision = Collision::Skip;
        let plan = plan(&index, &req, Path::new("/a.zip")).unwrap();

        let report = run(&plan, &req, &mut NoProgress, |_| Ok(b"theirs".to_vec())).unwrap();
        assert_eq!(report.skipped, ["notes.txt"]);
        assert_eq!(report.files, 0);
        assert_eq!(std::fs::read(out.join("notes.txt")).unwrap(), b"mine");
    }

    #[test]
    fn one_member_that_will_not_read_does_not_lose_the_others() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("out");
        let index = Index::build(vec![
            Member::file("good.txt", 4),
            Member::file("bad.txt", 4),
            Member::file("also-good.txt", 4),
        ]);
        let req = request(&out);
        let plan = plan(&index, &req, Path::new("/a.zip")).unwrap();

        let report = run(&plan, &req, &mut NoProgress, |member| {
            if member == "bad.txt" {
                Err(ArchiveError::Damaged {
                    path: "/a.zip".into(),
                    format: "zip",
                    detail: "truncated".into(),
                })
            } else {
                Ok(b"fine".to_vec())
            }
        })
        .unwrap();

        assert_eq!(report.files, 2);
        assert_eq!(report.failed.len(), 1);
        assert_eq!(report.failed[0].0, "bad.txt");
        assert!(out.join("good.txt").exists() && out.join("also-good.txt").exists());
    }

    #[test]
    fn cancelling_stops_the_extraction_and_says_so() {
        struct StopAfterFirst(usize);
        impl Progress for StopAfterFirst {
            fn advance(&mut self, _advance: Advance<'_>) -> Flow {
                self.0 += 1;
                if self.0 > 1 { Flow::Cancel } else { Flow::Continue }
            }
        }

        let dir = tempfile::tempdir().unwrap();
        let index = Index::build(vec![Member::file("a.txt", 1), Member::file("b.txt", 1)]);
        let req = request(dir.path());
        let plan = plan(&index, &req, Path::new("/a.zip")).unwrap();

        let err = run(&plan, &req, &mut StopAfterFirst(0), |_| Ok(b"x".to_vec()))
            .expect_err("a cancel is not a successful extraction");
        assert!(err.cancelled());
    }

    #[test]
    fn a_symlink_member_pointing_out_of_the_extraction_is_refused_not_followed() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("out");
        let mut link = Member::file("escape", 0);
        link.link_target = Some("../../../etc".to_string());
        let mut inner = Member::file("safe", 0);
        inner.link_target = Some("target.txt".to_string());
        let index = Index::build(vec![link, inner, Member::file("target.txt", 1)]);

        let req = request(&out);
        let plan = plan(&index, &req, Path::new("/a.tar")).unwrap();
        let report = run(&plan, &req, &mut NoProgress, |_| Ok(b"x".to_vec())).unwrap();

        assert_eq!(report.failed.len(), 1);
        assert_eq!(report.failed[0].0, "escape");
        assert!(!out.join("escape").exists());
        assert!(out.join("safe").exists(), "a link that stays inside is written");
    }

    #[test]
    fn a_member_asked_for_by_a_name_that_is_not_there_says_which_name() {
        let index = Index::build(vec![Member::file("a.txt", 1)]);
        let mut req = request(Path::new("/out"));
        req.members = vec!["nope.txt".to_string()];
        let err = plan(&index, &req, Path::new("/a.zip")).unwrap_err();
        assert!(err.to_string().contains("nope.txt"));
    }
}
