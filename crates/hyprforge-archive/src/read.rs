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
use crate::timestamp;
use crate::unlock::Unlock;
use std::collections::HashMap;
use std::io::{BufReader, Read};
use std::path::Path;

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
    for i in links {
        let name = members[i].path.clone();
        if let Ok(mut entry) = zip.by_name(&name) {
            let mut target = String::new();
            if entry.read_to_string(&mut target).is_ok() {
                members[i].link_target = Some(target);
            }
        }
    }

    Ok(Index::build(members))
}

/// Whether a Unix mode says "this is a symlink" — `S_IFLNK`, the file
/// type bits above the permissions.
fn is_symlink_mode(mode: u32) -> bool {
    mode & 0o170000 == 0o120000
}

fn zip_read_member(archive: &Path, member: &str, unlock: &Unlock) -> Result<Vec<u8>> {
    let mut zip = zip_open(archive)?;
    let name = resolve_name(&mut zip, archive, member)?;
    let mut bytes = Vec::new();
    zip_entry(&mut zip, archive, &name, unlock, &mut bytes)?;
    Ok(bytes)
}

/// Reads one zip entry into `into`, decrypting if it is encrypted and a
/// password was given.
///
/// A zip encrypts each entry, not the archive — so a zip can hold a
/// mixture, and asking for a password to read the unencrypted half of
/// one would be asking for nothing. Only an entry that says it is
/// encrypted goes down the decrypting path.
fn zip_entry<R: Read + std::io::Seek>(
    zip: &mut zip::ZipArchive<R>,
    archive: &Path,
    name: &str,
    unlock: &Unlock,
    into: &mut Vec<u8>,
) -> Result<()> {
    // Asked of the *raw* entry, which reads the header without trying
    // to decrypt anything — the question here is only "is this one
    // encrypted", and answering it must not need the password it is
    // being asked in order to request.
    let encrypted = zip
        .index_for_name(name)
        .and_then(|at| zip.by_index_raw(at).ok().map(|entry| entry.encrypted()))
        .unwrap_or(false);

    let mut entry = if encrypted {
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

    entry.read_to_end(into).map_err(|e| {
        // AES and ZipCrypto both authenticate on the way out, so a wrong
        // password that got past the header check fails *here*, as a
        // checksum or decryption error partway through the read. Same
        // answer: ask again.
        if encrypted {
            ArchiveError::PasswordRequired {
                path: archive.to_path_buf(),
            }
        } else {
            ArchiveError::io(archive, e)
        }
    })?;
    Ok(())
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
        run.item(item, progress, || {
            let mut bytes = Vec::new();
            zip_entry(&mut zip, archive, &name?, unlock, &mut bytes)?;
            Ok(Box::new(std::io::Cursor::new(bytes)) as Box<dyn Read>)
        })?;
    }
    Ok(run.finish())
}

// --- tar ------------------------------------------------------------

/// A tar reader over the right decompressor.
fn tar_reader(archive: &Path, compression: Compression) -> Result<tar::Archive<crate::stream::BoxRead<'static>>> {
    let file = std::fs::File::open(archive).map_err(|e| ArchiveError::io(archive, e))?;
    let stream = crate::stream::decoder(compression, Box::new(BufReader::new(file)))
        .map_err(|e| ArchiveError::io(archive, e))?;
    Ok(tar::Archive::new(stream))
}

fn tar_index(archive: &Path, compression: Compression) -> Result<Index> {
    let mut tar = tar_reader(archive, compression)?;
    let entries = tar
        .entries()
        .map_err(|e| tar_damaged(archive, e))?;

    let mut members = Vec::new();
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(e) => {
                // A tar that stops being readable partway is not an
                // empty tar: everything walked so far is real, and
                // reporting it beats reporting nothing. The warning is
                // what says the listing is short.
                tracing::warn!(
                    archive = %archive.display(),
                    error = %e,
                    "a tar stopped being readable partway through; listing what was read"
                );
                break;
            }
        };
        let header = entry.header();
        let path = entry.path().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
        if path.is_empty() {
            continue;
        }
        members.push(Member {
            is_dir: header.entry_type().is_dir(),
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
            encrypted: false,
            path,
        });
    }
    Ok(Index::build(members))
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
    let mut tar = tar_reader(archive, compression)?;

    // Every entry, not the first match. A tar is last-one-wins (see
    // `model`'s module doc): `tar rf` appends a second `notes.txt`
    // after the first, and the one that ends up on disk when anything
    // unpacks the archive is the last. Stopping at the first would hand
    // back a version the archive supersedes — and it is the *listing*
    // that would look right, which is what makes that failure hard to
    // see.
    let mut found: Option<Vec<u8>> = None;
    for entry in tar.entries().map_err(|e| tar_damaged(archive, e))? {
        let mut entry = entry.map_err(|e| tar_damaged(archive, e))?;
        let path = entry.path().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
        if crate::model::normalise(&path).as_deref() != Some(wanted.as_str()) {
            continue;
        }
        let mut bytes = Vec::new();
        entry.read_to_end(&mut bytes).map_err(|e| ArchiveError::io(archive, e))?;
        found = Some(bytes);
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
    let mut tar = tar_reader(archive, compression)?;
    let mut written: Vec<&str> = Vec::new();

    for entry in tar.entries().map_err(|e| tar_damaged(archive, e))? {
        let mut entry = match entry {
            Ok(entry) => entry,
            Err(e) => {
                tracing::warn!(
                    archive = %archive.display(),
                    error = %e,
                    "a tar stopped being readable partway through an extraction"
                );
                break;
            }
        };
        let path = entry.path().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
        let Some(normalised) = crate::model::normalise(&path) else {
            continue;
        };
        let Some(item) = planned.get(normalised.as_str()).copied() else {
            continue;
        };
        written.push(&item.member);
        run.item(item, progress, || Ok(Box::new(&mut entry) as Box<dyn Read>))?;
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

fn sevenz_index(archive: &Path, unlock: &Unlock) -> Result<Index> {
    let reader = sevenz_open(archive, unlock)?;
    let members = reader
        .archive()
        .files
        .iter()
        .map(|entry| Member {
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
            encrypted: false,
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

fn compressed_index(archive: &Path, compression: Compression) -> Result<Index> {
    let mut member = Member::file(compressed_member_name(archive, compression), 0);
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

impl ArchiveBackend for StdArchives {
    fn index_with(&self, archive: &Path, unlock: &Unlock) -> Result<Index> {
        match StdArchives::format_of(archive)? {
            // A zip's central directory is never encrypted, and a tar
            // has no encryption at all — only 7z can need a password to
            // say what is inside it.
            Format::Zip => zip_index(archive),
            Format::Tar(compression) => tar_index(archive, compression),
            Format::SevenZ => sevenz_index(archive, unlock),
            Format::Compressed(compression) => compressed_index(archive, compression),
        }
    }

    fn read_member_with(&self, archive: &Path, member: &str, unlock: &Unlock) -> Result<Vec<u8>> {
        match StdArchives::format_of(archive)? {
            Format::Zip => zip_read_member(archive, member, unlock),
            Format::Tar(compression) => tar_read_member(archive, compression, member),
            Format::SevenZ => sevenz_read_member(archive, member, unlock),
            Format::Compressed(compression) => compressed_read(archive, compression),
        }
    }

    fn extract_with(
        &self,
        archive: &Path,
        request: &ExtractRequest,
        unlock: &Unlock,
        progress: &mut dyn Progress,
    ) -> Result<ExtractReport> {
        let format = StdArchives::format_of(archive)?;
        let index = self.index_with(archive, unlock)?;
        let plan = extract::plan(&index, request, archive)?;

        match format {
            Format::Zip => zip_extract(archive, &plan, request, unlock, progress),
            Format::Tar(compression) => tar_extract(archive, compression, &plan, request, progress),
            Format::SevenZ => {
                // 7z members share compressed blocks, so pulling them one
                // at a time can decode a block per member. Acceptable
                // here and not for tar, because the reader keeps the file
                // open and seeks — it is not re-reading the archive from
                // byte zero each time, which is exactly what a tar would.
                let mut reader = sevenz_open(archive, unlock)?;
                let mut run = extract::Run::new(&plan, request)?;
                for item in &plan.items {
                    let name = sevenz_stored_name(&reader, archive, &item.member);
                    run.item(item, progress, || {
                        let bytes = reader
                            .read_file(&name?)
                            .map_err(|e| sevenz_error(archive, e))?;
                        Ok(Box::new(std::io::Cursor::new(bytes)) as Box<dyn Read>)
                    })?;
                }
                Ok(run.finish())
            }
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

    holder
        .persist(archive)
        .map_err(|e| ArchiveError::io(archive, e.error))?;
    Ok(())
}
