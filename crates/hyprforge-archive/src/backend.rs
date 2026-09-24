//! The seam between this crate's logic and the archive libraries under
//! it.
//!
//! The trait and its mock come before the real readers, deliberately —
//! the order CLAUDE.md records for the D-Bus modules, applied to a
//! different kind of "somebody else's code". The machine running tier 1
//! has no guaranteed tarball, no encrypted zip and no five-gigabyte 7z,
//! and everything above this trait (the tree, the routing, the browser's
//! behaviour when a member cannot be read) should be testable without
//! arranging any of them.
//!
//! Synchronous, like [`hyprforge_files_core::backend::FsBackend`] and
//! for the same reason: this is a library over files, not a service
//! behind a socket, and the caller that must not block already knows how
//! to move blocking work off its thread.

use crate::error::Result;
use crate::format::Format;
use crate::model::Index;
use crate::unlock::Unlock;
use std::path::{Path, PathBuf};

/// Whether a long operation should keep going.
///
/// Cancellation is the *return value* of a progress report rather than a
/// flag read from somewhere else, so there is exactly one place per loop
/// where both questions get asked and no way to report progress without
/// also giving the person a chance to stop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flow {
    Continue,
    Cancel,
}

/// Where a long operation has got to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Advance<'a> {
    /// The member being worked on right now.
    pub member: &'a str,
    pub files_done: usize,
    pub files_total: usize,
    pub bytes_done: u64,
    /// Uncompressed bytes expected in total — a lower bound, since a
    /// member whose size the archive never stated contributes nothing.
    /// See [`crate::model::Member::size_known`].
    pub bytes_total: u64,
}

pub trait Progress: Send {
    fn advance(&mut self, advance: Advance<'_>) -> Flow;
}

/// For a caller with nothing to report to and nothing to cancel.
pub struct NoProgress;

impl Progress for NoProgress {
    fn advance(&mut self, _advance: Advance<'_>) -> Flow {
        Flow::Continue
    }
}

/// What to do when an extracted member already exists at the
/// destination.
///
/// No `Default`. Overwriting someone's files and silently skipping them
/// are both defensible and they are not the same, so the caller states
/// which it meant — there is no answer here that is safe enough to
/// assume on their behalf.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Collision {
    Overwrite,
    /// Leave what is on disk and carry on; the report counts what was
    /// passed over, so the caller can say so rather than implying
    /// everything was written.
    Skip,
    /// Write beside it under a free name — `notes.txt`, `notes (1).txt`.
    Rename,
}

/// Which members to pull out, and where to put them.
#[derive(Debug, Clone)]
pub struct ExtractRequest {
    /// The members to extract; empty means everything.
    ///
    /// A directory named here brings everything under it — selecting a
    /// folder inside an archive and extracting it is the ordinary case,
    /// and making the caller expand the subtree itself would mean
    /// teaching every caller the tree.
    pub members: Vec<String>,
    pub dest: PathBuf,
    /// Dropped from the front of each member's path as it is written.
    ///
    /// This is what makes "extract this inner folder *here*" put
    /// `docs/guide.txt` at `guide.txt` rather than recreating the
    /// archive's own directory above it.
    pub strip_prefix: Option<String>,
    pub collision: Collision,
}

/// What an extraction did — including the parts it did not do.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ExtractReport {
    pub files: usize,
    pub dirs: usize,
    pub bytes: u64,
    /// Members left alone under [`Collision::Skip`].
    pub skipped: Vec<String>,
    /// Members that could not be written, each with a reason.
    ///
    /// A list rather than a failure, for the reason `StdBackend::read_dir`
    /// gives for a directory entry it cannot describe: one member with a
    /// broken compressed stream must not turn "here are the other four
    /// hundred files" into "nothing was extracted". The caller reports
    /// what is in here; it is never empty silently.
    pub failed: Vec<MemberFailure>,
}

/// One member that could not be written, and why.
///
/// The `reason` is the point. This was a `(String, String)` of member
/// and message, and the one caller that had to *act* on a failure —
/// opening an encrypted file, which needs a password prompt rather than
/// an error — could only do it by searching the message for the word
/// "password". That is a coupling to wording, and wording is exactly
/// the thing anyone is free to improve; the first reworded sentence
/// would have turned the prompt into a dead end with nothing failing to
/// say so.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemberFailure {
    pub member: String,
    pub reason: FailureReason,
    /// Already a sentence, for showing.
    pub message: String,
}

/// What kind of failure, for a caller that does something different
/// about one of them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureReason {
    /// This member is encrypted and the password was wrong or absent.
    /// The caller asks; it is not an error to report.
    NeedsPassword,
    /// Anything else — already described by `message`.
    Other,
}

impl MemberFailure {
    pub fn new(member: impl Into<String>, error: &crate::error::ArchiveError) -> MemberFailure {
        MemberFailure {
            member: member.into(),
            reason: match error {
                crate::error::ArchiveError::PasswordRequired { .. } => {
                    FailureReason::NeedsPassword
                }
                _ => FailureReason::Other,
            },
            message: error.to_string(),
        }
    }

    /// For a failure that is not an [`crate::error::ArchiveError`] — a
    /// directory that would not be created, a symlink refused.
    pub fn plain(member: impl Into<String>, message: impl Into<String>) -> MemberFailure {
        MemberFailure {
            member: member.into(),
            reason: FailureReason::Other,
            message: message.into(),
        }
    }
}

/// One thing going *into* a new archive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Source {
    /// Where it is on disk.
    pub path: PathBuf,
    /// What it is called inside the archive. Set by the caller rather
    /// than derived here, because "compress these three files" and
    /// "compress this folder" want different names for the same paths
    /// and only the caller knows which was asked for.
    pub as_member: String,
}

/// A change to an existing archive.
///
/// Every format here rewrites the whole file to apply one of these (see
/// [`crate::write`]), so they are applied as a batch: three separate
/// deletions are three full rewrites, and one batch of three is one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Edit {
    Add { source: PathBuf, as_member: String },
    /// Removes the member, and everything under it if it is a directory.
    Remove(String),
    Rename { from: String, to: String },
}

pub trait ArchiveBackend: Send + Sync {
    /// Reads the whole table of contents.
    ///
    /// Everything, in one call, rather than a lazy per-directory read:
    /// a tar has no table of contents at all — the only way to know what
    /// is in one is to walk every member, decompressing the whole stream
    /// on the way — so "list this one directory" is not a cheaper
    /// question than "list everything", and pretending otherwise would
    /// re-scan the archive at every step of a walk down into it.
    fn index_with(&self, archive: &Path, unlock: &Unlock) -> Result<Index>;

    /// One member's contents.
    fn read_member_with(&self, archive: &Path, member: &str, unlock: &Unlock) -> Result<Vec<u8>>;

    fn extract_with(
        &self,
        archive: &Path,
        request: &ExtractRequest,
        unlock: &Unlock,
        progress: &mut dyn Progress,
    ) -> Result<ExtractReport>;

    /// Applies `edits` to an existing archive, in place.
    ///
    /// Takes an `unlock` because a rewrite reads the whole archive
    /// first — editing an encrypted one needs the password just as
    /// reading it does.
    fn edit_with(
        &self,
        archive: &Path,
        edits: &[Edit],
        unlock: &Unlock,
        progress: &mut dyn Progress,
    ) -> Result<()>;

    /// Writes a brand new archive at `dest`.
    ///
    /// No `unlock`: writing an *encrypted* archive is a separate feature
    /// with its own question to ask (which cipher, and is the header
    /// encrypted too), and a password parameter that silently did
    /// nothing would be worse than not having one.
    fn create(
        &self,
        dest: &Path,
        format: Format,
        sources: &[Source],
        progress: &mut dyn Progress,
    ) -> Result<()>;

    // --- the same four, for the overwhelmingly common unencrypted case.
    //
    // Defaults rather than a second trait, and the short name is the one
    // without a password because that is what nearly every call site
    // means: a caller that has no password should not have to say so
    // four times per function. An implementor overrides the `_with`
    // forms and gets these for free.

    fn index(&self, archive: &Path) -> Result<Index> {
        self.index_with(archive, &Unlock::none())
    }

    fn read_member(&self, archive: &Path, member: &str) -> Result<Vec<u8>> {
        self.read_member_with(archive, member, &Unlock::none())
    }

    fn extract(
        &self,
        archive: &Path,
        request: &ExtractRequest,
        progress: &mut dyn Progress,
    ) -> Result<ExtractReport> {
        self.extract_with(archive, request, &Unlock::none(), progress)
    }

    fn edit(&self, archive: &Path, edits: &[Edit], progress: &mut dyn Progress) -> Result<()> {
        self.edit_with(archive, edits, &Unlock::none(), progress)
    }
}

#[cfg(any(test, feature = "mock"))]
pub mod mock {
    use super::*;
    use crate::error::ArchiveError;
    use crate::model::Member;
    use std::collections::HashMap;
    use std::sync::Mutex;

    /// An archive that holds what the test says it holds.
    ///
    /// Seeded per archive path, with each member's bytes beside its
    /// metadata so `read_member` and `index` cannot disagree about which
    /// members exist — a mock whose two answers about the same archive
    /// differ is worse than no mock, the note `MockBackend` in
    /// `hyprforge-files-core` makes about `count_children`.
    /// One seeded member: what it is, and what is in it.
    type Seeded = Vec<(Member, Vec<u8>)>;

    #[derive(Default)]
    pub struct MockArchives {
        archives: Mutex<HashMap<PathBuf, Seeded>>,
        /// Archives that fail to open at all.
        unreadable: Mutex<HashMap<PathBuf, String>>,
        /// Archives that will not open without a password, and the one
        /// that works.
        needs_password: Mutex<HashMap<PathBuf, String>>,
        /// Members whose *contents* cannot be read although they list
        /// perfectly well — a damaged compressed stream for one file in
        /// an otherwise fine zip, which is the case that must not sink a
        /// whole extraction.
        bad_members: Mutex<HashMap<PathBuf, Vec<String>>>,
    }

    impl MockArchives {
        pub fn new() -> Self {
            Self::default()
        }

        /// Seeds an archive from `(member path, contents)` pairs. A path
        /// ending in `/` is a directory and its contents are ignored.
        pub fn seed(&self, archive: impl Into<PathBuf>, members: &[(&str, &[u8])]) {
            let built = members
                .iter()
                .map(|(path, bytes)| {
                    if path.ends_with('/') {
                        (Member::dir(*path), Vec::new())
                    } else {
                        (Member::file(*path, bytes.len() as u64), bytes.to_vec())
                    }
                })
                .collect();
            self.archives.lock().unwrap().insert(archive.into(), built);
        }

        /// After this, opening `archive` fails with `why`.
        pub fn make_damaged(&self, archive: impl Into<PathBuf>, why: impl Into<String>) {
            self.unreadable.lock().unwrap().insert(archive.into(), why.into());
        }

        /// After this, `archive` needs `password` to open at all.
        pub fn make_encrypted(&self, archive: impl Into<PathBuf>, password: impl Into<String>) {
            self.needs_password
                .lock()
                .unwrap()
                .insert(archive.into(), password.into());
        }

        /// After this, `member` lists but will not read.
        pub fn make_member_unreadable(&self, archive: impl Into<PathBuf>, member: impl Into<String>) {
            self.bad_members
                .lock()
                .unwrap()
                .entry(archive.into())
                .or_default()
                .push(member.into());
        }

        fn members(&self, archive: &Path) -> Result<Seeded> {
            if let Some(why) = self.unreadable.lock().unwrap().get(archive) {
                return Err(ArchiveError::Damaged {
                    path: archive.to_path_buf(),
                    format: "mock",
                    detail: why.clone(),
                });
            }
            self.archives
                .lock()
                .unwrap()
                .get(archive)
                .cloned()
                .ok_or_else(|| ArchiveError::NotAnArchive {
                    path: archive.to_path_buf(),
                })
        }

        /// `Err` when this archive needs a password and `unlock` is not
        /// it.
        fn check_password(&self, archive: &Path, unlock: &Unlock) -> Result<()> {
            let held = self.needs_password.lock().unwrap();
            let Some(wanted) = held.get(archive) else {
                return Ok(());
            };
            match unlock.expose() {
                Some(given) if given == wanted => Ok(()),
                // A wrong password and no password are the same answer
                // to the caller: ask. Distinguishing them would mean
                // telling whoever is guessing that they are close.
                _ => Err(ArchiveError::PasswordRequired {
                    path: archive.to_path_buf(),
                }),
            }
        }

        fn member_is_bad(&self, archive: &Path, member: &str) -> bool {
            self.bad_members
                .lock()
                .unwrap()
                .get(archive)
                .is_some_and(|bad| bad.iter().any(|b| b == member))
        }
    }

    impl ArchiveBackend for MockArchives {
        fn index_with(&self, archive: &Path, unlock: &Unlock) -> Result<Index> {
            // A 7z can encrypt its header, so listing is the first thing
            // a password can be needed for — modelled here so the
            // caller's "prompt, then read again" path has something to
            // fail against.
            self.check_password(archive, unlock)?;
            Ok(Index::build(
                self.members(archive)?.into_iter().map(|(m, _)| m).collect(),
            ))
        }

        fn read_member_with(&self, archive: &Path, member: &str, unlock: &Unlock) -> Result<Vec<u8>> {
            self.check_password(archive, unlock)?;
            if self.member_is_bad(archive, member) {
                return Err(ArchiveError::Damaged {
                    path: archive.to_path_buf(),
                    format: "mock",
                    detail: format!("“{member}” will not decompress"),
                });
            }
            let wanted = crate::model::normalise(member);
            self.members(archive)?
                .into_iter()
                .find(|(m, _)| Some(&m.path) == wanted.as_ref())
                .map(|(_, bytes)| bytes)
                .ok_or_else(|| ArchiveError::MemberNotFound {
                    archive: archive.to_path_buf(),
                    member: member.to_string(),
                })
        }

        fn extract_with(
            &self,
            archive: &Path,
            request: &ExtractRequest,
            unlock: &Unlock,
            progress: &mut dyn Progress,
        ) -> Result<ExtractReport> {
            // Through the same planner and writer the real backends use,
            // so the mock cannot drift from them on the questions this
            // crate's own rules live in — which members a selected
            // directory brings, what `strip_prefix` does, and that
            // nothing lands outside the destination.
            let index = self.index_with(archive, unlock)?;
            let plan = crate::extract::plan(&index, request, archive)?;
            crate::extract::run(&plan, request, progress, |member| {
                self.read_member_with(archive, member, unlock)
            })
        }

        fn create(
            &self,
            dest: &Path,
            _format: Format,
            sources: &[Source],
            _progress: &mut dyn Progress,
        ) -> Result<()> {
            let members = sources
                .iter()
                .map(|s| {
                    let bytes = std::fs::read(&s.path).unwrap_or_default();
                    (Member::file(s.as_member.clone(), bytes.len() as u64), bytes)
                })
                .collect();
            self.archives.lock().unwrap().insert(dest.to_path_buf(), members);
            Ok(())
        }

        fn edit_with(
            &self,
            archive: &Path,
            edits: &[Edit],
            unlock: &Unlock,
            _progress: &mut dyn Progress,
        ) -> Result<()> {
            self.check_password(archive, unlock)?;
            let mut members = self.members(archive)?;
            for edit in edits {
                crate::write::apply_to_list(&mut members, edit);
            }
            self.archives.lock().unwrap().insert(archive.to_path_buf(), members);
            Ok(())
        }
    }
}
