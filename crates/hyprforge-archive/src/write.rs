//! Creating an archive, and changing one that already exists.
//!
//! # Every edit is a rewrite
//!
//! None of these three formats can have a member removed from the middle
//! of them in place. A tar is a stream; a 7z's members share compressed
//! blocks, so deleting one means re-encoding the block the others are
//! in; a zip *could* in principle be spliced, and doing it by hand for
//! one format out of three would mean two code paths where one will do.
//!
//! So an edit reads the archive and writes a new one beside it, and only
//! then replaces the original — `std::fs::rename` over the same
//! filesystem, which either happens or does not. The failure this
//! prevents is the one that matters: a rewrite interrupted halfway
//! (cancelled, out of disk, the machine losing power) must never leave a
//! truncated file where someone's archive used to be. Until the rename,
//! the original is untouched; after it, the new one is complete.
//!
//! That is also why [`crate::backend::Edit`]s are applied as a batch.
//! Three deletions applied one at a time are three full rewrites of the
//! same file.

use crate::backend::{ArchiveBackend, Collision, Edit, ExtractRequest, Progress, Source};
use crate::error::{ArchiveError, Result};
use crate::format::{Compression, Format};
use crate::model::{normalise, Member};
use std::io::Write;
use std::path::{Path, PathBuf};

/// Applies one edit to a list of `(member, contents)` pairs.
///
/// The pure half of editing: it is the same decision whether the
/// contents beside each member are bytes held in memory (the mock) or a
/// handle to be copied out of the old archive into the new one (the real
/// writers), so the *rules* — what a rename does to the things inside a
/// renamed folder, what removing a directory takes with it — live here
/// once and are tested without writing an archive at all.
pub fn apply_to_list<T>(members: &mut Vec<(Member, T)>, edit: &Edit) {
    match edit {
        Edit::Remove(path) => {
            let Some(path) = normalise(path) else { return };
            let under = format!("{path}/");
            // A directory takes its contents with it. Leaving them would
            // produce an archive whose members have no directory above
            // them — legal in every one of these formats, and displayed
            // by `Index` as a synthesised folder that reappears exactly
            // where the one just deleted was.
            members.retain(|(m, _)| m.path != path && !m.path.starts_with(&under));
        }
        Edit::Rename { from, to } => {
            let (Some(from), Some(to)) = (normalise(from), normalise(to)) else {
                return;
            };
            let under = format!("{from}/");
            for (member, _) in members.iter_mut() {
                if member.path == from {
                    member.path = to.clone();
                } else if let Some(rest) = member.path.strip_prefix(&under) {
                    // Renaming a folder renames the path of everything
                    // inside it. Those members are how the folder exists
                    // at all in a format that may never have listed it.
                    member.path = format!("{to}/{rest}");
                }
            }
        }
        Edit::Add { .. } => {
            // Added members come from the filesystem, so they are
            // appended by the caller that can read them — `apply_to_list`
            // has no bytes to attach. Removing whatever they land on
            // top of is still this function's business, so the two
            // cannot get out of step.
        }
    }
}

/// The member paths an [`Edit::Add`] would replace, so a caller can drop
/// them before appending.
pub fn replaced_by(edit: &Edit) -> Option<String> {
    match edit {
        Edit::Add { as_member, .. } => normalise(as_member),
        _ => None,
    }
}


// --- creating -------------------------------------------------------

/// Writes a brand new archive at `dest` holding `sources`.
pub fn create(
    dest: &Path,
    format: Format,
    sources: &[Source],
    progress: &mut dyn Progress,
) -> Result<()> {
    if !format.writable() {
        return Err(ArchiveError::Unsupported {
            path: dest.to_path_buf(),
            detail: format!("{} archives can only be read", format.extension()),
        });
    }

    // Expanded first so the progress total is a number rather than a
    // guess that grows as directories are walked — and so a source that
    // cannot be read fails before anything has been written.
    let members = expand(sources)?;

    // Through the same temporary-then-rename dance an edit uses: a
    // cancelled or failed *creation* must not leave a half-written
    // archive sitting where a complete one appears to be, and the
    // difference between the two is invisible in a listing.
    crate::read::rewrite_in_place(dest, |temporary| match format {
        Format::Zip => write_zip(temporary, &members, progress),
        Format::Tar(compression) => write_tar(temporary, compression, &members, progress),
        Format::SevenZ => write_sevenz(temporary, &members, progress),
        Format::Compressed(_) => unreachable!("refused by `writable` above"),
    })
}

/// One thing to write into an archive: where its bytes are, and what it
/// is called inside.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    pub member: String,
    pub is_dir: bool,
    pub source: Option<PathBuf>,
}

/// Walks every directory in `sources`, so the caller can hand over "this
/// folder" and get everything under it.
///
/// Directories are emitted as members of their own as well as being
/// walked. An archive whose folders exist only by implication reads back
/// fine, but an *empty* folder has nothing to imply it and would
/// silently disappear from the round trip.
fn expand(sources: &[Source]) -> Result<Vec<Item>> {
    let mut items = Vec::new();
    for source in sources {
        let Some(member) = normalise(&source.as_member) else {
            continue;
        };
        let meta = std::fs::symlink_metadata(&source.path)
            .map_err(|e| ArchiveError::io(source.path.clone(), e))?;
        if meta.is_dir() {
            items.push(Item { member: member.clone(), is_dir: true, source: None });
            walk(&source.path, &member, &mut items)?;
        } else {
            items.push(Item {
                member,
                is_dir: false,
                source: Some(source.path.clone()),
            });
        }
    }
    // Parents before children, the same ordering `extract::plan` uses
    // and for the same reason — a reader that creates directories as it
    // meets them should never meet a file first.
    items.sort_by(|a, b| a.member.cmp(&b.member));
    items.dedup_by(|a, b| a.member == b.member);
    Ok(items)
}

fn walk(dir: &Path, prefix: &str, items: &mut Vec<Item>) -> Result<()> {
    let listing = std::fs::read_dir(dir).map_err(|e| ArchiveError::io(dir, e))?;
    for entry in listing {
        // One unreadable entry does not sink the archive — the rule
        // `StdBackend::read_dir` follows, and the consequence here is
        // that a compressed folder can be short. The warning is what
        // says so.
        let Ok(entry) = entry else {
            tracing::warn!(dir = %dir.display(), "skipping a directory entry that could not be read");
            continue;
        };
        let name = entry.file_name().to_string_lossy().into_owned();
        let member = format!("{prefix}/{name}");
        let Ok(meta) = entry.metadata() else {
            tracing::warn!(path = %entry.path().display(), "skipping an entry whose metadata could not be read");
            continue;
        };
        if meta.is_dir() {
            items.push(Item { member: member.clone(), is_dir: true, source: None });
            walk(&entry.path(), &member, items)?;
        } else {
            items.push(Item { member, is_dir: false, source: Some(entry.path()) });
        }
    }
    Ok(())
}

fn write_zip(dest: &Path, items: &[Item], progress: &mut dyn Progress) -> Result<()> {
    let file = std::fs::File::create(dest).map_err(|e| ArchiveError::io(dest, e))?;
    let mut zip = zip::ZipWriter::new(std::io::BufWriter::new(file));
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);

    let total = items.iter().filter(|i| !i.is_dir).count();
    let mut done = 0;
    for item in items {
        crate::read::tick(progress, &item.member, done, total)?;
        if item.is_dir {
            zip.add_directory(&item.member, options)
                .map_err(|e| ArchiveError::Damaged {
                    path: dest.to_path_buf(),
                    format: "zip",
                    detail: e.to_string(),
                })?;
            continue;
        }
        let Some(source) = &item.source else { continue };
        zip.start_file(&item.member, options).map_err(|e| ArchiveError::Damaged {
            path: dest.to_path_buf(),
            format: "zip",
            detail: e.to_string(),
        })?;
        let mut input = std::fs::File::open(source).map_err(|e| ArchiveError::io(source.clone(), e))?;
        std::io::copy(&mut input, &mut zip).map_err(|e| ArchiveError::io(dest, e))?;
        done += 1;
    }
    zip.finish().map_err(|e| ArchiveError::Damaged {
        path: dest.to_path_buf(),
        format: "zip",
        detail: e.to_string(),
    })?;
    Ok(())
}

fn write_tar(
    dest: &Path,
    compression: Compression,
    items: &[Item],
    progress: &mut dyn Progress,
) -> Result<()> {
    let file = std::fs::File::create(dest).map_err(|e| ArchiveError::io(dest, e))?;
    let stream = crate::stream::encoder(compression, Box::new(std::io::BufWriter::new(file)))
        .map_err(|e| ArchiveError::io(dest, e))?;
    let mut tar = tar::Builder::new(stream);

    let total = items.iter().filter(|i| !i.is_dir).count();
    let mut done = 0;
    for item in items {
        crate::read::tick(progress, &item.member, done, total)?;
        if item.is_dir {
            let mut header = tar::Header::new_gnu();
            header.set_entry_type(tar::EntryType::Directory);
            header.set_size(0);
            header.set_mode(0o755);
            header.set_mtime(now_seconds());
            header.set_cksum();
            // The trailing slash is how a tar says "directory" to every
            // reader that does not look at the entry type.
            tar.append_data(&mut header, format!("{}/", item.member), std::io::empty())
                .map_err(|e| ArchiveError::io(dest, e))?;
            continue;
        }
        let Some(source) = &item.source else { continue };
        let mut input = std::fs::File::open(source).map_err(|e| ArchiveError::io(source.clone(), e))?;
        tar.append_file(&item.member, &mut input)
            .map_err(|e| ArchiveError::io(dest, e))?;
        done += 1;
    }

    // `into_inner` and not a bare drop: finishing the tar writes its
    // end-of-archive blocks, and the *compressor* under it then has its
    // own trailer to flush. A drop would do both in an order nobody
    // stated, and swallow any error from either — which is a truncated
    // archive reported as a successful one.
    let stream = tar.into_inner().map_err(|e| ArchiveError::io(dest, e))?;
    finish_stream(stream, dest)
}

/// Drops the encoder, surfacing any failure in its trailer.
fn finish_stream(mut stream: crate::stream::BoxWrite<'_>, dest: &Path) -> Result<()> {
    stream.flush().map_err(|e| ArchiveError::io(dest, e))?;
    drop(stream);
    Ok(())
}

fn write_sevenz(dest: &Path, items: &[Item], progress: &mut dyn Progress) -> Result<()> {
    let mut writer = sevenz_rust2::ArchiveWriter::create(dest)
        .map_err(|e| sevenz_write_error(dest, e))?;

    let total = items.iter().filter(|i| !i.is_dir).count();
    let mut done = 0;
    for item in items {
        crate::read::tick(progress, &item.member, done, total)?;
        if item.is_dir {
            let entry = sevenz_rust2::ArchiveEntry::new_directory(&item.member);
            writer
                .push_archive_entry::<std::fs::File>(entry, None)
                .map_err(|e| sevenz_write_error(dest, e))?;
            continue;
        }
        let Some(source) = &item.source else { continue };
        let input = std::fs::File::open(source).map_err(|e| ArchiveError::io(source.clone(), e))?;
        let entry = sevenz_rust2::ArchiveEntry::new_file(&item.member);
        writer
            .push_archive_entry(entry, Some(input))
            .map_err(|e| sevenz_write_error(dest, e))?;
        done += 1;
    }
    // `finish` answers with a plain `io::Error` where every other call
    // on the writer answers with the crate's own — the trailer is
    // written through the file handle directly.
    writer.finish().map_err(|e| ArchiveError::io(dest, e))?;
    Ok(())
}

fn sevenz_write_error(dest: &Path, error: sevenz_rust2::Error) -> ArchiveError {
    ArchiveError::Damaged {
        path: dest.to_path_buf(),
        format: "7z",
        detail: error.to_string(),
    }
}

fn now_seconds() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

// --- editing --------------------------------------------------------

/// Applies `edits` to an existing archive, in place.
///
/// Reads the whole thing out to a scratch directory, applies the edits
/// to the listing, and writes a new archive from the result — then
/// renames it over the original. See the module doc: every one of these
/// formats rewrites to change, and going through the filesystem keeps
/// one code path for three formats and never holds an archive's
/// contents in memory.
pub fn edit(archive: &Path, edits: &[Edit], progress: &mut dyn Progress) -> Result<()> {
    let format = crate::format::sniff(archive)
        .map_err(|e| ArchiveError::io(archive, e))?
        .ok_or_else(|| ArchiveError::NotAnArchive { path: archive.to_path_buf() })?;
    if !format.writable() {
        return Err(ArchiveError::Unsupported {
            path: archive.to_path_buf(),
            detail: format!(
                "{} files hold a single stream and can't be edited",
                format.extension()
            ),
        });
    }

    let backend = crate::read::StdArchives;
    let index = backend.index(archive)?;

    // Unpacked beside the archive rather than in `/tmp`: a home
    // directory and a temporary filesystem are usually different mounts,
    // and one of them is usually much smaller and in memory. Unpacking a
    // four-gigabyte archive into a tmpfs to change one name in it is how
    // a machine runs out of memory doing what looked like a rename.
    let scratch = tempfile::Builder::new()
        .prefix(".hyprforge-edit-")
        .tempdir_in(archive.parent().unwrap_or(Path::new(".")))
        .map_err(|e| ArchiveError::io(archive, e))?;

    let request = ExtractRequest {
        members: Vec::new(),
        dest: scratch.path().to_path_buf(),
        strip_prefix: None,
        collision: Collision::Overwrite,
    };
    backend.extract(archive, &request, progress)?;

    // The second element is where the member's bytes are *now*: the
    // path it was unpacked under, which is the name it had in the
    // archive. That is the whole reason `apply_to_list` is generic —
    // a rename changes `Member::path` and leaves this alone, so a
    // renamed member still knows which unpacked file it is. Deriving it
    // afterwards by matching names cannot work: after the rename there
    // is nothing left to match on, and the member would silently
    // disappear from the rewritten archive.
    let mut listing: Vec<(Member, String)> = index
        .members()
        .iter()
        .map(|m| (m.clone(), m.path.clone()))
        .collect();

    let mut added: Vec<Source> = Vec::new();
    for edit in edits {
        // An addition replaces whatever it lands on, rather than
        // producing an archive with the same name in it twice — which
        // both zip and tar permit and which reads back, by the
        // last-one-wins rule, as the new file having silently won
        // anyway. Saying so here makes it true in the file as well as
        // in the listing.
        if let Some(replaced) = replaced_by(edit) {
            apply_to_list(&mut listing, &Edit::Remove(replaced));
        }
        apply_to_list(&mut listing, edit);
        if let Edit::Add { source, as_member } = edit {
            added.push(Source {
                path: source.clone(),
                as_member: as_member.clone(),
            });
        }
    }

    let mut items: Vec<Item> = listing
        .into_iter()
        .map(|(member, unpacked_as)| Item {
            member: member.path,
            is_dir: member.is_dir,
            source: (!member.is_dir).then(|| scratch.path().join(unpacked_as)),
        })
        .collect();
    // Added members come off the real filesystem and may be whole
    // folders, so they go through the same walk a creation uses.
    items.append(&mut expand(&added)?);
    items.sort_by(|a, b| a.member.cmp(&b.member));
    items.dedup_by(|a, b| a.member == b.member);

    crate::read::rewrite_in_place(archive, |temporary| match format {
        Format::Zip => write_zip(temporary, &items, progress),
        Format::Tar(compression) => write_tar(temporary, compression, &items, progress),
        Format::SevenZ => write_sevenz(temporary, &items, progress),
        Format::Compressed(_) => unreachable!("refused by `writable` above"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn listing() -> Vec<(Member, ())> {
        vec![
            (Member::dir("docs"), ()),
            (Member::file("docs/guide.txt", 1), ()),
            (Member::file("docs/deep/more.txt", 2), ()),
            (Member::file("readme.md", 3), ()),
        ]
    }

    fn paths(members: &[(Member, ())]) -> Vec<&str> {
        members.iter().map(|(m, _)| m.path.as_str()).collect()
    }

    #[test]
    fn removing_a_folder_removes_what_was_inside_it() {
        let mut members = listing();
        apply_to_list(&mut members, &Edit::Remove("docs".to_string()));
        assert_eq!(paths(&members), ["readme.md"]);
    }

    /// The trap: an archive's directories are often implied by the paths
    /// under them, so a deletion that removed only the directory entry
    /// would leave the folder plainly visible in the listing, now
    /// holding the files it was supposed to take with it.
    #[test]
    fn removing_a_folder_does_not_leave_it_standing_as_a_synthesised_one() {
        let mut members = listing();
        apply_to_list(&mut members, &Edit::Remove("docs".to_string()));
        let index = crate::model::Index::build(members.into_iter().map(|(m, _)| m).collect());
        assert!(index.get("docs").is_none());
    }

    #[test]
    fn removing_one_file_leaves_its_neighbours_and_its_folder() {
        let mut members = listing();
        apply_to_list(&mut members, &Edit::Remove("docs/guide.txt".to_string()));
        assert_eq!(paths(&members), ["docs", "docs/deep/more.txt", "readme.md"]);
    }

    #[test]
    fn a_name_that_merely_starts_the_same_is_not_swept_up() {
        let mut members = vec![
            (Member::dir("docs"), ()),
            (Member::file("docs-backup/old.txt", 1), ()),
        ];
        apply_to_list(&mut members, &Edit::Remove("docs".to_string()));
        assert_eq!(paths(&members), ["docs-backup/old.txt"]);
    }

    #[test]
    fn renaming_a_folder_renames_the_paths_of_everything_in_it() {
        let mut members = listing();
        apply_to_list(
            &mut members,
            &Edit::Rename { from: "docs".to_string(), to: "manual".to_string() },
        );
        assert_eq!(
            paths(&members),
            ["manual", "manual/guide.txt", "manual/deep/more.txt", "readme.md"]
        );
    }

    #[test]
    fn renaming_one_file_touches_nothing_else() {
        let mut members = listing();
        apply_to_list(
            &mut members,
            &Edit::Rename { from: "readme.md".to_string(), to: "README.md".to_string() },
        );
        assert_eq!(
            paths(&members),
            ["docs", "docs/guide.txt", "docs/deep/more.txt", "README.md"]
        );
    }

    #[test]
    fn an_edit_naming_a_path_that_escapes_the_archive_does_nothing_at_all() {
        let mut members = listing();
        apply_to_list(&mut members, &Edit::Remove("../../etc".to_string()));
        assert_eq!(paths(&members).len(), 4);
    }
}
