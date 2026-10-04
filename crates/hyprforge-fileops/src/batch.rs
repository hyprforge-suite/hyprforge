//! Renaming a set of things at once, each beside itself.
//!
//! A batch is not a loop over single renames, for two reasons.
//!
//! **Order matters, and sometimes no order works.** Renaming `a` to `b`
//! and `b` to `c` has to do `b` first, or `a` lands on a name that is
//! still taken. Renaming `a` to `b` and `b` to `a` has no order at all: a
//! swap is a cycle, and the only way through one is to move one member
//! aside to a temporary name in the same folder first. [`plan`] works
//! the order out — and the temporary names — before anything touches a
//! disk, so it is pure and tested on its own, and the archive editor
//! uses the very same plan for member names that never touch one: an
//! archive's renames are applied to its table one after another too,
//! and a swap there collapses both members onto one name just as surely.
//!
//! **A batch that stops halfway is worse than one that did nothing.**
//! Half a folder renamed to `Holiday 001…` and half still `IMG_…` is a
//! mess nobody can easily see the edges of. [`apply`] checks the whole
//! set against the disk first (every source still there, every target
//! free or about to be freed), and if a rename fails anyway — permission,
//! a name that appeared a moment ago — it puts back every rename it had
//! already done, newest first, and reports exactly what it could not put
//! back. Nothing is left under a temporary name without being named in
//! that report.
//!
//! Every rename here refuses to replace anything (`RENAME_NOREPLACE`),
//! which `std::fs::rename` cannot be asked to do: the check before a
//! batch is only a check, and something can appear between it and the
//! rename. The kernel's refusal is the guarantee; the check is what lets
//! the person hear about it before anything moved.

use std::collections::{HashMap, HashSet};
use std::ffi::CString;
use std::io;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

/// One `rename(2)`, in the order [`plan`] decided.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Step {
    pub from: PathBuf,
    pub to: PathBuf,
}

/// Why a set of renames cannot be planned at all.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PlanError {
    /// Two renames would end at the same name.
    #[error("two things would both be called \u{201C}{}\u{201D}", name(.0))]
    SameTarget(PathBuf),
    /// The same thing is named twice as a source.
    #[error("\u{201C}{}\u{201D} is in the list twice", name(.0))]
    SameSource(PathBuf),
    /// A rename that would put something in another folder — that is a
    /// move, and a move can cross filesystems, which a rename cannot.
    #[error("\u{201C}{}\u{201D} would leave its folder", name(.0))]
    LeavesFolder(PathBuf),
}

fn name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

/// The order to carry out `moves` in, as `(from, to)` pairs, with a swap
/// or longer cycle broken by a temporary name.
///
/// `taken` says whether a path is occupied by something outside the
/// batch — for the disk, whether it exists; for an archive, whether the
/// listing has it. It is only asked about temporary names: whether the
/// *targets* are free is the caller's question, answered before this
/// (and, on a disk, again by the kernel).
///
/// A pair whose two sides are equal is left out — renaming something to
/// its own name is not a step.
///
/// Every step's `to` is free at the moment it runs, given the steps
/// before it: that is the whole contract, and the tests check it by
/// simulating the folder.
pub fn plan(moves: &[(PathBuf, PathBuf)], taken: impl Fn(&Path) -> bool) -> Result<Vec<Step>, PlanError> {
    let moves: Vec<(PathBuf, PathBuf)> = moves.iter().filter(|(from, to)| from != to).cloned().collect();

    let mut sources = HashSet::new();
    let mut targets = HashSet::new();
    for (from, to) in &moves {
        if from.parent() != to.parent() {
            return Err(PlanError::LeavesFolder(from.clone()));
        }
        if !sources.insert(from.clone()) {
            return Err(PlanError::SameSource(from.clone()));
        }
        if !targets.insert(to.clone()) {
            return Err(PlanError::SameTarget(to.clone()));
        }
    }

    // Where each pending rename currently has its thing, and which
    // pending rename is waiting for a given path to be freed. Targets are
    // distinct, so at most one waits on any path.
    let mut at: Vec<PathBuf> = moves.iter().map(|(from, _)| from.clone()).collect();
    let mut occupant: HashMap<PathBuf, usize> = moves.iter().enumerate().map(|(i, (from, _))| (from.clone(), i)).collect();
    let waiting_on: HashMap<PathBuf, usize> = moves.iter().enumerate().map(|(i, (_, to))| (to.clone(), i)).collect();

    let mut done = vec![false; moves.len()];
    let mut steps = Vec::with_capacity(moves.len());
    // Ready: renames whose target nobody in the batch is standing on.
    let mut ready: Vec<usize> = (0..moves.len()).filter(|&i| !occupant.contains_key(&moves[i].1)).collect();
    // Reversed so they run in the order they were given, which keeps the
    // plan stable and readable in a test.
    ready.reverse();
    let mut used_temporaries: HashSet<PathBuf> = HashSet::new();
    let mut remaining = moves.len();

    while remaining > 0 {
        if let Some(i) = ready.pop() {
            let from = at[i].clone();
            let to = moves[i].1.clone();
            steps.push(Step { from: from.clone(), to: to.clone() });
            occupant.remove(&from);
            done[i] = true;
            remaining -= 1;
            // Whoever was waiting for the path just vacated can go now.
            if let Some(&j) = waiting_on.get(&from) {
                if !done[j] {
                    ready.push(j);
                }
            }
            continue;
        }
        // Nothing is ready, so everything left is in a cycle: every
        // remaining target is held by another remaining source. Step the
        // first one aside, which frees its place for the rename waiting
        // on it, and the cycle unrolls from there.
        let i = (0..moves.len()).find(|&i| !done[i]).expect("remaining > 0");
        let from = at[i].clone();
        let temporary = temporary_for(&from, |p| {
            taken(p) || sources.contains(p) || targets.contains(p) || used_temporaries.contains(p)
        });
        used_temporaries.insert(temporary.clone());
        steps.push(Step { from: from.clone(), to: temporary.clone() });
        occupant.remove(&from);
        occupant.insert(temporary.clone(), i);
        at[i] = temporary;
        if let Some(&j) = waiting_on.get(&from) {
            if !done[j] {
                ready.push(j);
            }
        }
    }
    Ok(steps)
}

/// A name beside `path` for it to wait under while a cycle is broken.
///
/// Hidden, so another file manager glancing at the folder mid-batch does
/// not show a stranger in it — and with the original name inside it, so
/// that if one is ever left behind (a failure [`apply`] could not undo,
/// which its report names) a person who finds it knows what it was.
fn temporary_for(path: &Path, taken: impl Fn(&Path) -> bool) -> PathBuf {
    let original = name(path);
    (0..)
        .map(|n| {
            let candidate = if n == 0 {
                format!(".{original}.renaming")
            } else {
                format!(".{original}.renaming-{n}")
            };
            path.with_file_name(candidate)
        })
        .find(|candidate| !taken(candidate))
        .expect("an unbounded range always finds a free name")
}

/// Where a batch that failed left things.
///
/// Boxed wherever it is returned: it is several vectors and a sentence,
/// and the success it shares a `Result` with is one vector.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Failure {
    /// The rename that failed, as `(from, to)` on disk — `to` may be a
    /// temporary name.
    pub step: (PathBuf, PathBuf),
    /// Why, as the system put it.
    pub reason: String,
    /// Whether the failure was found before anything moved — the check
    /// against the disk, rather than a rename that failed.
    pub before_starting: bool,
    /// Renames that are done and stay done, because putting them back
    /// failed too: `(what it was called, what it is called now)`.
    pub renamed: Vec<(PathBuf, PathBuf)>,
    /// Things left under a temporary name, as `(what it was called,
    /// where it is now)`. Empty unless putting back failed midway
    /// through a cycle.
    pub stranded: Vec<(PathBuf, PathBuf)>,
}

impl Failure {
    /// Whether everything is as it was before the batch.
    pub fn nothing_changed(&self) -> bool {
        self.renamed.is_empty() && self.stranded.is_empty()
    }
}

/// Renames every `(from, to)` in `moves`, or none of them.
///
/// Checked first against the disk: every source must still be there,
/// and every target free or held by something else in the batch that is
/// about to move. Then [`plan`]ned and carried out step by step; if a
/// step fails, the steps already done are undone newest first. On
/// success, returns the pairs that were renamed (unchanged pairs left
/// out).
pub fn apply(moves: &[(PathBuf, PathBuf)]) -> Result<Vec<(PathBuf, PathBuf)>, Box<Failure>> {
    apply_with(moves, rename_noreplace)
}

/// [`apply`] with the rename itself handed in, so a failure partway —
/// and a failure while putting back — can be produced on purpose in a
/// test.
fn apply_with(
    moves: &[(PathBuf, PathBuf)],
    mut rename: impl FnMut(&Path, &Path) -> io::Result<()>,
) -> Result<Vec<(PathBuf, PathBuf)>, Box<Failure>> {
    let moves: Vec<(PathBuf, PathBuf)> = moves.iter().filter(|(f, t)| f != t).cloned().collect();
    let refuse = |step: &(PathBuf, PathBuf), reason: String| {
        Box::new(Failure { step: step.clone(), reason, before_starting: true, renamed: Vec::new(), stranded: Vec::new() })
    };

    let exists = |p: &Path| std::fs::symlink_metadata(p).is_ok();
    let sources: HashSet<&Path> = moves.iter().map(|(f, _)| f.as_path()).collect();
    for pair in &moves {
        let (from, to) = pair;
        if !exists(from) {
            return Err(refuse(pair, format!("\u{201C}{}\u{201D} isn't there any more", name(from))));
        }
        if exists(to) && !sources.contains(to.as_path()) && !same_file(from, to) {
            return Err(refuse(pair, format!("something called \u{201C}{}\u{201D} is already there", name(to))));
        }
    }
    let steps = match plan(&moves, exists) {
        Ok(steps) => steps,
        Err(e) => {
            let pair = moves.first().cloned().unwrap_or_default();
            return Err(refuse(&pair, e.to_string()));
        }
    };

    let mut done: Vec<&Step> = Vec::with_capacity(steps.len());
    for step in &steps {
        if let Err(e) = rename(&step.from, &step.to) {
            return Err(Box::new(put_back(&moves, done, step, e, &mut rename)));
        }
        done.push(step);
    }
    Ok(moves)
}

/// Undoes `done`, newest first, after `failed` failed with `error` — and
/// says where everything ended up.
fn put_back(
    moves: &[(PathBuf, PathBuf)],
    done: Vec<&Step>,
    failed: &Step,
    error: io::Error,
    rename: &mut impl FnMut(&Path, &Path) -> io::Result<()>,
) -> Failure {
    // Where each original thing is now, followed through the steps that
    // stayed done. Starts as "where it was"; a step that could not be
    // put back moves it on.
    let mut stuck: Vec<&Step> = Vec::new();
    for step in done.into_iter().rev() {
        if rename(&step.to, &step.from).is_err() {
            stuck.push(step);
        }
    }
    // Stuck steps are in reverse order; follow them forwards.
    stuck.reverse();
    let mut renamed = Vec::new();
    let mut stranded = Vec::new();
    for (original, target) in moves {
        let mut now = original.clone();
        for step in &stuck {
            if step.from == now {
                now = step.to.clone();
            }
        }
        if now == *original {
            continue;
        }
        if now == *target {
            renamed.push((original.clone(), now));
        } else {
            stranded.push((original.clone(), now));
        }
    }
    Failure {
        step: (failed.from.clone(), failed.to.clone()),
        reason: describe(&error, &failed.to),
        before_starting: false,
        renamed,
        stranded,
    }
}

/// An error in the words the window shows, for a rename onto `to`.
fn describe(error: &io::Error, to: &Path) -> String {
    match error.kind() {
        io::ErrorKind::AlreadyExists => format!("something called \u{201C}{}\u{201D} appeared there first", name(to)),
        io::ErrorKind::PermissionDenied => "you don't have permission to rename things there".to_string(),
        io::ErrorKind::NotFound => "it isn't there any more".to_string(),
        _ => error.to_string(),
    }
}

/// Whether `a` and `b` are the same file — how a change of case alone
/// looks on a filesystem that ignores case, where the "taken" name is
/// the thing being renamed.
fn same_file(a: &Path, b: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    match (std::fs::symlink_metadata(a), std::fs::symlink_metadata(b)) {
        (Ok(a), Ok(b)) => a.dev() == b.dev() && a.ino() == b.ino(),
        _ => false,
    }
}

extern "C" {
    // glibc 2.28 and later; this suite targets current Arch. Declared
    // here rather than through a `libc` dependency for the reason
    // `localtime.rs` gives: every binary links the C library already.
    fn renameat2(
        olddirfd: std::os::raw::c_int,
        oldpath: *const std::os::raw::c_char,
        newdirfd: std::os::raw::c_int,
        newpath: *const std::os::raw::c_char,
        flags: std::os::raw::c_uint,
    ) -> std::os::raw::c_int;
}

/// Linux's `AT_FDCWD`: resolve a relative path against the working
/// directory, as `rename(2)` does. Every path here is absolute anyway.
const AT_FDCWD: std::os::raw::c_int = -100;
/// `RENAME_NOREPLACE`: fail with `EEXIST` rather than replace the
/// target.
const RENAME_NOREPLACE: std::os::raw::c_uint = 1;

/// Renames `from` to `to`, refusing — atomically, in the kernel — to
/// replace anything already at `to`.
///
/// Two fallbacks, each for a real filesystem:
///
/// - One that does not support the flag (`EINVAL`: some FUSE
///   filesystems, older network ones) gets a check and a plain rename.
///   That re-opens the window the flag closes, and is still the best
///   such a filesystem allows.
/// - A change of case alone on one that ignores case (a FAT USB stick)
///   finds its own target "taken" — by itself. Same inode, so a plain
///   rename is exactly what was asked.
pub fn rename_noreplace(from: &Path, to: &Path) -> io::Result<()> {
    let c = |p: &Path| CString::new(p.as_os_str().as_bytes()).map_err(|_| io::Error::from(io::ErrorKind::InvalidInput));
    let (old, new) = (c(from)?, c(to)?);
    // SAFETY: both pointers are to live, NUL-terminated `CString`s that
    // outlive the call; `renameat2` reads them and keeps neither.
    let status = unsafe { renameat2(AT_FDCWD, old.as_ptr(), AT_FDCWD, new.as_ptr(), RENAME_NOREPLACE) };
    if status == 0 {
        return Ok(());
    }
    let error = io::Error::last_os_error();
    match error.raw_os_error() {
        // EINVAL: the flag is not supported here.
        Some(22) => {
            if std::fs::symlink_metadata(to).is_ok() && !same_file(from, to) {
                return Err(io::Error::from(io::ErrorKind::AlreadyExists));
            }
            std::fs::rename(from, to)
        }
        // EEXIST
        Some(17) if same_file(from, to) => std::fs::rename(from, to),
        _ => Err(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(s: &str) -> PathBuf {
        PathBuf::from(format!("/d/{s}"))
    }

    fn pairs(list: &[(&str, &str)]) -> Vec<(PathBuf, PathBuf)> {
        list.iter().map(|(a, b)| (p(a), p(b))).collect()
    }

    /// Runs `steps` against a pretend folder holding `names`, failing if
    /// any step lands on something or starts from nothing, and returns
    /// what the folder holds after — by where each original ended up.
    fn simulate(names: &[&str], steps: &[Step]) -> HashMap<PathBuf, PathBuf> {
        // path -> which original is there
        let mut folder: HashMap<PathBuf, PathBuf> = names.iter().map(|n| (p(n), p(n))).collect();
        for step in steps {
            assert!(!folder.contains_key(&step.to), "{step:?} lands on something");
            let original = folder.remove(&step.from).unwrap_or_else(|| panic!("{step:?} starts from nothing"));
            folder.insert(step.to.clone(), original);
        }
        folder.into_iter().map(|(now, original)| (original, now)).collect()
    }

    #[test]
    fn independent_renames_run_in_the_order_given() {
        let steps = plan(&pairs(&[("a", "x"), ("b", "y")]), |_| false).unwrap();
        assert_eq!(steps, vec![Step { from: p("a"), to: p("x") }, Step { from: p("b"), to: p("y") }]);
    }

    /// `a -> b` and `b -> c`: `b` has to leave before `a` can arrive.
    #[test]
    fn a_chain_runs_from_its_free_end() {
        let steps = plan(&pairs(&[("a", "b"), ("b", "c")]), |_| false).unwrap();
        assert_eq!(steps.len(), 2, "a chain needs no temporary name");
        let after = simulate(&["a", "b"], &steps);
        assert_eq!(after[&p("a")], p("b"));
        assert_eq!(after[&p("b")], p("c"));
    }

    #[test]
    fn a_swap_goes_through_one_temporary_name() {
        let steps = plan(&pairs(&[("a", "b"), ("b", "a")]), |_| false).unwrap();
        assert_eq!(steps.len(), 3, "one step aside, then the two renames");
        let after = simulate(&["a", "b"], &steps);
        assert_eq!(after[&p("a")], p("b"));
        assert_eq!(after[&p("b")], p("a"));
    }

    #[test]
    fn a_long_cycle_and_a_chain_together_all_land() {
        // A rotation of five, plus a chain hanging off nothing in the
        // batch, plus a self-rename that is not a step at all.
        let moves = pairs(&[
            ("1", "2"),
            ("2", "3"),
            ("3", "4"),
            ("4", "5"),
            ("5", "1"),
            ("x", "y"),
            ("y", "z"),
            ("same", "same"),
        ]);
        let steps = plan(&moves, |_| false).unwrap();
        assert_eq!(steps.len(), 8, "seven renames and one step aside");
        let after = simulate(&["1", "2", "3", "4", "5", "x", "y", "same"], &steps);
        for (from, to) in &moves {
            assert_eq!(&after[from], to, "{from:?}");
        }
    }

    /// A temporary name never lands on something already in the folder.
    #[test]
    fn a_temporary_name_avoids_everything_already_there() {
        let taken = |path: &Path| path == p(".a.renaming");
        let steps = plan(&pairs(&[("a", "b"), ("b", "a")]), taken).unwrap();
        assert_eq!(steps[0].to, p(".a.renaming-1"));
    }

    #[test]
    fn two_renames_onto_one_name_cannot_be_planned() {
        let result = plan(&pairs(&[("a", "x"), ("b", "x")]), |_| false);
        assert_eq!(result, Err(PlanError::SameTarget(p("x"))));
    }

    #[test]
    fn a_rename_into_another_folder_is_not_a_rename() {
        let result = plan(&[(p("a"), PathBuf::from("/elsewhere/a"))], |_| false);
        assert!(matches!(result, Err(PlanError::LeavesFolder(_))));
    }

    // --- on a disk ------------------------------------------------------

    fn touch(dir: &Path, name: &str, contents: &str) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, contents).unwrap();
        path
    }

    fn read(dir: &Path, name: &str) -> String {
        std::fs::read_to_string(dir.join(name)).unwrap()
    }

    fn names(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> =
            std::fs::read_dir(dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        names.sort();
        names
    }

    #[test]
    fn a_swap_on_disk_swaps_the_contents() {
        let dir = tempfile::tempdir().unwrap();
        let a = touch(dir.path(), "a", "was a");
        let b = touch(dir.path(), "b", "was b");
        apply(&[(a.clone(), b.clone()), (b, a)]).unwrap();
        assert_eq!(read(dir.path(), "a"), "was b");
        assert_eq!(read(dir.path(), "b"), "was a");
        assert_eq!(names(dir.path()), ["a", "b"], "no temporary name is left behind");
    }

    #[test]
    fn a_rename_never_replaces_what_is_already_there() {
        let dir = tempfile::tempdir().unwrap();
        let a = touch(dir.path(), "a", "a");
        touch(dir.path(), "b", "keep me");
        let error = rename_noreplace(&a, &dir.path().join("b")).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(read(dir.path(), "b"), "keep me");
    }

    /// The check before anything moves is what lets the person hear
    /// about a clash without a single file having been touched.
    #[test]
    fn a_target_taken_outside_the_batch_stops_it_before_anything_moves() {
        let dir = tempfile::tempdir().unwrap();
        let a = touch(dir.path(), "a", "a");
        let b = touch(dir.path(), "b", "b");
        touch(dir.path(), "taken", "someone else's");
        let failure = apply(&[(a, dir.path().join("x")), (b, dir.path().join("taken"))]).unwrap_err();
        assert!(failure.before_starting);
        assert!(failure.nothing_changed());
        assert!(failure.reason.contains("taken"), "{}", failure.reason);
        assert_eq!(names(dir.path()), ["a", "b", "taken"]);
    }

    #[test]
    fn a_source_that_has_gone_stops_it_before_anything_moves() {
        let dir = tempfile::tempdir().unwrap();
        let a = touch(dir.path(), "a", "a");
        let failure = apply(&[(a, dir.path().join("x")), (dir.path().join("gone"), dir.path().join("y"))]).unwrap_err();
        assert!(failure.before_starting && failure.nothing_changed());
        assert_eq!(names(dir.path()), ["a"]);
    }

    /// The third rename fails: the two before it are put back, and the
    /// folder is exactly as it was.
    #[test]
    fn a_failure_partway_puts_back_everything_already_renamed() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        let moves: Vec<_> = ["a", "b", "c", "d"].iter().map(|n| (touch(d, n, n), d.join(format!("{n}2")))).collect();
        let mut calls = 0;
        let failure = apply_with(&moves, |from, to| {
            calls += 1;
            if calls == 3 {
                return Err(io::Error::from(io::ErrorKind::PermissionDenied));
            }
            rename_noreplace(from, to)
        })
        .unwrap_err();
        assert!(!failure.before_starting);
        assert!(failure.nothing_changed(), "{failure:?}");
        assert_eq!(failure.step.0, d.join("c"));
        assert!(failure.reason.contains("permission"), "{}", failure.reason);
        assert_eq!(names(d), ["a", "b", "c", "d"]);
    }

    /// The worst case, produced on purpose: a swap fails on its last
    /// step, and putting back the step aside fails too. The thing left
    /// under a temporary name is named in the report — never silently.
    #[test]
    fn what_could_not_be_put_back_is_named_with_where_it_is() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        let a = touch(d, "a", "was a");
        let b = touch(d, "b", "was b");
        let mut calls = 0;
        let failure = apply_with(&[(a.clone(), b.clone()), (b.clone(), a.clone())], |from, to| {
            calls += 1;
            match calls {
                // aside, then b -> a, then a's final rename fails...
                3 => Err(io::Error::from(io::ErrorKind::PermissionDenied)),
                // ...and putting the step aside back fails as well.
                5 => Err(io::Error::from(io::ErrorKind::PermissionDenied)),
                _ => rename_noreplace(from, to),
            }
        })
        .unwrap_err();
        assert!(failure.renamed.is_empty(), "b's rename was put back: {failure:?}");
        assert_eq!(failure.stranded, vec![(a, d.join(".a.renaming"))]);
        assert_eq!(read(d, ".a.renaming"), "was a", "and it really is there");
        assert_eq!(read(d, "b"), "was b");
    }

    /// A change of case alone, which a case-ignoring filesystem would
    /// otherwise call a clash with itself, is still allowed through the
    /// check — on this filesystem it is simply a rename.
    #[test]
    fn changing_only_the_case_renames() {
        let dir = tempfile::tempdir().unwrap();
        let a = touch(dir.path(), "notes", "n");
        apply(&[(a, dir.path().join("Notes"))]).unwrap();
        assert_eq!(names(dir.path()), ["Notes"]);
    }
}
