//! Copy, move and rename — files and whole directory trees, driven one
//! bounded step at a time.
//!
//! # Why `step()`, not `run(callback)` or a plan-then-execute split
//!
//! A GUI copying 40GB has to keep painting at 60fps and has to be able to
//! stop *now*, not "after this file". A `run` that blocks until the whole
//! tree is copied, calling back into the UI for progress, forces the
//! caller to run it on another thread and somehow marshal a cancel signal
//! back across that boundary anyway — and the callback still fires from
//! the wrong thread. A plan-then-execute split (compute everything, then
//! blast through it) has the same problem in a different place: nothing
//! stops the "execute" half from being just as blocking as `run` was.
//!
//! `step()` sidesteps both: each call does one bounded unit of work — one
//! directory listed, one small file copied whole, one chunk of a large
//! one — and returns. A caller on a 60fps paint loop calls `step()` once
//! per frame (or however many fit in its budget) and redraws between
//! calls; a caller on a background thread just loops. Either way,
//! cancellation is [`Operation::request_cancel`] setting an
//! [`std::sync::atomic::AtomicBool`] that every `step()` checks *before*
//! it does that step's unit of work — so a cancel lands between chunks of
//! the file currently copying, not only between files, and never mid
//! partially-written chunk.
//!
//! # What gets walked, and when
//!
//! Placing a source into a destination needs to know two things before
//! any bytes move: how big the job is (for an honest progress bar) and
//! whether the destination already has something in the way at every
//! level (so a collision is never discovered mid-copy, after it's too
//! late to ask cleanly). So this crate walks the whole source tree before
//! copying a single byte — but not as one blocking call: `Phase::Walking`
//! lists exactly one directory per `step()`, the same bounded-unit-of-work
//! discipline as everything else here. The honest cost: a tree with a
//! million files takes a million-ish `stat` calls, spread over that many
//! `step()` calls, before the first byte of the first file moves — real
//! wall-clock time, paid up front, in exchange for a progress bar that
//! means something instead of guessing.
//!
//! One case skips the walk entirely: a same-filesystem move. [`OpKind::Move`]
//! asks [`Filesystem::device_of`] whether source and destination share a
//! device *before* doing anything else, and if they do, hands the whole
//! job to a single `rename(2)` — no walk, no per-file copy, the operation
//! completes in one `step()` call. Reusing [`Filesystem`] rather than
//! calling `std::fs::rename` and inspecting its error is deliberate: a
//! test can declare "these are different devices" through
//! `crate::fs::mock::MockFilesystem` (behind the `mock` feature) without a second real mount, which
//! is the entire reason that trait exists (see the module doc on
//! [`crate::fs`]).
//!
//! # Durability
//!
//! [`hyprforge_paths::write_atomic`] is the wrong tool here on purpose —
//! it replaces one small config file with a complete new version, and a
//! tree copy has no "old version" to atomically swap over; it is building
//! something new at a path nothing occupied. Instead, every copied file's
//! handle gets `sync_all()`'d — flushed to the drive, not just to the
//! page cache — before that file is recorded as succeeded. A crash right
//! after this crate reports "done" must not lose a file that a shell
//! `sync` and an unplug would have kept.
//!
//! # What is not preserved
//!
//! Regular permission bits (the low 12 bits of `st_mode`) and mtime are
//! preserved, best-effort — a failure to preserve one is logged and does
//! not fail the copy. Not preserved at all: extended attributes, ACLs,
//! hard-link identity (two hard-linked sources become two independent
//! files at the destination), and a symlink's own mtime (Linux has no
//! stable safe API this crate uses for `lutimes`; the *target* of a copied
//! symlink is never touched regardless).

use crate::fs::Filesystem;
use crate::trash::unique_name;
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// One `step()`'s worth of file I/O — small enough that a caller driving
/// this from a paint loop never blocks on it long enough to drop a frame,
/// large enough that a 40GB file does not take millions of calls to move.
const CHUNK_SIZE: usize = 1024 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum OpsError {
    #[error("{path} does not exist")]
    SourceNotFound { path: PathBuf },

    #[error("could not tell which filesystem {path} is on: {source}")]
    Filesystem {
        path: PathBuf,
        #[source]
        source: io::Error,
    },

    /// The source file or directory entry was there when this crate
    /// walked the tree, or even when it opened it for reading, and is
    /// gone now — someone else deleted or moved it out from under this
    /// operation while it was running.
    #[error("{path} disappeared while it was being copied")]
    SourceVanished { path: PathBuf },

    /// Distinguished from a generic I/O failure because it is the one a
    /// user can act on immediately: free some space and retry, rather
    /// than "check the logs".
    #[error("could not write {path}: the disk is full")]
    DiskFull { path: PathBuf },

    /// Same reasoning as `DiskFull` — this is not "something went wrong",
    /// it is "you don't have permission to write there", a sentence a UI
    /// can show as-is. The *destination* side only: see `ReadDenied`.
    #[error("you don't have permission to write to {path}")]
    PermissionDenied { path: PathBuf },

    /// The source side's half of `PermissionDenied`: a file that could not
    /// be opened, or a folder that could not be listed, for want of
    /// permission to read it.
    ///
    /// Its own variant because the two used to share one, and so a file
    /// that could not be *read* was reported as "you don't have
    /// permission to write to" the file being copied — a sentence about
    /// the wrong file's wrong permission, sending someone to fix a folder
    /// that was never the problem.
    #[error("you don't have permission to read {path}")]
    ReadDenied { path: PathBuf },

    /// A same-filesystem move that `rename(2)` refused for permission.
    /// Moving needs write access to both the folder an item leaves and the
    /// one it goes to, and the kernel does not say which was missing — so
    /// this names both rather than guessing and blaming the destination.
    #[error("you don't have permission to move {from} to {to}: moving needs write access to the folder it leaves and the one it goes to")]
    MoveDenied { from: PathBuf, to: PathBuf },

    #[error("could not read {path}: {source}")]
    Read {
        path: PathBuf,
        #[source]
        source: io::Error,
    },

    #[error("could not write {path}: {source}")]
    Write {
        path: PathBuf,
        #[source]
        source: io::Error,
    },

    #[error("could not remove {path}: {source}")]
    Remove {
        path: PathBuf,
        #[source]
        source: io::Error,
    },

    #[error("could not list the directory {path}: {source}")]
    ListDir {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
}

impl OpsError {
    /// Whether this failed for want of permission — and so is the one
    /// kind of failure that trying again as someone with more of it
    /// could fix.
    ///
    /// The three variants that say so in words, and any other whose
    /// underlying error is `PermissionDenied`: removing a source after a
    /// move, listing a folder and asking which filesystem a path is on
    /// can each be refused for permission without being classified into
    /// a sentence of their own.
    pub fn is_permission(&self) -> bool {
        match self {
            OpsError::PermissionDenied { .. } | OpsError::ReadDenied { .. } | OpsError::MoveDenied { .. } => true,
            OpsError::Filesystem { source, .. }
            | OpsError::Read { source, .. }
            | OpsError::Write { source, .. }
            | OpsError::Remove { source, .. }
            | OpsError::ListDir { source, .. } => source.kind() == io::ErrorKind::PermissionDenied,
            OpsError::SourceNotFound { .. } | OpsError::SourceVanished { .. } | OpsError::DiskFull { .. } => false,
        }
    }
}

/// Classifies an I/O failure encountered reading from the *source* side.
/// `NotFound` here specifically means "it was there a moment ago" — the
/// walk already established it existed — never "nothing was ever there".
fn classify_read(path: &Path, source: io::Error) -> OpsError {
    match source.kind() {
        io::ErrorKind::NotFound => OpsError::SourceVanished { path: path.to_path_buf() },
        io::ErrorKind::PermissionDenied => OpsError::ReadDenied { path: path.to_path_buf() },
        _ => OpsError::Read { path: path.to_path_buf(), source },
    }
}

/// Classifies an I/O failure encountered writing to the *destination*
/// side, where the same OS error kinds mean different sentences than they
/// do on the source side (see [`classify_read`]).
fn classify_write(path: &Path, source: io::Error) -> OpsError {
    match source.kind() {
        io::ErrorKind::StorageFull => OpsError::DiskFull { path: path.to_path_buf() },
        io::ErrorKind::PermissionDenied => OpsError::PermissionDenied { path: path.to_path_buf() },
        _ => OpsError::Write { path: path.to_path_buf(), source },
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpKind {
    Copy,
    Move,
}

/// How a collision at a destination path was actually resolved. Whether a
/// move used `rename(2)` or fell back to copy-then-delete is exactly the
/// distinction the test suite pins — see the module doc's "What gets
/// walked, and when" section — so it is part of the public report, not an
/// implementation detail a caller has to infer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MoveStrategy {
    Renamed,
    CopyThenDelete,
}

/// The caller's answer to a [`Collision`]. Never resolved automatically —
/// see the crate-level rule that collisions are never silently resolved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CollisionPolicy {
    Skip,
    Replace,
    /// Auto-renamed via the same `stem.N.ext` scheme `trash.rs` uses for
    /// exactly this purpose — see `crate::trash::unique_name`.
    KeepBoth,
    Cancel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CollisionDecision {
    pub policy: CollisionPolicy,
    /// When true, every later collision in this same operation is
    /// resolved with `policy` without pausing to ask again.
    pub apply_to_rest: bool,
}

/// A destination path that already has something at it. Returned by
/// [`Operation::step`] instead of proceeding; the caller answers with
/// [`Operation::resolve`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Collision {
    /// The source entry that would land at `dest`, absolute.
    pub source: PathBuf,
    /// The destination path something already occupies, absolute.
    pub dest: PathBuf,
}

/// Progress as of the most recent `step()`. `bytes_total`/`entries_total`
/// are `None` while the source tree is still being walked (see the module
/// doc) — legitimately unknown until the walk finishes, not a guess dressed
/// up as a number.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Progress {
    pub bytes_done: u64,
    pub bytes_total: Option<u64>,
    pub entries_done: u64,
    pub entries_total: Option<u64>,
    /// The source path the step just made progress on.
    pub current: PathBuf,
}

/// What `step()` did.
#[derive(Debug, Clone)]
pub enum StepOutcome {
    Progress(Progress),
    /// Stop and ask the caller; call [`Operation::resolve`] to continue.
    Collision(Collision),
    Done(Report),
}

/// The final account of an operation: not a boolean, because a partial
/// failure deep in a large tree still needs every entry that *did* succeed
/// named, so the caller can retry only what failed. See the crate-level
/// rule against rolling back a partial failure.
#[derive(Debug, Clone, Default)]
pub struct Report {
    pub succeeded: Vec<PathBuf>,
    pub skipped: Vec<PathBuf>,
    /// Rendered messages, one per failed source entry — already the
    /// actionable sentence an `OpsError`'s `Display` produces, kept as
    /// text here because a `Report` needs to be cheap to hold onto after
    /// the `Operation` that produced it is gone.
    pub failed: Vec<(PathBuf, String)>,
    /// Which of [`Self::failed`]'s paths failed for want of permission
    /// ([`OpsError::is_permission`]) — carried as data beside the
    /// sentences, so a caller deciding whether to offer "try again as
    /// administrator" never has to recognise one by its wording, which a
    /// message is free to change.
    pub denied: Vec<PathBuf>,
    pub cancelled: bool,
    /// `None` for a [`OpKind::Copy`]; `Some` for a [`OpKind::Move`] that
    /// got at least as far as deciding how to move the root.
    pub move_strategy: Option<MoveStrategy>,
    /// Set only for a move that copied successfully but then could not
    /// remove the now-redundant source — content is safe (it exists at
    /// both ends), but the caller needs to know the source is not gone.
    pub source_removal_failed: Option<String>,
    /// Where the root actually landed. The destination asked for, unless
    /// a Keep Both answer to a conflict on the root renamed it — which is
    /// the case a caller cannot work out for itself, and the one an undo
    /// needs: taking back a copy means finding the copy.
    pub dest: PathBuf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EntryKind {
    Dir,
    File { size: u64 },
    Symlink,
}

#[derive(Debug, Clone)]
struct Entry {
    /// Stable identity: where this entry actually lives under the source
    /// root. Never rewritten — a `KeepBoth` decision changes where the
    /// entry *lands*, never where it came from.
    source_relative: PathBuf,
    /// Where this entry lands under the destination root. Starts equal to
    /// `source_relative`; a `KeepBoth` decision on an ancestor directory
    /// rewrites this (and every descendant's) prefix.
    dest_relative: PathBuf,
    kind: EntryKind,
}

#[derive(Debug, Clone, Copy)]
enum CollisionCtx {
    Root,
    Entry(usize),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    RootCheck,
    Walking,
    Copying,
    Finalizing,
    Done,
}

/// An in-progress chunked copy of one file — the state that survives
/// between `step()` calls while a large file is only partway copied.
struct FileCopy {
    reader: File,
    writer: File,
    source_path: PathBuf,
    dest_path: PathBuf,
    src_meta: fs::Metadata,
    /// The copy's scratch buffer, allocated once when the file is opened
    /// and reused for every chunk.
    ///
    /// It used to be a `vec![0u8; CHUNK_SIZE]` inside the per-chunk
    /// step, which allocates *and zeroes* a megabyte, fills it, and
    /// frees it — once per megabyte copied. The module doc's own 40 GB
    /// example is forty thousand of those, and 40 GB of pointless page
    /// touching, for a buffer whose contents are overwritten before
    /// anything reads them.
    buf: Vec<u8>,
}

/// A copy, move or rename in progress. See the module doc for the design
/// this drives: call [`Operation::step`] repeatedly; it returns
/// [`StepOutcome::Collision`] when it needs a decision
/// ([`Operation::resolve`]) and [`StepOutcome::Done`] with a [`Report`]
/// when there is nothing left to do.
pub struct Operation<F: Filesystem> {
    fs: F,
    kind: OpKind,
    source: PathBuf,
    /// The resolved destination root — may differ from what the caller
    /// asked for if the root itself collided and was resolved `KeepBoth`.
    dest: PathBuf,
    cancel: Arc<AtomicBool>,
    phase: Phase,

    entries: Vec<Entry>,
    walk_stack: Vec<PathBuf>,
    bytes_total: u64,

    index: usize,
    bytes_done: u64,
    current_copy: Option<FileCopy>,

    pending_collision: Option<CollisionCtx>,
    collision_policy_for_rest: Option<CollisionPolicy>,
    /// `source_relative` prefixes whose whole subtree is being skipped —
    /// set when a directory entry itself is skipped, so its children are
    /// never even looked at, let alone prompted about individually.
    skip_prefixes: Vec<PathBuf>,

    succeeded: Vec<PathBuf>,
    skipped: Vec<PathBuf>,
    failed: Vec<(PathBuf, String)>,
    denied: Vec<PathBuf>,
    cancelled: bool,
    move_strategy: Option<MoveStrategy>,
    source_removal_failed: Option<String>,
}

impl<F: Filesystem> Operation<F> {
    pub fn new(fs: F, kind: OpKind, source: impl Into<PathBuf>, dest: impl Into<PathBuf>) -> Self {
        Operation {
            fs,
            kind,
            source: source.into(),
            dest: dest.into(),
            cancel: Arc::new(AtomicBool::new(false)),
            phase: Phase::RootCheck,
            entries: Vec::new(),
            walk_stack: Vec::new(),
            bytes_total: 0,
            index: 0,
            bytes_done: 0,
            current_copy: None,
            pending_collision: None,
            collision_policy_for_rest: None,
            skip_prefixes: Vec::new(),
            succeeded: Vec::new(),
            skipped: Vec::new(),
            failed: Vec::new(),
            denied: Vec::new(),
            cancelled: false,
            move_strategy: None,
            source_removal_failed: None,
        }
    }

    /// A handle another thread can use to cancel this operation while
    /// this thread is the one calling `step()` — the shape a GUI needs: a
    /// worker thread drives `step()`, the UI thread reacts to a Cancel
    /// button.
    pub fn cancel_handle(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.cancel)
    }

    /// Cancels from the same thread that is driving `step()` — sugar over
    /// [`Self::cancel_handle`] for the common case of a single-threaded
    /// caller (a test, or a caller that polls `step()` from an event loop
    /// on one thread).
    pub fn request_cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    /// Answers a [`StepOutcome::Collision`] returned by the previous
    /// `step()` call. Panics if no collision is pending — a programming
    /// error in the caller, not a condition this crate can recover from.
    pub fn resolve(&mut self, decision: CollisionDecision) {
        let ctx = self.pending_collision.take().expect("resolve() called with no pending collision");
        if decision.apply_to_rest {
            self.collision_policy_for_rest = Some(decision.policy);
        }
        match ctx {
            CollisionCtx::Root => self.apply_root_collision(decision.policy),
            CollisionCtx::Entry(idx) => self.apply_entry_collision(idx, decision.policy),
        }
    }

    /// Advances the operation by one bounded unit of work. See the module
    /// doc for why the unit is bounded this way.
    pub fn step(&mut self) -> StepOutcome {
        if self.phase != Phase::Done && self.cancel.load(Ordering::Relaxed) {
            return self.cancel_now();
        }
        match self.phase {
            Phase::Done => StepOutcome::Done(self.build_report()),
            Phase::RootCheck => self.step_root_check(),
            Phase::Walking => self.step_walk(),
            Phase::Copying => self.step_copy(),
            Phase::Finalizing => self.step_finalize(),
        }
    }

    fn cancel_now(&mut self) -> StepOutcome {
        self.cancelled = true;
        // Abandon a file mid-chunk-copy: remove the partial destination
        // file rather than leave something that looks like a real file
        // sitting at that path.
        if let Some(copy) = self.current_copy.take() {
            drop(copy.writer);
            let _ = fs::remove_file(&copy.dest_path);
        }
        // The source is never touched by a copy-then-delete move until
        // Finalizing, which we are skipping straight past here — so
        // cancelling, at any point before that, leaves the source
        // completely untouched, by construction rather than by an extra
        // check.
        self.phase = Phase::Done;
        StepOutcome::Done(self.build_report())
    }

    fn build_report(&mut self) -> Report {
        Report {
            succeeded: std::mem::take(&mut self.succeeded),
            skipped: std::mem::take(&mut self.skipped),
            failed: std::mem::take(&mut self.failed),
            denied: std::mem::take(&mut self.denied),
            cancelled: self.cancelled,
            move_strategy: self.move_strategy,
            source_removal_failed: self.source_removal_failed.take(),
            dest: self.dest.clone(),
        }
    }

    fn fail(&mut self, path: PathBuf, err: OpsError) {
        if err.is_permission() {
            self.denied.push(path.clone());
        }
        self.failed.push((path, err.to_string()));
    }

    // ---- Phase::RootCheck ------------------------------------------------

    fn step_root_check(&mut self) -> StepOutcome {
        // `resolve()` re-enters here after a root collision decision —
        // guard entry creation so a Replace/KeepBoth resolution doesn't
        // stat the root and push a second `entries[0]`.
        if self.entries.is_empty() {
            let root_status = match fs::symlink_metadata(&self.source) {
                Ok(meta) => meta,
                Err(e) if e.kind() == io::ErrorKind::NotFound => {
                    self.fail(self.source.clone(), OpsError::SourceNotFound { path: self.source.clone() });
                    self.phase = Phase::Done;
                    return self.step();
                }
                Err(source) => {
                    self.fail(
                        self.source.clone(),
                        OpsError::Read { path: self.source.clone(), source },
                    );
                    self.phase = Phase::Done;
                    return self.step();
                }
            };
            let kind = entry_kind_of(&root_status);
            // The walk counts every file *under* a folder; a root that is
            // itself a file is never walked, so its size is counted here.
            // Without this a one-file copy reported `bytes_total` 0 and a
            // window showed "5.9 GiB of 0 B" for a 24 GiB copy.
            if let EntryKind::File { size } = kind {
                self.bytes_total += size;
            }
            self.entries.push(Entry { source_relative: PathBuf::new(), dest_relative: PathBuf::new(), kind });
        }

        // A collision on the root is checked before anything else — in
        // particular before a `rename(2)` is even considered, because
        // `rename` on POSIX silently replaces an existing destination,
        // which would turn "ask the user" into "guess for them".
        if fs::symlink_metadata(&self.dest).is_ok() {
            if let Some(outcome) = self.check_collision_or_prompt(CollisionCtx::Root, self.source.clone(), self.dest.clone()) {
                return outcome;
            }
        }
        self.continue_root_check()
    }

    /// Runs after the root's collision (if any) has been resolved:
    /// decides rename vs. walk-and-copy for a move, or heads straight
    /// into walking for a copy.
    fn continue_root_check(&mut self) -> StepOutcome {
        if self.phase == Phase::Done {
            return self.step();
        }
        let root_kind = self.entries[0].kind;

        if self.kind == OpKind::Move {
            let same_device = match (self.fs.device_of(&self.source), self.fs.device_of(&self.dest)) {
                (Ok(a), Ok(b)) => a == b,
                (Err(source), _) | (_, Err(source)) => {
                    self.fail(self.source.clone(), OpsError::Filesystem { path: self.source.clone(), source });
                    self.phase = Phase::Done;
                    return self.step();
                }
            };
            if same_device {
                match fs::rename(&self.source, &self.dest) {
                    Ok(()) => {
                        self.move_strategy = Some(MoveStrategy::Renamed);
                        self.succeeded.push(self.dest.clone());
                        self.phase = Phase::Done;
                        return self.step();
                    }
                    Err(e) if e.kind() == io::ErrorKind::CrossesDevices => {
                        // `device_of` said "same", the kernel disagrees
                        // (an overlay or bind mount can do this) — trust
                        // the syscall and fall back, rather than fail an
                        // otherwise-legitimate move.
                    }
                    Err(source) if source.kind() == io::ErrorKind::PermissionDenied => {
                        let denied = OpsError::MoveDenied { from: self.source.clone(), to: self.dest.clone() };
                        self.fail(self.source.clone(), denied);
                        self.phase = Phase::Done;
                        return self.step();
                    }
                    Err(source) => {
                        self.fail(self.source.clone(), classify_write(&self.dest, source));
                        self.phase = Phase::Done;
                        return self.step();
                    }
                }
            }
            self.move_strategy = Some(MoveStrategy::CopyThenDelete);
        }

        if root_kind == EntryKind::Dir {
            self.walk_stack.push(PathBuf::new());
            self.phase = Phase::Walking;
        } else {
            self.phase = Phase::Copying;
        }
        self.step()
    }

    fn apply_root_collision(&mut self, policy: CollisionPolicy) {
        match policy {
            CollisionPolicy::Skip => {
                self.skipped.push(self.source.clone());
                self.phase = Phase::Done;
            }
            CollisionPolicy::Cancel => {
                self.cancelled = true;
                self.phase = Phase::Done;
            }
            CollisionPolicy::Replace => {
                if let Err(source) = remove_any(&self.dest) {
                    self.fail(self.source.clone(), OpsError::Remove { path: self.dest.clone(), source });
                    self.phase = Phase::Done;
                }
            }
            CollisionPolicy::KeepBoth => match renamed_sibling(&self.dest) {
                Ok(new_dest) => self.dest = new_dest,
                Err(source) => {
                    self.fail(self.source.clone(), OpsError::Filesystem { path: self.dest.clone(), source });
                    self.phase = Phase::Done;
                }
            },
        }
    }

    // ---- Phase::Walking ----------------------------------------------

    fn step_walk(&mut self) -> StepOutcome {
        let Some(rel_dir) = self.walk_stack.pop() else {
            self.index = 0;
            self.phase = Phase::Copying;
            return self.step();
        };
        // The root itself, not `source.join("")`: that spells the folder
        // with a trailing slash, and the path ends up in a sentence.
        let abs_dir = if rel_dir.as_os_str().is_empty() { self.source.clone() } else { self.source.join(&rel_dir) };
        let read = match fs::read_dir(&abs_dir) {
            Ok(read) => read,
            Err(source) => {
                // A folder that cannot be listed for want of permission is
                // the same sentence as a file that cannot be opened for it.
                let err = if source.kind() == io::ErrorKind::PermissionDenied {
                    OpsError::ReadDenied { path: abs_dir.clone() }
                } else {
                    OpsError::ListDir { path: abs_dir.clone(), source }
                };
                self.fail(abs_dir.clone(), err);
                return StepOutcome::Progress(self.progress_snapshot(abs_dir));
            }
        };

        let mut children: Vec<_> = read.collect::<Result<Vec<_>, _>>().unwrap_or_default();
        children.sort_by_key(|e| e.file_name());
        for child in children {
            let child_rel = rel_dir.join(child.file_name());
            let child_abs = self.source.join(&child_rel);
            let status = match fs::symlink_metadata(&child_abs) {
                Ok(s) => s,
                Err(source) => {
                    self.fail(child_abs.clone(), classify_read(&child_abs, source));
                    continue;
                }
            };
            let kind = entry_kind_of(&status);
            if let EntryKind::File { size } = kind {
                self.bytes_total += size;
            }
            if kind == EntryKind::Dir {
                self.walk_stack.push(child_rel.clone());
            }
            self.entries.push(Entry {
                source_relative: child_rel.clone(),
                dest_relative: child_rel,
                kind,
            });
        }
        StepOutcome::Progress(self.progress_snapshot(abs_dir))
    }

    fn progress_snapshot(&self, current: PathBuf) -> Progress {
        let walking = self.phase == Phase::Walking;
        Progress {
            bytes_done: self.bytes_done,
            bytes_total: if walking { None } else { Some(self.bytes_total) },
            entries_done: self.index as u64,
            entries_total: if walking { None } else { Some(self.entries.len() as u64) },
            current,
        }
    }

    // ---- Phase::Copying -------------------------------------------------

    fn step_copy(&mut self) -> StepOutcome {
        if let Some(copy) = self.current_copy.take() {
            return self.continue_file_copy(copy);
        }
        if self.index >= self.entries.len() {
            self.phase = Phase::Finalizing;
            return self.step();
        }

        let idx = self.index;
        let entry = self.entries[idx].clone();
        let src_path = join_relative(&self.source, &entry.source_relative);

        if self.skip_prefixes.iter().any(|p| entry.source_relative.starts_with(p)) {
            self.skipped.push(src_path);
            self.index += 1;
            return StepOutcome::Progress(self.progress_snapshot(join_relative(&self.source, &entry.source_relative)));
        }

        let dest_path = join_relative(&self.dest, &entry.dest_relative);

        // The root's own collision was already handled in RootCheck.
        if idx != 0 {
            if let Ok(existing) = fs::symlink_metadata(&dest_path) {
                let both_dirs = entry.kind == EntryKind::Dir && existing.is_dir() && !existing.file_type().is_symlink();
                if both_dirs {
                    // Merging into an existing directory is not a
                    // collision worth pausing over — every file manager
                    // does this silently, and the files *inside* still
                    // get their own collision prompts individually.
                    self.succeeded.push(dest_path);
                    self.index += 1;
                    return StepOutcome::Progress(self.progress_snapshot(src_path));
                }
                if let Some(outcome) = self.check_collision_or_prompt(CollisionCtx::Entry(idx), src_path.clone(), dest_path.clone()) {
                    return outcome;
                }
                if self.phase == Phase::Done {
                    return self.step();
                }
                // A collision decision may have changed idx's dest_relative
                // (KeepBoth) or removed what was at dest_path (Replace) —
                // re-read before creating.
            }
        }

        self.create_entry(idx)
    }

    fn create_entry(&mut self, idx: usize) -> StepOutcome {
        let entry = self.entries[idx].clone();
        let src_path = join_relative(&self.source, &entry.source_relative);
        let dest_path = join_relative(&self.dest, &entry.dest_relative);

        match entry.kind {
            EntryKind::Dir => {
                match fs::create_dir(&dest_path) {
                    Ok(()) => {
                        if let Ok(meta) = fs::symlink_metadata(&src_path) {
                            let _ = fs::set_permissions(&dest_path, fs::Permissions::from_mode(meta.mode() & 0o7777));
                        }
                        self.succeeded.push(dest_path);
                    }
                    Err(source) => self.fail(src_path.clone(), classify_write(&dest_path, source)),
                }
                self.index += 1;
                StepOutcome::Progress(self.progress_snapshot(src_path))
            }
            EntryKind::Symlink => {
                match fs::read_link(&src_path) {
                    Ok(target) => match std::os::unix::fs::symlink(&target, &dest_path) {
                        Ok(()) => self.succeeded.push(dest_path),
                        Err(source) => self.fail(src_path.clone(), classify_write(&dest_path, source)),
                    },
                    Err(source) => self.fail(src_path.clone(), classify_read(&src_path, source)),
                }
                self.index += 1;
                StepOutcome::Progress(self.progress_snapshot(src_path))
            }
            EntryKind::File { .. } => {
                let src_meta = match fs::symlink_metadata(&src_path) {
                    Ok(m) => m,
                    Err(source) => {
                        self.fail(src_path.clone(), classify_read(&src_path, source));
                        self.index += 1;
                        return StepOutcome::Progress(self.progress_snapshot(src_path));
                    }
                };
                let reader = match File::open(&src_path) {
                    Ok(f) => f,
                    Err(source) => {
                        self.fail(src_path.clone(), classify_read(&src_path, source));
                        self.index += 1;
                        return StepOutcome::Progress(self.progress_snapshot(src_path));
                    }
                };
                let writer = match File::create(&dest_path) {
                    Ok(f) => f,
                    Err(source) => {
                        self.fail(src_path.clone(), classify_write(&dest_path, source));
                        self.index += 1;
                        return StepOutcome::Progress(self.progress_snapshot(src_path));
                    }
                };
                self.current_copy = Some(FileCopy {
                    reader,
                    writer,
                    source_path: src_path.clone(),
                    dest_path,
                    src_meta,
                    buf: vec![0u8; CHUNK_SIZE],
                });
                StepOutcome::Progress(self.progress_snapshot(src_path))
            }
        }
    }

    /// One `CHUNK_SIZE` unit of an in-progress file copy — the bounded
    /// step that keeps a single huge file from blocking a caller for the
    /// whole transfer, and that gives cancellation somewhere to land
    /// short of "wait for this file to finish".
    fn continue_file_copy(&mut self, mut copy: FileCopy) -> StepOutcome {
        let n = match copy.reader.read(&mut copy.buf) {
            Ok(n) => n,
            Err(source) => {
                self.fail(copy.source_path.clone(), classify_read(&copy.source_path, source));
                drop(copy.writer);
                let _ = fs::remove_file(&copy.dest_path);
                self.index += 1;
                return StepOutcome::Progress(self.progress_snapshot(copy.source_path));
            }
        };

        if n == 0 {
            // EOF: flush all the way to the drive before this file counts
            // as done — see the module doc's "Durability" section.
            if let Err(source) = copy.writer.sync_all() {
                self.fail(copy.source_path.clone(), classify_write(&copy.dest_path, source));
                let _ = fs::remove_file(&copy.dest_path);
                self.index += 1;
                return StepOutcome::Progress(self.progress_snapshot(copy.source_path));
            }
            preserve_attrs(&copy.writer, &copy.source_path, &copy.dest_path, &copy.src_meta, "file");
            self.succeeded.push(copy.dest_path.clone());
            self.index += 1;
            return StepOutcome::Progress(self.progress_snapshot(copy.source_path));
        }

        if let Err(source) = copy.writer.write_all(&copy.buf[..n]) {
            self.fail(copy.source_path.clone(), classify_write(&copy.dest_path, source));
            drop(copy.writer);
            let _ = fs::remove_file(&copy.dest_path);
            self.index += 1;
            return StepOutcome::Progress(self.progress_snapshot(copy.source_path));
        }
        self.bytes_done += n as u64;
        let snapshot = self.progress_snapshot(copy.source_path.clone());
        self.current_copy = Some(copy);
        StepOutcome::Progress(snapshot)
    }

    /// Checks whether `dest` is occupied and, if so, either resolves it
    /// immediately (an `apply_to_rest` policy already on file) or reports
    /// a [`StepOutcome::Collision`] for the caller to answer. Returns
    /// `Some(outcome)` only when the caller must stop and look at that
    /// outcome right now.
    fn check_collision_or_prompt(
        &mut self,
        ctx: CollisionCtx,
        source: PathBuf,
        dest: PathBuf,
    ) -> Option<StepOutcome> {
        if let Some(policy) = self.collision_policy_for_rest {
            match ctx {
                CollisionCtx::Root => self.apply_root_collision(policy),
                CollisionCtx::Entry(idx) => self.apply_entry_collision(idx, policy),
            }
            return None;
        }
        self.pending_collision = Some(ctx);
        Some(StepOutcome::Collision(Collision { source, dest }))
    }

    fn apply_entry_collision(&mut self, idx: usize, policy: CollisionPolicy) {
        let entry = self.entries[idx].clone();
        let src_path = join_relative(&self.source, &entry.source_relative);
        let dest_path = join_relative(&self.dest, &entry.dest_relative);

        match policy {
            CollisionPolicy::Skip => {
                self.skipped.push(src_path);
                if entry.kind == EntryKind::Dir {
                    self.skip_prefixes.push(entry.source_relative.clone());
                }
                self.index += 1;
            }
            CollisionPolicy::Cancel => {
                self.cancelled = true;
                self.phase = Phase::Done;
            }
            CollisionPolicy::Replace => {
                if let Err(source) = remove_any(&dest_path) {
                    self.fail(src_path, OpsError::Remove { path: dest_path, source });
                    self.index += 1;
                }
                // Falls through to create_entry on the next step() call
                // with dest now clear; index is unchanged so step_copy
                // reprocesses this same entry.
            }
            CollisionPolicy::KeepBoth => match renamed_sibling(&dest_path) {
                Ok(new_dest) => {
                    let new_relative = new_dest.strip_prefix(&self.dest).unwrap_or(&new_dest).to_path_buf();
                    self.rename_entry_and_descendants(idx, new_relative);
                }
                Err(source) => {
                    self.fail(src_path, OpsError::Filesystem { path: dest_path, source });
                    self.index += 1;
                }
            },
        }
    }

    /// Rewrites `entries[idx]`'s `dest_relative` to `new_relative`, and
    /// every descendant's `dest_relative` prefix along with it — needed
    /// because the walk already discovered the whole tree (including
    /// everything under a directory that only now, at copy time, turns
    /// out to collide) using the *original* names.
    fn rename_entry_and_descendants(&mut self, idx: usize, new_relative: PathBuf) {
        let old_prefix = self.entries[idx].source_relative.clone();
        for entry in self.entries[idx..].iter_mut() {
            if let Ok(suffix) = entry.source_relative.strip_prefix(&old_prefix) {
                entry.dest_relative = if suffix.as_os_str().is_empty() {
                    new_relative.clone()
                } else {
                    new_relative.join(suffix)
                };
            }
        }
    }

    // ---- Phase::Finalizing ------------------------------------------

    fn step_finalize(&mut self) -> StepOutcome {
        // Bottom-up so a child's mtime fixup happens before its parent's
        // — creating a child updates the parent's mtime, so the parent
        // must be touched last to have its own preserved value stick.
        let mut dirs: Vec<usize> = self
            .entries
            .iter()
            .enumerate()
            .filter(|(_, e)| e.kind == EntryKind::Dir)
            .map(|(i, _)| i)
            .collect();
        dirs.sort_by_key(|&i| std::cmp::Reverse(self.entries[i].dest_relative.components().count()));
        for i in dirs {
            let src = self.source.join(&self.entries[i].source_relative);
            let dst = self.dest.join(&self.entries[i].dest_relative);
            if let (Ok(meta), Ok(dir)) = (fs::symlink_metadata(&src), File::open(&dst)) {
                preserve_attrs(&dir, &src, &dst, &meta, "directory");
            }
        }

        if self.kind == OpKind::Move
            && self.move_strategy == Some(MoveStrategy::CopyThenDelete)
            && self.failed.is_empty()
            && !self.cancelled
        {
            let root_kind = self.entries[0].kind;
            let result = match root_kind {
                EntryKind::Dir => fs::remove_dir_all(&self.source),
                _ => fs::remove_file(&self.source),
            };
            if let Err(source) = result {
                self.source_removal_failed =
                    Some(OpsError::Remove { path: self.source.clone(), source }.to_string());
            }
        }

        self.phase = Phase::Done;
        self.step()
    }
}

impl Operation<crate::fs::RealFilesystem> {
    /// Convenience for the overwhelmingly common case: the real
    /// filesystem. Anything driving a test against a fake cross-device
    /// layout uses [`Operation::new`] directly with a
    /// `crate::fs::mock::MockFilesystem`.
    pub fn real(kind: OpKind, source: impl Into<PathBuf>, dest: impl Into<PathBuf>) -> Self {
        Operation::new(crate::fs::RealFilesystem, kind, source, dest)
    }
}

/// `base.join(relative)`, except when `relative` is empty — `Path::join`
/// with an empty `PathBuf` appends a trailing slash (`"a.txt".join("")`
/// is `"a.txt/"`), which turns a plain file's own path into one `stat`
/// refuses with `ENOTDIR`. The root entry's relative path is legitimately
/// empty (it *is* the root), so this has to be handled, not avoided.
fn join_relative(base: &Path, relative: &Path) -> PathBuf {
    if relative.as_os_str().is_empty() {
        base.to_path_buf()
    } else {
        base.join(relative)
    }
}

fn entry_kind_of(status: &fs::Metadata) -> EntryKind {
    if status.file_type().is_symlink() {
        EntryKind::Symlink
    } else if status.is_dir() {
        EntryKind::Dir
    } else {
        EntryKind::File { size: status.len() }
    }
}

/// Removes whatever is at `path`, file or directory, without following a
/// symlink there (a `Replace` decision on a colliding symlink must remove
/// the link, never whatever it points to).
fn remove_any(path: &Path) -> io::Result<()> {
    let status = fs::symlink_metadata(path)?;
    if status.is_dir() && !status.file_type().is_symlink() {
        fs::remove_dir_all(path)
    } else {
        fs::remove_file(path)
    }
}

/// `KeepBoth`'s destination: `dest`'s parent plus a disambiguated name,
/// reusing [`unique_name`] rather than a second `stem.N.ext` scheme.
fn renamed_sibling(dest: &Path) -> io::Result<PathBuf> {
    let parent = dest.parent().unwrap_or_else(|| Path::new("."));
    let name = dest.file_name().ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidInput, "destination path has no file name")
    })?;
    let unique = unique_name(parent, name)?;
    Ok(parent.join(unique))
}

/// Best-effort permission/mtime preservation for a just-copied file, using
/// the still-open handle so setting the time is one `fchtimes`-style call
/// on an fd this process already has open, not a second lookup by path.
/// Never fails the copy: losing these is logged, not fatal — see the
/// module doc's "What is not preserved" section for what is skipped
/// entirely rather than best-effort.
/// Copies the source's permissions and timestamps onto what was just
/// written, best-effort.
///
/// Best-effort throughout, and deliberately: a copy that landed but
/// whose mtime could not be set is a copy, not a failure, and a
/// destination filesystem that refuses either (FAT, a read-only-ish
/// mount, an NFS export squashing the caller) must not turn a
/// successful transfer into a reported error.
///
/// `what` names the thing in the two warnings, and is the only
/// difference there ever was between the file and directory versions of
/// this — setting `accessed` on a directory is as harmless as setting it
/// on a file, and both are already best-effort.
///
/// The source's `user.*` extended attributes come across too — a file's
/// tags among them — see `crate::xattr`.
fn preserve_attrs(handle: &File, source: &Path, dest_path: &Path, src_meta: &fs::Metadata, what: &'static str) {
    crate::xattr::copy_user(source, handle, dest_path);
    if let Err(e) = fs::set_permissions(dest_path, fs::Permissions::from_mode(src_meta.mode() & 0o7777)) {
        tracing::warn!(path = %dest_path.display(), error = %e, "could not preserve {what} permissions");
    }
    let mut times = fs::FileTimes::new();
    if let Ok(modified) = src_meta.modified() {
        times = times.set_modified(modified);
    }
    if let Ok(accessed) = src_meta.accessed() {
        times = times.set_accessed(accessed);
    }
    if let Err(e) = handle.set_times(times) {
        tracing::warn!(path = %dest_path.display(), error = %e, "could not preserve {what} mtime");
    }
}

/// Drives an [`Operation`] to completion, resolving every collision with
/// `decide`, for callers (and tests) that do not need to interleave their
/// own work between `step()` calls. Built entirely on the public
/// `step()`/`resolve()` seam — it is sugar, not a second code path.
pub fn drive<F: Filesystem>(
    op: &mut Operation<F>,
    mut decide: impl FnMut(&Collision) -> CollisionDecision,
) -> Report {
    loop {
        match op.step() {
            StepOutcome::Progress(_) => {}
            StepOutcome::Collision(collision) => {
                let decision = decide(&collision);
                op.resolve(decision);
            }
            StepOutcome::Done(report) => return report,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fs::mock::MockFilesystem;
    use std::os::unix::fs::PermissionsExt;
    use std::time::{Duration, SystemTime};

    fn write_file(path: &Path, contents: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, contents).unwrap();
    }

    fn no_collisions(_: &Collision) -> CollisionDecision {
        panic!("no collision was expected in this test");
    }

    /// A copy keeps what the person put on a file — its tags above all —
    /// and a folder's too.
    #[test]
    fn a_copy_keeps_the_user_attributes_of_files_and_folders() {
        let dir = tempfile::tempdir().unwrap();
        let src_root = dir.path().join("src");
        write_file(&src_root.join("a.txt"), "hello");
        let tagged = |p: &Path| crate::xattr::set(p, "user.xdg.tags", b"work");
        if let Err(e) = tagged(&src_root.join("a.txt")) {
            eprintln!("HYPRFORGE-SKIP: the temporary directory holds no user extended attributes ({e})");
            return;
        }
        tagged(&src_root).unwrap();
        crate::xattr::set(&src_root.join("a.txt"), "user.xdg.origin.url", b"https://example.org").unwrap();
        let dest = dir.path().join("dest");
        let mut op = Operation::real(OpKind::Copy, &src_root, &dest);
        let report = drive(&mut op, no_collisions);
        assert!(report.failed.is_empty(), "{:?}", report.failed);
        let get = |p: &Path, n: &str| crate::xattr::get(p, n).unwrap();
        assert_eq!(get(&dest.join("a.txt"), "user.xdg.tags").as_deref(), Some(&b"work"[..]));
        assert_eq!(get(&dest.join("a.txt"), "user.xdg.origin.url").as_deref(), Some(&b"https://example.org"[..]));
        assert_eq!(get(&dest, "user.xdg.tags").as_deref(), Some(&b"work"[..]), "the folder's own tag");
    }

    #[test]
    fn a_same_filesystem_move_takes_the_rename_syscall_path_not_copy_then_delete() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("a.txt");
        write_file(&source, "hello");
        let source_inode = fs::metadata(&source).unwrap().ino();
        let dest = dir.path().join("b.txt");

        let fs_seam = MockFilesystem::new(); // everything one device by default
        let mut op = Operation::new(fs_seam, OpKind::Move, &source, &dest);
        let report = drive(&mut op, no_collisions);

        assert_eq!(report.move_strategy, Some(MoveStrategy::Renamed));
        assert!(!source.exists());
        // A rename keeps the inode; a copy-then-delete would not — this
        // is the assertion on *how* it moved, not just that it did.
        assert_eq!(fs::metadata(&dest).unwrap().ino(), source_inode);
        assert_eq!(fs::read_to_string(&dest).unwrap(), "hello");
    }

    #[test]
    fn a_cross_filesystem_move_copies_then_deletes() {
        let dir = tempfile::tempdir().unwrap();
        let src_root = dir.path().join("src");
        let source = src_root.join("a.txt");
        write_file(&source, "hello");
        let dest = dir.path().join("dst").join("a.txt");
        fs::create_dir_all(dest.parent().unwrap()).unwrap();

        let fs_seam = MockFilesystem::new();
        fs_seam.mount(&src_root, 7); // a different device than dest's default

        let mut op = Operation::new(fs_seam, OpKind::Move, &source, &dest);
        let report = drive(&mut op, no_collisions);

        assert_eq!(report.move_strategy, Some(MoveStrategy::CopyThenDelete));
        assert!(!source.exists(), "source must be removed only after a verified copy");
        assert_eq!(fs::read_to_string(&dest).unwrap(), "hello");
    }

    #[test]
    fn a_cross_filesystem_move_leaves_the_source_untouched_if_the_copy_fails() {
        let dir = tempfile::tempdir().unwrap();
        let src_root = dir.path().join("src");
        let source = src_root.join("a.txt");
        write_file(&source, "hello");
        // A destination directory that does not exist makes `File::create`
        // fail — standing in for "the copy failed" without needing to
        // fake disk-full or permission errors.
        let dest = dir.path().join("does-not-exist").join("a.txt");

        let fs_seam = MockFilesystem::new();
        fs_seam.mount(&src_root, 7);

        let mut op = Operation::new(fs_seam, OpKind::Move, &source, &dest);
        let report = drive(&mut op, no_collisions);

        assert_eq!(report.move_strategy, Some(MoveStrategy::CopyThenDelete));
        assert!(!report.failed.is_empty(), "expected the copy to fail");
        assert!(source.exists(), "source must survive a failed copy");
        assert_eq!(fs::read_to_string(&source).unwrap(), "hello");
    }

    #[test]
    fn cancelling_mid_tree_leaves_the_source_completely_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let src_root = dir.path().join("src");
        for name in ["a.txt", "b.txt", "c.txt"] {
            write_file(&src_root.join(name), "contents");
        }
        let dest = dir.path().join("dst");

        let fs_seam = MockFilesystem::new();
        fs_seam.mount(&src_root, 7); // force copy-then-delete, not a rename

        let mut op = Operation::new(fs_seam, OpKind::Move, &src_root, &dest);
        // Let it get partway in, then cancel.
        for _ in 0..3 {
            op.step();
        }
        op.request_cancel();
        let report = drive(&mut op, no_collisions);

        assert!(report.cancelled);
        assert!(src_root.exists(), "source directory must still exist");
        for name in ["a.txt", "b.txt", "c.txt"] {
            assert_eq!(fs::read_to_string(src_root.join(name)).unwrap(), "contents");
        }
    }

    #[test]
    fn a_failure_on_one_file_does_not_abandon_the_rest_and_the_report_names_it() {
        let dir = tempfile::tempdir().unwrap();
        let src_root = dir.path().join("src");
        write_file(&src_root.join("good1.txt"), "one");
        write_file(&src_root.join("bad.txt"), "two");
        write_file(&src_root.join("good2.txt"), "three");
        std::fs::set_permissions(src_root.join("bad.txt"), std::fs::Permissions::from_mode(0o000)).unwrap();
        let dest = dir.path().join("dst");

        let mut op = Operation::real(OpKind::Copy, &src_root, &dest);
        let report = drive(&mut op, no_collisions);

        // Restore permissions so tempdir cleanup can remove it.
        std::fs::set_permissions(src_root.join("bad.txt"), std::fs::Permissions::from_mode(0o644)).unwrap();

        // Root process running as UID 0 ignores the permission bits, so
        // this assertion only holds unprivileged — matching every other
        // permission-based test already in this crate.
        assert_eq!(report.failed.len(), 1, "report: {report:?}");
        assert_eq!(report.failed[0].0, src_root.join("bad.txt"));
        assert!(dest.join("good1.txt").exists());
        assert!(dest.join("good2.txt").exists());
        assert!(!dest.join("bad.txt").exists());
    }

    /// Root ignores permission bits, so these say so rather than pass
    /// having checked nothing — the `HYPRFORGE-SKIP` convention.
    fn running_as_root() -> bool {
        let root = std::fs::metadata("/proc/self").map(|m| m.uid() == 0).unwrap_or(false);
        if root {
            eprintln!("HYPRFORGE-SKIP: running as root, which ignores the permission bits this test needs");
        }
        root
    }

    #[test]
    fn a_file_that_cannot_be_read_is_said_to_be_unreadable_not_unwritable() {
        if running_as_root() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("secret.txt");
        write_file(&source, "x");
        std::fs::set_permissions(&source, std::fs::Permissions::from_mode(0o000)).unwrap();
        let dest = dir.path().join("copy.txt");

        let report = drive(&mut Operation::real(OpKind::Copy, &source, &dest), no_collisions);
        std::fs::set_permissions(&source, std::fs::Permissions::from_mode(0o644)).unwrap();

        assert_eq!(report.failed.len(), 1, "report: {report:?}");
        let said = &report.failed[0].1;
        assert_eq!(*said, format!("you don't have permission to read {}", source.display()));
        assert_eq!(report.denied, vec![source.clone()], "carried as data, not only as a sentence");
    }

    #[test]
    fn a_folder_that_cannot_be_listed_is_said_to_be_unreadable() {
        if running_as_root() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("locked");
        fs::create_dir(&source).unwrap();
        write_file(&source.join("inside.txt"), "x");
        std::fs::set_permissions(&source, std::fs::Permissions::from_mode(0o000)).unwrap();

        let report = drive(&mut Operation::real(OpKind::Copy, &source, dir.path().join("dst")), no_collisions);
        std::fs::set_permissions(&source, std::fs::Permissions::from_mode(0o755)).unwrap();

        assert!(
            report.failed.iter().any(|(_, said)| *said == format!("you don't have permission to read {}", source.display())),
            "report: {report:?}"
        );
        assert!(report.denied.contains(&source), "report: {report:?}");
    }

    #[test]
    fn a_destination_that_cannot_be_written_is_still_said_to_be_unwritable() {
        if running_as_root() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("a.txt");
        write_file(&source, "x");
        let shut = dir.path().join("shut");
        fs::create_dir(&shut).unwrap();
        std::fs::set_permissions(&shut, std::fs::Permissions::from_mode(0o555)).unwrap();
        let dest = shut.join("a.txt");

        let report = drive(&mut Operation::real(OpKind::Copy, &source, &dest), no_collisions);
        std::fs::set_permissions(&shut, std::fs::Permissions::from_mode(0o755)).unwrap();

        assert_eq!(report.failed.len(), 1, "report: {report:?}");
        assert_eq!(report.failed[0].1, format!("you don't have permission to write to {}", dest.display()));
        assert_eq!(report.denied.len(), 1, "report: {report:?}");
    }

    #[test]
    fn a_refused_rename_names_both_folders_rather_than_blaming_one() {
        if running_as_root() {
            return;
        }
        // Same filesystem, so the move is a rename; the *source's* folder
        // is the read-only one. Blaming the destination — what this said
        // before — would send someone to fix a folder that is fine.
        let dir = tempfile::tempdir().unwrap();
        let from_dir = dir.path().join("from");
        fs::create_dir(&from_dir).unwrap();
        let source = from_dir.join("a.txt");
        write_file(&source, "x");
        std::fs::set_permissions(&from_dir, std::fs::Permissions::from_mode(0o555)).unwrap();
        let dest = dir.path().join("a.txt");

        let report = drive(&mut Operation::real(OpKind::Move, &source, &dest), no_collisions);
        std::fs::set_permissions(&from_dir, std::fs::Permissions::from_mode(0o755)).unwrap();

        assert_eq!(report.failed.len(), 1, "report: {report:?}");
        assert!(report.failed[0].1.starts_with(&format!("you don't have permission to move {}", source.display())), "{report:?}");
        assert!(source.exists(), "a refused move leaves the source where it was");
        assert_eq!(report.denied, vec![source.clone()]);
    }

    /// Only permission is "denied": the one failure a retry with more of
    /// it could fix. A file that is not there is not offered again as
    /// administrator.
    #[test]
    fn a_failure_that_is_not_about_permission_is_not_called_denied() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("never-was.txt");
        let report = drive(&mut Operation::real(OpKind::Copy, &source, dir.path().join("copy.txt")), no_collisions);
        assert!(!report.failed.is_empty(), "report: {report:?}");
        assert!(report.denied.is_empty(), "report: {report:?}");
    }

    #[test]
    fn permission_is_recognised_in_every_variant_that_can_carry_it() {
        let p = PathBuf::from("/x");
        let denied = || io::Error::from(io::ErrorKind::PermissionDenied);
        let other = || io::Error::from(io::ErrorKind::Other);
        assert!(OpsError::PermissionDenied { path: p.clone() }.is_permission());
        assert!(OpsError::ReadDenied { path: p.clone() }.is_permission());
        assert!(OpsError::MoveDenied { from: p.clone(), to: p.clone() }.is_permission());
        assert!(OpsError::Remove { path: p.clone(), source: denied() }.is_permission());
        assert!(OpsError::ListDir { path: p.clone(), source: denied() }.is_permission());
        assert!(!OpsError::Remove { path: p.clone(), source: other() }.is_permission());
        assert!(!OpsError::DiskFull { path: p.clone() }.is_permission());
        assert!(!OpsError::SourceVanished { path: p }.is_permission());
    }

    #[test]
    fn collision_skip_leaves_the_existing_destination_file_alone() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("src.txt");
        write_file(&source, "new");
        let dest = dir.path().join("dst.txt");
        write_file(&dest, "old");

        let mut op = Operation::real(OpKind::Copy, &source, &dest);
        let report = drive(&mut op, |_| CollisionDecision { policy: CollisionPolicy::Skip, apply_to_rest: false });

        assert_eq!(report.skipped, vec![source.clone()]);
        assert_eq!(fs::read_to_string(&dest).unwrap(), "old");
        assert!(source.exists());
    }

    #[test]
    fn collision_replace_overwrites_the_existing_destination_file() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("src.txt");
        write_file(&source, "new");
        let dest = dir.path().join("dst.txt");
        write_file(&dest, "old");

        let mut op = Operation::real(OpKind::Copy, &source, &dest);
        let report = drive(&mut op, |_| CollisionDecision { policy: CollisionPolicy::Replace, apply_to_rest: false });

        assert_eq!(report.succeeded, vec![dest.clone()]);
        assert_eq!(fs::read_to_string(&dest).unwrap(), "new");
    }

    #[test]
    fn collision_keep_both_auto_renames_using_the_trash_disambiguation_scheme() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("src.txt");
        write_file(&source, "new");
        let dest = dir.path().join("dst.txt");
        write_file(&dest, "old");

        let mut op = Operation::real(OpKind::Copy, &source, &dest);
        let report = drive(&mut op, |_| CollisionDecision { policy: CollisionPolicy::KeepBoth, apply_to_rest: false });

        assert_eq!(report.succeeded, vec![dir.path().join("dst.2.txt")]);
        assert_eq!(fs::read_to_string(dir.path().join("dst.2.txt")).unwrap(), "new");
        assert_eq!(fs::read_to_string(&dest).unwrap(), "old", "the original must be untouched");
        assert_eq!(report.dest, dir.path().join("dst.2.txt"), "the report says where it really went");
    }

    #[test]
    fn the_report_names_the_destination_asked_for_when_nothing_was_in_the_way() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("src.txt");
        write_file(&source, "x");
        let dest = dir.path().join("dst.txt");
        let mut op = Operation::real(OpKind::Copy, &source, &dest);
        let report = drive(&mut op, |_| unreachable!("nothing is in the way"));
        assert_eq!(report.dest, dest);
    }

    #[test]
    fn collision_cancel_stops_the_whole_operation() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("src.txt");
        write_file(&source, "new");
        let dest = dir.path().join("dst.txt");
        write_file(&dest, "old");

        let mut op = Operation::real(OpKind::Copy, &source, &dest);
        let report = drive(&mut op, |_| CollisionDecision { policy: CollisionPolicy::Cancel, apply_to_rest: false });

        assert!(report.cancelled);
        assert!(report.succeeded.is_empty());
        assert_eq!(fs::read_to_string(&dest).unwrap(), "old");
    }

    #[test]
    fn apply_to_rest_resolves_every_later_collision_without_asking_again() {
        let dir = tempfile::tempdir().unwrap();
        let src_root = dir.path().join("src");
        write_file(&src_root.join("a.txt"), "new-a");
        write_file(&src_root.join("b.txt"), "new-b");
        let dst_root = dir.path().join("dst");
        write_file(&dst_root.join("a.txt"), "old-a");
        write_file(&dst_root.join("b.txt"), "old-b");

        let mut op = Operation::real(OpKind::Copy, &src_root, &dst_root);
        let mut asked = 0;
        let report = drive(&mut op, |_| {
            asked += 1;
            CollisionDecision { policy: CollisionPolicy::Replace, apply_to_rest: true }
        });

        assert_eq!(asked, 1, "only the first collision should need asking");
        assert_eq!(fs::read_to_string(dst_root.join("a.txt")).unwrap(), "new-a");
        assert_eq!(fs::read_to_string(dst_root.join("b.txt")).unwrap(), "new-b");
        assert!(report.failed.is_empty(), "report: {report:?}");
    }

    #[test]
    fn a_symlink_is_copied_as_a_symlink_not_followed() {
        let dir = tempfile::tempdir().unwrap();
        let src_root = dir.path().join("src");
        let target = dir.path().join("target.txt");
        write_file(&target, "content");
        fs::create_dir_all(&src_root).unwrap();
        std::os::unix::fs::symlink(&target, src_root.join("link")).unwrap();
        let dest = dir.path().join("dst");

        let mut op = Operation::real(OpKind::Copy, &src_root, &dest);
        let report = drive(&mut op, no_collisions);

        assert!(report.failed.is_empty(), "report: {report:?}");
        let dest_link = dest.join("link");
        let meta = fs::symlink_metadata(&dest_link).unwrap();
        assert!(meta.file_type().is_symlink(), "must land as a symlink, not the file it points to");
        assert_eq!(fs::read_link(&dest_link).unwrap(), target);
    }

    #[test]
    fn a_tree_containing_a_symlink_to_its_own_ancestor_terminates() {
        let dir = tempfile::tempdir().unwrap();
        let src_root = dir.path().join("src");
        let sub = src_root.join("sub");
        fs::create_dir_all(&sub).unwrap();
        write_file(&sub.join("file.txt"), "x");
        // Points back at an ancestor directory — following it would
        // recurse forever; this crate must never even try.
        std::os::unix::fs::symlink(&src_root, sub.join("loop")).unwrap();
        let dest = dir.path().join("dst");

        let mut op = Operation::real(OpKind::Copy, &src_root, &dest);
        let mut steps = 0;
        let report = loop {
            match op.step() {
                StepOutcome::Done(report) => break report,
                StepOutcome::Collision(_) => panic!("no collision expected"),
                StepOutcome::Progress(_) => {}
            }
            steps += 1;
            assert!(steps < 10_000, "operation did not terminate — the symlink loop was followed");
        };

        assert!(report.failed.is_empty(), "report: {report:?}");
        assert!(fs::symlink_metadata(dest.join("sub").join("loop")).unwrap().file_type().is_symlink());
    }

    #[test]
    fn copying_a_file_preserves_its_bytes_and_its_mtime() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("a.txt");
        write_file(&source, "exact bytes, byte for byte");
        let old_time = SystemTime::now() - Duration::from_secs(3 * 24 * 60 * 60);
        let f = fs::File::options().write(true).open(&source).unwrap();
        f.set_times(fs::FileTimes::new().set_modified(old_time)).unwrap();
        drop(f);
        let dest = dir.path().join("b.txt");

        let mut op = Operation::real(OpKind::Copy, &source, &dest);
        let report = drive(&mut op, no_collisions);

        assert!(report.failed.is_empty(), "report: {report:?}");
        assert_eq!(fs::read(&dest).unwrap(), fs::read(&source).unwrap());
        let src_mtime = fs::metadata(&source).unwrap().modified().unwrap();
        let dst_mtime = fs::metadata(&dest).unwrap().modified().unwrap();
        let diff = src_mtime.duration_since(dst_mtime).unwrap_or_else(|e| e.duration());
        assert!(diff < Duration::from_secs(1), "mtime not preserved: {src_mtime:?} vs {dst_mtime:?}");
    }

    #[test]
    fn an_empty_directory_copies_as_an_empty_directory() {
        let dir = tempfile::tempdir().unwrap();
        let src_root = dir.path().join("src");
        fs::create_dir_all(src_root.join("empty")).unwrap();
        let dest = dir.path().join("dst");

        let mut op = Operation::real(OpKind::Copy, &src_root, &dest);
        let report = drive(&mut op, no_collisions);

        assert!(report.failed.is_empty(), "report: {report:?}");
        let dest_empty = dest.join("empty");
        assert!(dest_empty.is_dir());
        assert_eq!(fs::read_dir(&dest_empty).unwrap().count(), 0);
    }

    #[test]
    fn a_large_file_copy_can_be_cancelled_mid_file_not_only_between_files() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("big.bin");
        // Bigger than one CHUNK_SIZE so at least one step() lands
        // mid-file, proving cancellation is checked more often than once
        // per file.
        fs::write(&source, vec![7u8; CHUNK_SIZE * 3]).unwrap();
        let dest = dir.path().join("big-copy.bin");

        let mut op = Operation::real(OpKind::Copy, &source, &dest);
        // First step opens the file and does nothing else; second step
        // copies exactly one chunk — still short of the whole file.
        op.step();
        op.step();
        op.request_cancel();
        let report = drive(&mut op, no_collisions);

        assert!(report.cancelled);
        assert!(!dest.exists(), "a cancelled mid-file copy must not leave a partial file behind");
        assert_eq!(fs::read(&source).unwrap().len(), CHUNK_SIZE * 3, "source must be untouched");
    }

    /// Found live in Files: a one-file copy reported a total of 0 bytes,
    /// so its progress read "5.9 GiB of 0 B" and its bar could not fill.
    #[test]
    fn copying_one_file_reports_that_files_size_as_the_total() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("big.bin");
        fs::write(&source, vec![7u8; CHUNK_SIZE * 2 + 5]).unwrap();
        let mut op = Operation::real(OpKind::Copy, &source, dir.path().join("copy.bin"));
        let totals: Vec<Option<u64>> = std::iter::from_fn(|| match op.step() {
            StepOutcome::Progress(p) => Some(p.bytes_total),
            _ => None,
        })
        .collect();
        assert!(!totals.is_empty());
        for total in totals.into_iter().flatten() {
            assert_eq!(total, (CHUNK_SIZE * 2 + 5) as u64);
        }
    }
}
