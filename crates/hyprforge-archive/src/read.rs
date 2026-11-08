//! The real archives, over `zip`, `tar` and `sevenz-rust2`.
//!
//! Three libraries with three shapes, flattened to one [`Index`] and one
//! way of getting a member's bytes. Everything format-specific stops
//! here.
//!
//! # tar is not like the other two
//!
//! A zip and a 7z have a table of contents: the reader seeks to it, and
//! answering "what is in here" costs one read regardless of size.
//! A tar has no index whatsoever — it is a stream of headers each
//! followed by its file — so the only way to know what is inside one is
//! to walk the whole thing, and if it is compressed, to decompress the
//! whole thing on the way. That is not a flaw in this code and it cannot
//! be optimised away; it is what the format is.
//!
//! Two consequences run through this module. Listing a large `.tar.xz`
//! is genuinely slow, and the caller must treat it as blocking work (the
//! file manager caches the index per archive for exactly this reason).
//! And extracting from a tar must be a *single pass* — see
//! [`crate::extract::Run`] — because asking a tar for members one at a
//! time re-reads the whole stream per member.

use crate::backend::{
    Advance, ArchiveBackend, Edit, ExtractReport, ExtractRequest, Flow, Progress, Source,
};
use crate::error::{ArchiveError, Result};
use crate::extract::{self, PlanItem};
use crate::format::{self, Compression, Format};
use crate::model::{Index, Member};
use crate::pin::Pinned;
use crate::timestamp;
use crate::unlock::Unlock;
use std::cell::Cell;
use std::collections::HashMap;
use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};
use std::rc::Rc;

/// Archives on the real filesystem.
pub struct StdArchives;

impl StdArchives {
    /// What this file is, by its bytes.
    ///
    /// Every entry point starts here rather than trusting the extension,
    /// so a `.zip` that is really a tarball opens and a `.zip` that is
    /// really a photo is reported as not an archive instead of as a
    /// damaged one.
    fn format_of(archive: &Path) -> Result<Format> {
        match format::sniff(archive) {
            Ok(Some(format)) => Ok(format),
            Ok(None) => Err(ArchiveError::NotAnArchive {
                path: archive.to_path_buf(),
            }),
            Err(e) => Err(ArchiveError::io(archive, e)),
        }
    }

    fn open(archive: &Path) -> Result<BufReader<std::fs::File>> {
        std::fs::File::open(archive)
            .map(BufReader::new)
            .map_err(|e| ArchiveError::io(archive, e))
    }
}

// --- zip ------------------------------------------------------------

fn zip_open(archive: &Path) -> Result<zip::ZipArchive<BufReader<std::fs::File>>> {
    zip::ZipArchive::new(StdArchives::open(archive)?).map_err(|e| zip_error(archive, e))
}

fn zip_error(archive: &Path, error: zip::result::ZipError) -> ArchiveError {
    match error {
        zip::result::ZipError::FileNotFound => ArchiveError::MemberNotFound {
            archive: archive.to_path_buf(),
            member: String::new(),
        },
        zip::result::ZipError::UnsupportedArchive(detail) => ArchiveError::Unsupported {
            path: archive.to_path_buf(),
            detail: detail.to_string(),
        },
        zip::result::ZipError::Io(e) => ArchiveError::io(archive, e),
        other => ArchiveError::Damaged {
            path: archive.to_path_buf(),
            format: "zip",
            detail: other.to_string(),
        },
    }
}

fn zip_index(archive: &Path) -> Result<Index> {
    let mut zip = zip_open(archive)?;
    let mut members = Vec::with_capacity(zip.len());
    for i in 0..zip.len() {
        // A single unreadable central-directory entry skips rather than
        // failing the listing — the rule `StdBackend::read_dir` follows
        // for a directory entry whose metadata will not come back.
        let entry = match zip.by_index_raw(i) {
            Ok(entry) => entry,
            Err(e) => {
                tracing::warn!(
                    archive = %archive.display(),
                    index = i,
                    error = %e,
                    "skipping a zip entry whose header could not be read"
                );
                continue;
            }
        };
        let name = entry.name().to_string();
        let is_dir = entry.is_dir();
        members.push(Member {
            path: name,
            is_dir,
            size: entry.size(),
            // A zip always states a size in its central directory, even
            // for an entry streamed with a data descriptor — the
            // descriptor is what the *local* header lacks.
            size_known: true,
            compressed: Some(entry.compressed_size()),
            modified: entry.last_modified().and_then(|dos| {
                timestamp::from_civil_utc(
                    dos.year() as i32,
                    dos.month() as u32,
                    dos.day() as u32,
                    dos.hour() as u32,
                    dos.minute() as u32,
                    dos.second() as u32,
                )
            }),
            mode: entry.unix_mode(),
            link_target: None,
            hard_link: false,
            encrypted: entry.encrypted(),
        });
    }

    // A zip records a symlink as an ordinary member whose *contents* are
    // the target, flagged in the unix mode. Reading those contents means
    // a second pass, because the borrow above is over the whole archive
    // — and it is only worth doing for the handful of entries the mode
    // actually flags, which is why it is not folded into the loop.
    let links: Vec<usize> = members
        .iter()
        .enumerate()
        .filter(|(_, m)| m.mode.is_some_and(is_symlink_mode))
        .map(|(i, _)| i)
        .collect();
    // Read through a limit, because those contents are as long as the
    // archive says: a "link" whose target is two gigabytes of one
    // letter compresses to almost nothing and was read whole into the
    // listing. No filesystem here takes a target past `PATH_MAX`, so a
    // longer one is not a link anything could create — it is listed
    // without a target, and unpacks as the plain file it then is.
    for i in links {
        let name = members[i].path.clone();
        if let Ok(entry) = zip.by_name(&name) {
            let mut target = String::new();
            let read = entry.take(LINK_TARGET_MAX as u64 + 1).read_to_string(&mut target);
            if read.is_ok() && target.len() <= LINK_TARGET_MAX {
                members[i].link_target = Some(target);
            } else {
                tracing::warn!(
                    archive = %archive.display(),
                    "a zip symlink's target is too long, or not text; listing it as a file"
                );
            }
        }
    }

    Ok(Index::build(members))
}

/// The longest symlink target this reads out of a zip — Linux's
/// `PATH_MAX`, which no target can exceed and still be created.
const LINK_TARGET_MAX: usize = 4096;

/// Whether a Unix mode says "this is a symlink" — `S_IFLNK`, the file
/// type bits above the permissions.
fn is_symlink_mode(mode: u32) -> bool {
    mode & 0o170000 == 0o120000
}

fn zip_read_member(archive: &Path, member: &str, unlock: &Unlock) -> Result<Vec<u8>> {
    let mut zip = zip_open(archive)?;
    let name = resolve_name(&mut zip, archive, member)?;
    let mut bytes = Vec::new();
    zip_entry(&mut zip, archive, &name, unlock)?
        .read_to_end(&mut bytes)
        .map_err(|e| extract::copy_error(archive, e))?;
    Ok(bytes)
}

/// One zip entry, opened for reading — decrypting if it is encrypted
/// and a password was given.
///
/// A zip encrypts each entry, not the archive — so a zip can hold a
/// mixture, and asking for a password to read the unencrypted half of
/// one would be asking for nothing. Only an entry that says it is
/// encrypted goes down the decrypting path.
///
/// A reader and not the bytes: an extraction streams it straight into
/// the destination file, so a member does not have to fit in memory to
/// be unpacked. It used to be read whole into a `Vec` first — a
/// four-gigabyte disk image in a zip was four gigabytes of this
/// process.
fn zip_entry<'z, R: Read + std::io::Seek + 'z>(
    zip: &'z mut zip::ZipArchive<R>,
    archive: &Path,
    name: &str,
    unlock: &Unlock,
) -> Result<Box<dyn Read + 'z>> {
    // Asked of the *raw* entry, which reads the header without trying
    // to decrypt anything — the question here is only "is this one
    // encrypted", and answering it must not need the password it is
    // being asked in order to request.
    let encrypted = zip
        .index_for_name(name)
        .and_then(|at| zip.by_index_raw(at).ok().map(|entry| entry.encrypted()))
        .unwrap_or(false);

    let entry = if encrypted {
        let Some(password) = unlock.expose() else {
            return Err(ArchiveError::PasswordRequired {
                path: archive.to_path_buf(),
            });
        };
        zip.by_name_decrypt(name, password.as_bytes()).map_err(|e| {
            // zip's own error for a bad password is an
            // `InvalidPassword`-shaped `InvalidArchive`; either way the
            // answer the caller needs is "ask again", not "this archive
            // is damaged", which would send someone looking for a
            // backup of a perfectly good file.
            match e {
                zip::result::ZipError::InvalidPassword => ArchiveError::PasswordRequired {
                    path: archive.to_path_buf(),
                },
                other => zip_error(archive, other),
            }
        })?
    } else {
        zip.by_name(name).map_err(|e| zip_error(archive, e))?
    };

    Ok(Box::new(Explained {
        inner: entry,
        archive: archive.to_path_buf(),
        // AES and ZipCrypto both authenticate on the way out, so a
        // wrong password that got past the header check fails partway
        // through the read, as a checksum or decryption error. Same
        // answer: ask again.
        failure: if encrypted { Failure::Password } else { Failure::Read },
    }))
}

/// What a read failure of a member means.
#[derive(Clone, Copy)]
enum Failure {
    /// The member is encrypted: a failure partway is a wrong password.
    Password,
    /// Anything else is the archive not reading.
    Read,
}

/// A member's reader whose failures say what they are.
///
/// A streamed member fails inside somebody else's `io::copy`, as a bare
/// `io::Error` that would be reported against the *destination* file —
/// "notes.txt couldn't be read" about a file this is writing — and,
/// worse, without the `NeedsPassword` reason that opens a prompt. So
/// the failure travels as the [`ArchiveError`] it is, inside the
/// `io::Error`, and [`extract::copy_error`] takes it back out.
struct Explained<R> {
    inner: R,
    archive: PathBuf,
    failure: Failure,
}

impl<R: Read> Read for Explained<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.inner.read(buf).map_err(|e| {
            let explained = match self.failure {
                Failure::Password => ArchiveError::PasswordRequired {
                    path: self.archive.clone(),
                },
                Failure::Read => ArchiveError::io(self.archive.clone(), e),
            };
            std::io::Error::other(explained)
        })
    }
}

/// The name a zip actually stores for the member this crate calls
/// `member`.
///
/// They differ whenever the archive spells a path in a way
/// [`crate::model::normalise`] tidied — `./docs/guide.txt`, or a
/// backslash from a Windows tool — and `by_name` matches the stored
/// string exactly. Without this, every member of such an archive lists
/// perfectly and none of them opens.
fn resolve_name<R: Read + std::io::Seek>(
    zip: &mut zip::ZipArchive<R>,
    archive: &Path,
    member: &str,
) -> Result<String> {
    let wanted = crate::model::normalise(member).ok_or_else(|| ArchiveError::MemberNotFound {
        archive: archive.to_path_buf(),
        member: member.to_string(),
    })?;
    if zip.index_for_name(&wanted).is_some() {
        return Ok(wanted);
    }
    for i in 0..zip.len() {
        if let Some(name) = zip.name_for_index(i) {
            if crate::model::normalise(name).as_deref() == Some(wanted.as_str()) {
                return Ok(name.to_string());
            }
        }
    }
    Err(ArchiveError::MemberNotFound {
        archive: archive.to_path_buf(),
        member: member.to_string(),
    })
}

fn zip_extract(
    archive: &Path,
    plan: &extract::Plan,
    request: &ExtractRequest,
    unlock: &Unlock,
    progress: &mut dyn Progress,
) -> Result<ExtractReport> {
    let mut zip = zip_open(archive)?;
    let mut run = extract::Run::new(plan, request)?;
    for item in &plan.items {
        // Resolved before the closure so a member that is simply not
        // there is a failed row rather than a borrow tangle.
        let name = resolve_name(&mut zip, archive, &item.member);
        let zip = &mut zip;
        run.item(item, progress, move || {
            // Moved out of the capture so the reader it returns can
            // borrow the archive for as long as the write takes, rather
            // than only for the closure's body.
            let zip = zip;
            zip_entry(zip, archive, &name?, unlock)
        })?;
    }
    Ok(run.finish())
}

// --- tar ------------------------------------------------------------

/// How many bytes a tar reader may take, between handing over one
/// member and the next, before the archive is called damaged.
///
/// The gap is headers: the next member's own, and any extension headers
/// in front of it — a GNU long name (`L`), a long link target (`K`), a
/// pax record (`x`). The `tar` crate reads each of those *whole* before
/// it hands the member over, and their size is whatever the archive
/// claims. A two-gigabyte "name" of one repeated letter is a few
/// megabytes as a `.tar.xz`, and listing it used to allocate the two
/// gigabytes. A real long name is a path — a few kilobytes at most — so
/// a megabyte is far beyond anything honest and still nothing to hold.
const BETWEEN_MEMBERS: u64 = 1024 * 1024;

/// Counts what a tar reader takes while it is looking for the next
/// member — see [`BETWEEN_MEMBERS`].
///
/// Metering only that gap, not the members' own data: a member is
/// streamed to wherever it is going and may be any size at all. The
/// gap is made to hold *only* headers by draining each member before
/// asking for the next (see [`walk_tar`]); otherwise the crate's own
/// skip over an unread member would count against the limit.
#[derive(Default)]
struct Meter {
    on: Cell<bool>,
    taken: Cell<u64>,
    tripped: Cell<bool>,
}

struct Metered<R> {
    inner: R,
    meter: Rc<Meter>,
}

impl<R: Read> Read for Metered<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let meter = &self.meter;
        if meter.on.get() && meter.taken.get() > BETWEEN_MEMBERS {
            meter.tripped.set(true);
            return Err(std::io::Error::other("a header claims more than a header can hold"));
        }
        let n = self.inner.read(buf)?;
        meter.taken.set(meter.taken.get() + n as u64);
        Ok(n)
    }
}

type TarStream = Metered<crate::stream::BoxRead<'static>>;

/// A tar reader over the right decompressor, and the meter on it.
fn tar_reader(archive: &Path, compression: Compression) -> Result<(tar::Archive<TarStream>, Rc<Meter>)> {
    let file = std::fs::File::open(archive).map_err(|e| ArchiveError::io(archive, e))?;
    let stream = crate::stream::decoder(compression, Box::new(BufReader::new(file)))
        .map_err(|e| ArchiveError::io(archive, e))?;
    let meter = Rc::new(Meter::default());
    let metered = Metered { inner: stream, meter: Rc::clone(&meter) };
    Ok((tar::Archive::new(metered), meter))
}

/// How a walk over a tar ended.
enum Walked {
    /// Every member was handed over.
    Whole,
    /// The tar stopped being readable partway. Everything handed over
    /// before this is real; what to do about the rest is the caller's.
    Cut(std::io::Error),
}

/// Walks a tar once, handing `each` every member with its path.
///
/// Every reader of a tar goes through here, so the limit on headers
/// (see [`BETWEEN_MEMBERS`]) cannot be forgotten by one of them. A
/// header past the limit is an error, not a short listing: it is not a
/// tar that happened to end early, and reporting it as one would show
/// someone a plausible half of a hostile file.
fn walk_tar(
    archive: &Path,
    compression: Compression,
    mut each: impl FnMut(&mut tar::Entry<'_, TarStream>, String) -> Result<()>,
) -> Result<Walked> {
    let (mut tar, meter) = tar_reader(archive, compression)?;
    let mut entries = tar.entries().map_err(|e| tar_damaged(archive, e))?;
    loop {
        meter.taken.set(0);
        meter.on.set(true);
        let next = entries.next();
        meter.on.set(false);

        let mut entry = match next {
            None => return Ok(Walked::Whole),
            Some(Ok(entry)) => entry,
            Some(Err(_)) if meter.tripped.get() => {
                return Err(ArchiveError::Damaged {
                    path: archive.to_path_buf(),
                    format: "tar",
                    detail: format!(
                        "a header claims to hold more than {} KiB, which no real header does",
                        BETWEEN_MEMBERS / 1024
                    ),
                });
            }
            Some(Err(e)) => return Ok(Walked::Cut(e)),
        };

        // A pax *global* header is settings for the archive as a whole,
        // not a member — `git archive` writes one first, and it used to
        // list as a file called `pax_global_header`. (The per-member
        // `x` header is consumed by the crate itself.)
        if !entry.header().entry_type().is_pax_global_extensions() {
            let path = entry.path().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
            each(&mut entry, path)?;
        }

        // Drained here, unmetered, so the next `next` reads only headers.
        // A failure is left for that `next` to report as the tar ending.
        let _ = std::io::copy(&mut entry, &mut std::io::sink());
    }
}

fn tar_index(archive: &Path, compression: Compression) -> Result<Index> {
    let mut members = Vec::new();
    let walked = walk_tar(archive, compression, |entry, path| {
        if path.is_empty() {
            return Ok(());
        }
        let header = entry.header();
        let kind = header.entry_type();
        members.push(Member {
            is_dir: kind.is_dir(),
            size: entry.size(),
            size_known: true,
            // Nothing per-member: a compressed tar is one stream, and
            // no individual member in it has a compressed size.
            compressed: None,
            modified: header.mtime().ok().map(|s| timestamp::from_unix(s as i64)),
            mode: header.mode().ok(),
            link_target: entry
                .link_name()
                .ok()
                .flatten()
                .map(|p| p.to_string_lossy().into_owned()),
            hard_link: kind.is_hard_link(),
            encrypted: false,
            path,
        });
        Ok(())
    })?;
    if let Walked::Cut(e) = walked {
        // A tar that stops being readable partway is not an empty tar:
        // everything walked so far is real, and reporting it beats
        // reporting nothing. The warning is what says the listing is
        // short.
        tracing::warn!(
            archive = %archive.display(),
            error = %e,
            "a tar stopped being readable partway through; listing what was read"
        );
    }
    Ok(Index::build(members))
}

/// A tar member that must deliver every byte its header promised.
///
/// A tar cut short ends a member's data with a plain end-of-file — no
/// checksum to fail, as a zip or a 7z member would — so a truncated
/// download unpacked its last member short and reported it extracted.
/// Reading past the promise is fine (a sparse member expands beyond
/// the bytes it stores); stopping short of it is the archive being
/// damaged, and says so.
struct Whole<R> {
    inner: R,
    promised: u64,
    archive: PathBuf,
}

impl<R: Read> Read for Whole<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = self.inner.read(buf)?;
        if n == 0 && self.promised > 0 && !buf.is_empty() {
            return Err(std::io::Error::other(ArchiveError::Damaged {
                path: self.archive.clone(),
                format: "tar",
                detail: "it ends partway through this member".to_string(),
            }));
        }
        self.promised = self.promised.saturating_sub(n as u64);
        Ok(n)
    }
}

fn tar_damaged(archive: &Path, error: std::io::Error) -> ArchiveError {
    ArchiveError::Damaged {
        path: archive.to_path_buf(),
        format: "tar",
        detail: error.to_string(),
    }
}

fn tar_read_member(archive: &Path, compression: Compression, member: &str) -> Result<Vec<u8>> {
    let wanted = crate::model::normalise(member).ok_or_else(|| ArchiveError::MemberNotFound {
        archive: archive.to_path_buf(),
        member: member.to_string(),
    })?;
    // Every entry, not the first match. A tar is last-one-wins (see
    // `model`'s module doc): `tar rf` appends a second `notes.txt`
    // after the first, and the one that ends up on disk when anything
    // unpacks the archive is the last. Stopping at the first would hand
    // back a version the archive supersedes — and it is the *listing*
    // that would look right, which is what makes that failure hard to
    // see.
    let mut found: Option<Vec<u8>> = None;
    let walked = walk_tar(archive, compression, |entry, path| {
        if crate::model::normalise(&path).as_deref() != Some(wanted.as_str()) {
            return Ok(());
        }
        let mut bytes = Vec::new();
        let promised = entry.size();
        Whole { inner: &mut *entry, promised, archive: archive.to_path_buf() }
            .read_to_end(&mut bytes)
            .map_err(|e| extract::copy_error(archive, e))?;
        found = Some(bytes);
        Ok(())
    })?;
    if let Walked::Cut(e) = walked {
        return Err(tar_damaged(archive, e));
    }

    found.ok_or_else(|| ArchiveError::MemberNotFound {
        archive: archive.to_path_buf(),
        member: member.to_string(),
    })
}

/// A single pass over the tar, writing every planned member as it goes.
///
/// See the module doc: pulling members out one at a time would re-read
/// and re-decompress the whole archive per member.
fn tar_extract(
    archive: &Path,
    compression: Compression,
    plan: &extract::Plan,
    request: &ExtractRequest,
    progress: &mut dyn Progress,
) -> Result<ExtractReport> {
    let planned: HashMap<&str, &PlanItem> =
        plan.items.iter().map(|item| (item.member.as_str(), item)).collect();

    let mut run = extract::Run::new(plan, request)?;
    let mut written: Vec<&str> = Vec::new();

    // A member the tar names twice arrives here twice, and both go to
    // `run.item` — which knows the second is superseding its own
    // earlier write rather than colliding with somebody's file. See
    // `extract::Run`.
    let walked = walk_tar(archive, compression, |entry, path| {
        let Some(normalised) = crate::model::normalise(&path) else {
            return Ok(());
        };
        let Some(item) = planned.get(normalised.as_str()).copied() else {
            return Ok(());
        };
        written.push(&item.member);
        let promised = entry.size();
        run.item(item, progress, || {
            Ok(Box::new(Whole {
                inner: &mut *entry,
                promised,
                archive: archive.to_path_buf(),
            }) as Box<dyn Read>)
        })
    })?;
    if let Walked::Cut(e) = walked {
        tracing::warn!(
            archive = %archive.display(),
            error = %e,
            "a tar stopped being readable partway through an extraction"
        );
    }

    // Directories the archive never listed have no entry to arrive on,
    // so nothing above created them — and an empty one asked for
    // explicitly would otherwise be silently missing from the result.
    for item in plan.items.iter().filter(|i| i.is_dir) {
        if !written.contains(&item.member.as_str()) {
            run.item(item, progress, || Ok(Box::new(std::io::empty()) as Box<dyn Read>))?;
        }
    }

    Ok(run.finish())
}

// --- 7z -------------------------------------------------------------

fn sevenz_open(
    archive: &Path,
    unlock: &Unlock,
) -> Result<sevenz_rust2::ArchiveReader<std::fs::File>> {
    // Unlike a zip, a 7z can encrypt its *header*, so the password is
    // needed to list the archive at all — which is why `index` takes an
    // `Unlock` and not only `read_member`.
    let password = match unlock.expose() {
        Some(password) => sevenz_rust2::Password::from(password),
        None => sevenz_rust2::Password::empty(),
    };
    sevenz_rust2::ArchiveReader::open(archive, password).map_err(|e| sevenz_error(archive, e))
}

fn sevenz_error(archive: &Path, error: sevenz_rust2::Error) -> ArchiveError {
    let message = error.to_string();
    let lowered = message.to_lowercase();
    if lowered.contains("password") {
        return ArchiveError::PasswordRequired {
            path: archive.to_path_buf(),
        };
    }
    if lowered.contains("unsupported") {
        return ArchiveError::Unsupported {
            path: archive.to_path_buf(),
            detail: message,
        };
    }
    ArchiveError::Damaged {
        path: archive.to_path_buf(),
        format: "7z",
        detail: message,
    }
}

/// Which of a 7z's entries are encrypted, by position in its file list.
///
/// A 7z encrypts *blocks*, not entries: an entry is encrypted when the
/// block holding its data has AES among its coders. Worked out here so
/// a 7z's listing carries the same per-member flag a zip's does — and
/// so an edit can refuse one (see [`crate::write::edit`]) without
/// first unpacking it in the clear.
fn sevenz_encrypted(archive: &sevenz_rust2::Archive) -> Vec<bool> {
    let aes = sevenz_rust2::EncoderMethod::ID_AES256_SHA256;
    (0..archive.files.len())
        .map(|i| {
            archive
                .stream_map
                .file_block_index
                .get(i)
                .copied()
                .flatten()
                .and_then(|block| archive.blocks.get(block))
                .is_some_and(|block| block.coders.iter().any(|c| c.encoder_method_id() == aes))
        })
        .collect()
}

fn sevenz_index(archive: &Path, unlock: &Unlock) -> Result<Index> {
    let reader = sevenz_open(archive, unlock)?;
    let encrypted = sevenz_encrypted(reader.archive());
    let members = reader
        .archive()
        .files
        .iter()
        .zip(encrypted)
        .map(|(entry, encrypted)| Member {
            path: entry.name().to_string(),
            is_dir: entry.is_directory(),
            size: entry.size(),
            size_known: true,
            compressed: Some(entry.compressed_size),
            modified: entry
                .has_last_modified_date
                .then(|| timestamp::from_filetime(filetime_ticks(&entry.last_modified_date)))
                .flatten(),
            // 7z records Windows attributes; a mode is only there if the
            // archive was written on a Unix and said so, and digging it
            // out of the attribute word is not worth guessing wrong.
            mode: None,
            link_target: None,
            hard_link: false,
            encrypted,
        })
        .collect();
    Ok(Index::build(members))
}

fn sevenz_read_member(archive: &Path, member: &str, unlock: &Unlock) -> Result<Vec<u8>> {
    let mut reader = sevenz_open(archive, unlock)?;
    let name = sevenz_stored_name(&reader, archive, member)?;
    reader.read_file(&name).map_err(|e| sevenz_error(archive, e))
}

/// 7z's own spelling of a member's name, for the reason
/// [`resolve_name`] exists for zip.
fn sevenz_stored_name(
    reader: &sevenz_rust2::ArchiveReader<std::fs::File>,
    archive: &Path,
    member: &str,
) -> Result<String> {
    let wanted = crate::model::normalise(member).ok_or_else(|| ArchiveError::MemberNotFound {
        archive: archive.to_path_buf(),
        member: member.to_string(),
    })?;
    reader
        .archive()
        .files
        .iter()
        .map(|entry| entry.name().to_string())
        .find(|name| crate::model::normalise(name).as_deref() == Some(wanted.as_str()))
        .ok_or_else(|| ArchiveError::MemberNotFound {
            archive: archive.to_path_buf(),
            member: member.to_string(),
        })
}

/// A single pass over a 7z, streaming every planned member as it goes.
///
/// The tar shape, for the same two reasons. A 7z's members share
/// compressed blocks — a *solid* archive is one block for everything —
/// so pulling them out one at a time by name decodes the block from its
/// start for every member, and the crate's `read_file` also hands each
/// one back as a whole `Vec`, so a member had to fit in memory to be
/// unpacked. Walking the blocks once, in order, streams each member
/// straight into its file.
///
/// Every entry the walk passes is drained, written or not: the members
/// of a block are one stream read in sequence, and an entry left unread
/// would be read as the start of the next.
fn sevenz_extract(
    archive: &Path,
    plan: &extract::Plan,
    request: &ExtractRequest,
    unlock: &Unlock,
    progress: &mut dyn Progress,
) -> Result<ExtractReport> {
    let mut reader = sevenz_open(archive, unlock)?;
    let encrypted: std::collections::HashSet<String> = reader
        .archive()
        .files
        .iter()
        .zip(sevenz_encrypted(reader.archive()))
        .filter(|(_, encrypted)| *encrypted)
        .filter_map(|(entry, _)| crate::model::normalise(entry.name()))
        .collect();
    let planned: HashMap<&str, &PlanItem> =
        plan.items.iter().map(|item| (item.member.as_str(), item)).collect();

    let mut run = extract::Run::new(plan, request)?;
    let mut written: Vec<&str> = Vec::new();
    // The walk's callback answers in the 7z crate's error type, which
    // cannot carry a cancel — so a stop is kept here and the walk told
    // only to end.
    let mut stopped: Option<ArchiveError> = None;

    reader
        .for_each_entries(|entry, data| {
            let name = crate::model::normalise(entry.name());
            if let Some(item) = name.as_deref().and_then(|n| planned.get(n).copied()) {
                written.push(&item.member);
                let failure = if encrypted.contains(&item.member) {
                    Failure::Password
                } else {
                    Failure::Read
                };
                let wrote = run.item(item, progress, || {
                    Ok(Box::new(Explained {
                        inner: &mut *data,
                        archive: archive.to_path_buf(),
                        failure,
                    }) as Box<dyn Read>)
                });
                if let Err(e) = wrote {
                    stopped = Some(e);
                    return Ok(false);
                }
            }
            // A failure here is the block not decoding, which the next
            // member's read will meet and report as its own row.
            let _ = std::io::copy(data, &mut std::io::sink());
            Ok(true)
        })
        .map_err(|e| sevenz_error(archive, e))?;
    if let Some(e) = stopped {
        return Err(e);
    }

    // A folder the archive names only by implication has no entry to
    // arrive on — see `tar_extract`.
    for item in plan.items.iter().filter(|i| i.is_dir) {
        if !written.contains(&item.member.as_str()) {
            run.item(item, progress, || Ok(Box::new(std::io::empty()) as Box<dyn Read>))?;
        }
    }
    Ok(run.finish())
}

// --- a single compressed file ---------------------------------------

/// The one member inside `dump.sql.gz`: the file's own name with the
/// compression extension taken off.
///
/// Falls back to the whole name when there is nothing to strip, so a
/// gzip stream named without an extension still has a member to show
/// rather than an empty one.
fn compressed_member_name(archive: &Path, compression: Compression) -> String {
    let name = archive
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "contents".to_string());
    let suffix = format!(".{}", compression.extension());
    match name.strip_suffix(&suffix) {
        Some(stripped) if !stripped.is_empty() => stripped.to_string(),
        _ => format!("{name}.contents"),
    }
}

/// `named` supplies the member's name and `archive` everything else —
/// see `StdArchives::index_pinned` for why they differ.
fn compressed_index(archive: &Path, named: &Path, compression: Compression) -> Result<Index> {
    let mut member = Member::file(compressed_member_name(named, compression), 0);
    // The size is the *decompressed* length, and the only way to learn
    // it is to decompress the whole stream — which listing a file must
    // not do. Unknown is the honest answer, and `size_known` is what
    // lets a caller render it as such instead of as zero bytes.
    member.size_known = false;
    member.modified = std::fs::metadata(archive).ok().and_then(|m| m.modified().ok());
    Ok(Index::build(vec![member]))
}

fn compressed_read(archive: &Path, compression: Compression) -> Result<Vec<u8>> {
    let file = std::fs::File::open(archive).map_err(|e| ArchiveError::io(archive, e))?;
    let mut stream = crate::stream::decoder(compression, Box::new(BufReader::new(file)))
        .map_err(|e| ArchiveError::io(archive, e))?;
    let mut bytes = Vec::new();
    stream.read_to_end(&mut bytes).map_err(|e| ArchiveError::Damaged {
        path: archive.to_path_buf(),
        format: "compressed stream",
        detail: e.to_string(),
    })?;
    Ok(bytes)
}

// --- the backend ----------------------------------------------------

impl StdArchives {
    /// [`ArchiveBackend::index_with`], reading an already-pinned
    /// `archive` and naming things after `named`.
    ///
    /// `named` is never opened. It exists for the one listing whose
    /// contents are named after the file rather than stored in it: a
    /// single compressed file's member is its own name with the suffix
    /// stripped, and through a pin that name is a descriptor number —
    /// `dump.sql.gz` would list, and then extract, as a file called `9`.
    fn index_pinned(archive: &Path, named: &Path, unlock: &Unlock) -> Result<Index> {
        match StdArchives::format_of(archive)? {
            // A zip's central directory is never encrypted, and a tar
            // has no encryption at all — only 7z can need a password to
            // say what is inside it.
            Format::Zip => zip_index(archive),
            Format::Tar(compression) => tar_index(archive, compression),
            Format::SevenZ => sevenz_index(archive, unlock),
            Format::Compressed(compression) => compressed_index(archive, named, compression),
        }
    }

    /// [`ArchiveBackend::extract_with`], against an already-pinned path.
    ///
    /// Lists through `index_pinned` rather than `index_with`: the plan
    /// names the files it writes after the listing, so the listing has
    /// to be named after the real archive — see `index_pinned`.
    fn extract_pinned(
        &self,
        archive: &Path,
        named: &Path,
        request: &ExtractRequest,
        unlock: &Unlock,
        progress: &mut dyn Progress,
    ) -> Result<ExtractReport> {
        let format = StdArchives::format_of(archive)?;
        let index = StdArchives::index_pinned(archive, named, unlock)?;
        let plan = extract::plan(&index, request, archive)?;

        match format {
            Format::Zip => zip_extract(archive, &plan, request, unlock, progress),
            Format::Tar(compression) => tar_extract(archive, compression, &plan, request, progress),
            Format::SevenZ => sevenz_extract(archive, &plan, request, unlock, progress),
            Format::Compressed(compression) => {
                let mut run = extract::Run::new(&plan, request)?;
                for item in &plan.items {
                    run.item(item, progress, || {
                        let file =
                            std::fs::File::open(archive).map_err(|e| ArchiveError::io(archive, e))?;
                        crate::stream::decoder(compression, Box::new(BufReader::new(file)))
                            .map_err(|e| ArchiveError::io(archive, e))
                            .map(|stream| Box::new(stream) as Box<dyn Read>)
                    })?;
                }
                Ok(run.finish())
            }
        }
    }
}


impl ArchiveBackend for StdArchives {
    // Each entry point runs inside a `Pinned`, with `archive` rebound to
    // the pinned path, so every open below it — the sniff, the listing,
    // each member — reads one version of the file. See `crate::pin` for
    // the race this closes. Rebinding the name rather than introducing a
    // second one is deliberate: nothing inside can reach the unpinned
    // path by accident. The one thing that needs the real name is
    // naming, which is why `index_pinned` takes it separately.

    fn index_with(&self, archive: &Path, unlock: &Unlock) -> Result<Index> {
        let named = archive;
        Pinned::open(archive)?.read(|archive| StdArchives::index_pinned(archive, named, unlock))
    }

    fn read_member_with(&self, archive: &Path, member: &str, unlock: &Unlock) -> Result<Vec<u8>> {
        Pinned::open(archive)?.read(|archive| match StdArchives::format_of(archive)? {
            Format::Zip => zip_read_member(archive, member, unlock),
            Format::Tar(compression) => tar_read_member(archive, compression, member),
            Format::SevenZ => sevenz_read_member(archive, member, unlock),
            Format::Compressed(compression) => compressed_read(archive, compression),
        })
    }

    fn extract_with(
        &self,
        archive: &Path,
        request: &ExtractRequest,
        unlock: &Unlock,
        progress: &mut dyn Progress,
    ) -> Result<ExtractReport> {
        let named = archive;
        Pinned::open(archive)?.extract(|archive| self.extract_pinned(archive, named, request, unlock, progress))
    }

    fn create(
        &self,
        dest: &Path,
        format: Format,
        sources: &[Source],
        progress: &mut dyn Progress,
    ) -> Result<()> {
        crate::write::create(dest, format, sources, progress)
    }

    fn edit_with(
        &self,
        archive: &Path,
        edits: &[Edit],
        unlock: &Unlock,
        progress: &mut dyn Progress,
    ) -> Result<()> {
        crate::write::edit(archive, edits, unlock, progress)
    }
}

/// A progress reporter's view of one member, for the writers in
/// [`crate::write`], which have the same cancel obligation as the
/// readers here.
pub(crate) fn tick(
    progress: &mut dyn Progress,
    member: &str,
    files_done: usize,
    files_total: usize,
) -> Result<()> {
    match progress.advance(Advance {
        member,
        files_done,
        files_total,
        bytes_done: 0,
        bytes_total: 0,
    }) {
        Flow::Continue => Ok(()),
        Flow::Cancel => Err(ArchiveError::Cancelled),
    }
}

/// FILETIME ticks out of whichever type this build of `sevenz-rust2`
/// uses for a date.
///
/// The crate's `nt-time` feature swaps the field's type, and this crate
/// does not enable it — so the value is 7z's own wrapper around the raw
/// tick count. Going through `Into<u64>` rather than naming the type
/// keeps this working either way.
fn filetime_ticks<T: Copy + Into<u64>>(value: &T) -> u64 {
    (*value).into()
}

/// Rewrites `archive` by way of a temporary file beside it — see
/// [`crate::write`]'s module doc on why every edit is a rewrite.
///
/// Beside it, not in `/tmp`: `std::fs::rename` is only atomic within one
/// filesystem, and a temporary directory is very often a different one.
/// Renaming across filesystems is a copy, which is precisely the
/// half-written-file case the whole arrangement exists to avoid.
pub(crate) fn rewrite_in_place(
    archive: &Path,
    write: impl FnOnce(&Path) -> Result<()>,
) -> Result<()> {
    // Through a symlink to the archive it names. The rename below
    // replaces whatever is *at* the path, and for a link that is the
    // link: an edit of `latest.zip -> builds/0412.zip` turned the link
    // into a regular file and left the real archive unedited, with
    // nothing to say the two had come apart.
    let resolved;
    let archive = match std::fs::symlink_metadata(archive) {
        Ok(meta) if meta.file_type().is_symlink() => match std::fs::canonicalize(archive) {
            Ok(real) => {
                resolved = real;
                resolved.as_path()
            }
            // A dangling link has nothing to resolve to; a new archive
            // written there replaces the link, as it always did.
            Err(_) => archive,
        },
        _ => archive,
    };
    let parent = archive.parent().unwrap_or(Path::new("."));
    let name = archive.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();

    // A *unique* temporary, not `.<name>.hyprforge-new`. A fixed name is
    // shared by any two rewrites of the same archive happening at once,
    // and they then corrupt each other in a way that reports itself
    // backwards: measured with two concurrent edits of one zip, the edit
    // that returned `Ok` had its change lost and the edit that returned
    // `Err` had its change applied, because the second writer truncated
    // the file the first was about to rename into place.
    //
    // This does not make concurrent edits *correct*. Each reads the whole
    // archive and writes a whole new one, so the later rename still wins
    // and the earlier edit is still lost; serialising that is the
    // caller's job. What it guarantees is the part a library owes on its
    // own — whatever ends up at `archive` is a complete archive that one
    // writer wrote, and no rewrite can leave another's half-written bytes
    // there.
    let holder = tempfile::Builder::new()
        .prefix(&format!(".{name}."))
        .suffix(".hyprforge-new")
        .tempfile_in(parent)
        .map_err(|e| ArchiveError::io(archive, e))?;
    let temporary = holder.path().to_path_buf();

    // Dropping `holder` removes the file, so a failure or a cancel leaves
    // nothing lying beside someone's archive — including on a panic,
    // which the hand-rolled remove-on-error could not promise.
    write(&temporary)?;

    // The archive keeps its own permissions. `tempfile` creates the
    // temporary owner-only (0600), and the rename puts *that* file at
    // the archive's name — so an in-place edit quietly made a shared,
    // 0644 archive private, which nothing reported and the next person
    // to read it found out. Best effort, like the copy's own permission
    // preservation in hyprforge-fileops: a filesystem that cannot hold
    // the bits (FAT) is not a reason to lose the edit.
    if let Ok(meta) = std::fs::metadata(archive) {
        use std::os::unix::fs::PermissionsExt;
        let mode = meta.permissions().mode() & 0o7777;
        if let Err(e) = std::fs::set_permissions(&temporary, std::fs::Permissions::from_mode(mode)) {
            tracing::warn!(error = %e, "could not keep an archive's permissions across its rewrite");
        }
    }

    // On the disk before the rename, not merely written. A rename is
    // atomic in the namespace but says nothing about the data: after a
    // power cut the new name can survive and point at blocks that never
    // reached the disk — a zero-length or garbage file where somebody's
    // complete archive used to be, which is exactly what this function
    // exists to prevent. The writers wrote through their own handles to
    // this same inode, so syncing this one covers what they wrote.
    holder
        .as_file()
        .sync_all()
        .map_err(|e| ArchiveError::io(archive, e))?;

    holder
        .persist(archive)
        .map_err(|e| ArchiveError::io(archive, e.error))?;

    // And the rename itself, which lives in the directory. Best effort:
    // the edit has happened, and a filesystem that cannot sync a
    // directory (some network mounts) is no reason to report it failed.
    if let Err(e) = std::fs::File::open(parent).and_then(|dir| dir.sync_all()) {
        tracing::warn!(error = %e, "could not sync the directory an archive was rewritten in");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::{Collision, NoProgress};

    /// A zip at `dest` holding one file per `(name, contents)`.
    fn zip_of(dest: &Path, files: &[(&str, &str)]) {
        let staging = tempfile::tempdir().unwrap();
        let sources: Vec<Source> = files
            .iter()
            .map(|(name, contents)| {
                let path = staging.path().join(name);
                std::fs::write(&path, contents).unwrap();
                Source {
                    path,
                    as_member: name.to_string(),
                }
            })
            .collect();
        StdArchives.create(dest, Format::Zip, &sources, &mut NoProgress).unwrap();
    }

    /// The extraction half of the race `crate::pin` closes, put exactly
    /// where it has to land: another writer renames a different archive
    /// over this one after the extraction has pinned it and before it
    /// has read anything. What comes out is the archive as it was —
    /// all of it, and none of the newcomer — rather than a listing of
    /// one archive unpacked from another, which is what reading by
    /// name produced: members the plan expected and the file no longer
    /// held, each a failure row, beside a destination half-written.
    #[test]
    fn an_archive_replaced_mid_extraction_is_extracted_as_it_was() {
        let dir = tempfile::tempdir().unwrap();
        let archive = dir.path().join("sample.zip");
        zip_of(&archive, &[("a.txt", "a"), ("b.txt", "b")]);
        let pin = Pinned::open(&archive).unwrap();

        let newcomer = dir.path().join("newcomer.zip");
        zip_of(&newcomer, &[("z.txt", "z")]);
        std::fs::rename(&newcomer, &archive).unwrap();

        let out = dir.path().join("out");
        let request = ExtractRequest {
            members: Vec::new(),
            dest: out.clone(),
            strip_prefix: None,
            collision: Collision::Overwrite,
        };
        let report = pin
            .extract(|source| {
                StdArchives.extract_pinned(source, &archive, &request, &Unlock::none(), &mut NoProgress)
            })
            .unwrap();

        assert!(report.failed.is_empty(), "{:?}", report.failed);
        let mut written: Vec<String> = std::fs::read_dir(&out)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        written.sort();
        assert_eq!(written, ["a.txt", "b.txt"]);
    }
}
