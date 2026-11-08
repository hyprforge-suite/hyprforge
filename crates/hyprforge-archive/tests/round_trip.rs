//! Real archives, on a real disk, through the real backend.
//!
//! Everything in the crate's own unit tests runs against a seeded model
//! or a mock. These build actual files and read them back, because the
//! claims this crate makes about somebody else's format — that a tar's
//! directories come back, that a zip's timestamps survive, that 7z can
//! be written and reopened — are exactly the ones the type system cannot
//! see.
//!
//! No daemon, no compositor, no network: they run in tier 1.

use hyprforge_archive::backend::{
    ArchiveBackend, Collision, Edit, ExtractRequest, NoProgress,
};
use hyprforge_archive::{Compression, Format, Source, StdArchives, Unlock};
use std::path::{Path, PathBuf};

/// A small tree to pack: two files at different depths, and an empty
/// directory — which is the one thing that disappears from an archive
/// whose folders exist only by implication.
fn sample_tree(root: &Path) -> PathBuf {
    let tree = root.join("payload");
    std::fs::create_dir_all(tree.join("docs/deep")).unwrap();
    std::fs::create_dir_all(tree.join("empty")).unwrap();
    std::fs::write(tree.join("readme.md"), b"# hello").unwrap();
    std::fs::write(tree.join("docs/guide.txt"), b"a guide, of sorts").unwrap();
    std::fs::write(tree.join("docs/deep/more.txt"), vec![b'x'; 10_000]).unwrap();
    tree
}

fn sources(tree: &Path) -> Vec<Source> {
    vec![Source {
        path: tree.to_path_buf(),
        as_member: "payload".to_string(),
    }]
}

fn every_writable_format() -> Vec<(Format, &'static str)> {
    vec![
        (Format::Zip, "sample.zip"),
        (Format::Tar(Compression::None), "sample.tar"),
        (Format::Tar(Compression::Gzip), "sample.tar.gz"),
        (Format::Tar(Compression::Bzip2), "sample.tar.bz2"),
        (Format::Tar(Compression::Xz), "sample.tar.xz"),
        (Format::Tar(Compression::Zstd), "sample.tar.zst"),
        (Format::SevenZ, "sample.7z"),
    ]
}

#[test]
fn every_format_round_trips_a_tree_through_create_list_and_extract() {
    for (format, name) in every_writable_format() {
        let dir = tempfile::tempdir().unwrap();
        let tree = sample_tree(dir.path());
        let archive = dir.path().join(name);

        StdArchives
            .create(&archive, format, &sources(&tree), &mut NoProgress)
            .unwrap_or_else(|e| panic!("{name} could not be created: {e}"));

        // The listing agrees with what went in.
        let index = StdArchives
            .index(&archive)
            .unwrap_or_else(|e| panic!("{name} could not be listed: {e}"));
        assert!(
            index.get("payload/docs/guide.txt").is_some(),
            "{name} lost a nested file"
        );
        assert!(
            index.get("payload/docs").is_some_and(|m| m.is_dir),
            "{name} lost the directory above it"
        );
        assert_eq!(
            index.get("payload/docs/deep/more.txt").map(|m| m.size),
            Some(10_000),
            "{name} reported the wrong size for a member"
        );

        // One member's bytes come back byte for byte.
        let bytes = StdArchives
            .read_member(&archive, "payload/docs/guide.txt")
            .unwrap_or_else(|e| panic!("{name} could not read a member: {e}"));
        assert_eq!(bytes, b"a guide, of sorts", "{name} changed a member's contents");

        // And so does the whole thing, extracted.
        let out = dir.path().join("out");
        let report = StdArchives
            .extract(
                &archive,
                &ExtractRequest {
                    members: Vec::new(),
                    dest: out.clone(),
                    strip_prefix: None,
                    collision: Collision::Overwrite,
                },
                &mut NoProgress,
            )
            .unwrap_or_else(|e| panic!("{name} could not be extracted: {e}"));

        assert!(report.failed.is_empty(), "{name} failed members: {:?}", report.failed);
        assert_eq!(report.files, 3, "{name} extracted {} files", report.files);
        assert_eq!(
            std::fs::read(out.join("payload/readme.md")).unwrap(),
            b"# hello",
            "{name} corrupted a file on the way out"
        );
        assert_eq!(
            std::fs::read(out.join("payload/docs/deep/more.txt")).unwrap().len(),
            10_000,
            "{name} truncated the largest member"
        );
    }
}

/// An empty directory has no files under it to imply it, so it is the
/// one thing a "directories are synthesised from paths" design loses if
/// the writer does not record it deliberately.
#[test]
fn an_empty_directory_survives_every_format() {
    for (format, name) in every_writable_format() {
        let dir = tempfile::tempdir().unwrap();
        let tree = sample_tree(dir.path());
        let archive = dir.path().join(name);
        StdArchives
            .create(&archive, format, &sources(&tree), &mut NoProgress)
            .unwrap();

        let index = StdArchives.index(&archive).unwrap();
        assert!(
            index.get("payload/empty").is_some_and(|m| m.is_dir),
            "{name} lost an empty directory"
        );

        let out = dir.path().join("out");
        StdArchives
            .extract(
                &archive,
                &ExtractRequest {
                    members: Vec::new(),
                    dest: out.clone(),
                    strip_prefix: None,
                    collision: Collision::Overwrite,
                },
                &mut NoProgress,
            )
            .unwrap();
        assert!(
            out.join("payload/empty").is_dir(),
            "{name} did not put the empty directory back"
        );
    }
}

#[test]
fn extracting_one_folders_contents_puts_them_at_the_destination_itself() {
    let dir = tempfile::tempdir().unwrap();
    let tree = sample_tree(dir.path());
    let archive = dir.path().join("sample.zip");
    StdArchives
        .create(&archive, Format::Zip, &sources(&tree), &mut NoProgress)
        .unwrap();

    let out = dir.path().join("out");
    StdArchives
        .extract(
            &archive,
            &ExtractRequest {
                members: vec!["payload/docs".to_string()],
                dest: out.clone(),
                strip_prefix: Some("payload/docs".to_string()),
                collision: Collision::Overwrite,
            },
            &mut NoProgress,
        )
        .unwrap();

    assert!(out.join("guide.txt").is_file(), "the folder's contents land at the top");
    assert!(!out.join("payload").exists(), "and its path above it does not come along");
    assert!(!out.join("readme.md").exists(), "nor anything outside the selection");
}

#[test]
fn what_a_file_is_named_does_not_decide_what_it_is_read_as() {
    let dir = tempfile::tempdir().unwrap();
    let tree = sample_tree(dir.path());
    // A real gzipped tar, called something else entirely.
    let archive = dir.path().join("mystery.bin");
    StdArchives
        .create(
            &archive,
            Format::Tar(Compression::Gzip),
            &sources(&tree),
            &mut NoProgress,
        )
        .unwrap();

    let index = StdArchives.index(&archive).unwrap();
    assert!(
        index.get("payload/readme.md").is_some(),
        "a tarball must open under any name"
    );
}

/// The `.gz` that is not a `.tar.gz` — see `format`'s module doc.
#[test]
fn a_single_compressed_file_browses_as_one_member_named_without_its_suffix() {
    let dir = tempfile::tempdir().unwrap();
    let archive = dir.path().join("dump.sql.gz");
    {
        use std::io::Write;
        let file = std::fs::File::create(&archive).unwrap();
        let mut encoder = flate2::write::GzEncoder::new(file, flate2::Compression::default());
        encoder.write_all(b"SELECT 1;").unwrap();
        encoder.finish().unwrap();
    }

    let index = StdArchives.index(&archive).unwrap();
    let members: Vec<&str> = index.members().iter().map(|m| m.path.as_str()).collect();
    assert_eq!(members, ["dump.sql"]);
    assert!(
        !index.members()[0].size_known,
        "the decompressed size is not knowable without decompressing, and must not read as zero"
    );
    assert_eq!(StdArchives.read_member(&archive, "dump.sql").unwrap(), b"SELECT 1;");
}

/// Extraction writes the member under the name the listing gave it,
/// and that name comes from the *file's* name — the one thing a
/// compressed stream does not store. Reading through a pinned
/// descriptor briefly named it after the descriptor instead, which
/// would have written `dump.sql` to disk as a file called `9`; the
/// listing test above did not notice, because only extraction turns
/// the name into a path.
#[test]
fn a_single_compressed_file_extracts_under_its_own_name() {
    let dir = tempfile::tempdir().unwrap();
    let archive = dir.path().join("dump.sql.gz");
    {
        use std::io::Write;
        let file = std::fs::File::create(&archive).unwrap();
        let mut encoder = flate2::write::GzEncoder::new(file, flate2::Compression::default());
        encoder.write_all(b"SELECT 1;").unwrap();
        encoder.finish().unwrap();
    }
    let out = dir.path().join("out");

    StdArchives
        .extract(
            &archive,
            &ExtractRequest {
                members: Vec::new(),
                dest: out.clone(),
                strip_prefix: None,
                collision: Collision::Overwrite,
            },
            &mut NoProgress,
        )
        .unwrap();

    let written: Vec<String> = std::fs::read_dir(&out)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(written, ["dump.sql"]);
    assert_eq!(std::fs::read(out.join("dump.sql")).unwrap(), b"SELECT 1;");
}

#[test]
fn a_file_that_is_not_an_archive_says_so_rather_than_claiming_to_be_damaged() {
    let dir = tempfile::tempdir().unwrap();
    let liar = dir.path().join("photo.zip");
    std::fs::write(&liar, b"\xff\xd8\xff\xe0 this is a jpeg, honestly").unwrap();

    let err = StdArchives.index(&liar).expect_err("not an archive");
    let message = err.to_string().to_lowercase();
    assert!(!message.contains("damaged"), "{message}");
    assert!(message.contains("isn't an archive"), "{message}");
}

// --- editing --------------------------------------------------------

/// The three formats that can be edited, and the reason the fourth
/// cannot: `Format::Compressed` holds one stream with no room for a
/// second member.
fn every_editable_format() -> Vec<(Format, &'static str)> {
    every_writable_format()
}

#[test]
fn renaming_a_folder_inside_an_archive_moves_everything_under_it() {
    for (format, name) in every_editable_format() {
        let dir = tempfile::tempdir().unwrap();
        let tree = sample_tree(dir.path());
        let archive = dir.path().join(name);
        StdArchives
            .create(&archive, format, &sources(&tree), &mut NoProgress)
            .unwrap();

        StdArchives
            .edit(
                &archive,
                &[Edit::Rename {
                    from: "payload/docs".to_string(),
                    to: "payload/manual".to_string(),
                }],
                &mut NoProgress,
            )
            .unwrap_or_else(|e| panic!("{name} could not be edited: {e}"));

        let index = StdArchives.index(&archive).unwrap();
        assert!(index.get("payload/docs").is_none(), "{name} kept the old name");
        assert!(
            index.get("payload/manual/deep/more.txt").is_some(),
            "{name} lost a file under the renamed folder"
        );
        assert_eq!(
            StdArchives.read_member(&archive, "payload/manual/guide.txt").unwrap(),
            b"a guide, of sorts",
            "{name} changed the contents of a renamed member"
        );
        // Untouched members are still untouched.
        assert!(index.get("payload/readme.md").is_some(), "{name} lost an unrelated file");
    }
}

#[test]
fn removing_a_folder_removes_what_was_inside_it_in_the_file_as_well() {
    for (format, name) in every_editable_format() {
        let dir = tempfile::tempdir().unwrap();
        let tree = sample_tree(dir.path());
        let archive = dir.path().join(name);
        StdArchives
            .create(&archive, format, &sources(&tree), &mut NoProgress)
            .unwrap();

        StdArchives
            .edit(&archive, &[Edit::Remove("payload/docs".to_string())], &mut NoProgress)
            .unwrap();

        let index = StdArchives.index(&archive).unwrap();
        assert!(index.get("payload/docs").is_none(), "{name} left the folder standing");
        assert!(
            index.get("payload/docs/guide.txt").is_none(),
            "{name} left a file inside a folder it deleted"
        );
        assert!(index.get("payload/readme.md").is_some(), "{name} deleted too much");
    }
}

#[test]
fn adding_a_file_to_an_archive_puts_it_where_it_was_asked_for() {
    for (format, name) in every_editable_format() {
        let dir = tempfile::tempdir().unwrap();
        let tree = sample_tree(dir.path());
        let archive = dir.path().join(name);
        StdArchives
            .create(&archive, format, &sources(&tree), &mut NoProgress)
            .unwrap();

        let extra = dir.path().join("notes.txt");
        std::fs::write(&extra, b"added later").unwrap();
        StdArchives
            .edit(
                &archive,
                &[Edit::Add {
                    source: extra,
                    as_member: "payload/docs/notes.txt".to_string(),
                }],
                &mut NoProgress,
            )
            .unwrap();

        assert_eq!(
            StdArchives.read_member(&archive, "payload/docs/notes.txt").unwrap(),
            b"added later",
            "{name} did not add the file"
        );
        assert!(
            StdArchives.index(&archive).unwrap().get("payload/readme.md").is_some(),
            "{name} lost what was already there"
        );
    }
}

/// Both zip and tar permit the same name twice, and the last one wins
/// when anything unpacks the archive — so an "add" that left the old
/// member in place would produce a file that reads back correctly and
/// is quietly twice the size, with a superseded copy inside it.
#[test]
fn adding_over_an_existing_member_replaces_it_rather_than_shadowing_it() {
    let dir = tempfile::tempdir().unwrap();
    let tree = sample_tree(dir.path());
    let archive = dir.path().join("sample.tar");
    StdArchives
        .create(&archive, Format::Tar(Compression::None), &sources(&tree), &mut NoProgress)
        .unwrap();

    let replacement = dir.path().join("replacement");
    std::fs::write(&replacement, b"the new guide").unwrap();
    StdArchives
        .edit(
            &archive,
            &[Edit::Add {
                source: replacement,
                as_member: "payload/docs/guide.txt".to_string(),
            }],
            &mut NoProgress,
        )
        .unwrap();

    let index = StdArchives.index(&archive).unwrap();
    let copies = index
        .members()
        .iter()
        .filter(|m| m.path == "payload/docs/guide.txt")
        .count();
    assert_eq!(copies, 1, "the superseded member is still in the file");
    assert_eq!(
        StdArchives.read_member(&archive, "payload/docs/guide.txt").unwrap(),
        b"the new guide"
    );
}

/// The property the temporary-file-then-rename dance exists for: an edit
/// that fails must leave the original archive exactly as it was, not a
/// truncated file where one used to be.
#[test]
fn an_edit_that_fails_leaves_the_original_archive_untouched() {
    let dir = tempfile::tempdir().unwrap();
    let tree = sample_tree(dir.path());
    let archive = dir.path().join("sample.zip");
    StdArchives
        .create(&archive, Format::Zip, &sources(&tree), &mut NoProgress)
        .unwrap();
    let before = std::fs::read(&archive).unwrap();

    // A source that is not there is the simplest way to fail partway
    // through the rewrite, after the archive has been read.
    let err = StdArchives.edit(
        &archive,
        &[Edit::Add {
            source: dir.path().join("does-not-exist"),
            as_member: "payload/nope.txt".to_string(),
        }],
        &mut NoProgress,
    );
    assert!(err.is_err(), "adding a file that is not there should fail");

    assert_eq!(
        std::fs::read(&archive).unwrap(),
        before,
        "a failed edit rewrote the archive anyway"
    );
    // And nothing is left lying beside it.
    let leftovers: Vec<_> = std::fs::read_dir(dir.path())
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.contains("hyprforge-new"))
        .collect();
    assert!(leftovers.is_empty(), "left a temporary file behind: {leftovers:?}");
}

#[test]
fn a_single_compressed_file_cannot_be_edited_and_says_why() {
    let dir = tempfile::tempdir().unwrap();
    let archive = dir.path().join("dump.sql.gz");
    {
        use std::io::Write;
        let file = std::fs::File::create(&archive).unwrap();
        let mut encoder = flate2::write::GzEncoder::new(file, flate2::Compression::default());
        encoder.write_all(b"SELECT 1;").unwrap();
        encoder.finish().unwrap();
    }

    let err = StdArchives
        .edit(&archive, &[Edit::Remove("dump.sql".to_string())], &mut NoProgress)
        .expect_err("a single stream has nothing to edit");
    assert!(err.to_string().contains("single stream"), "{err}");
}

/// A crafted archive, not a seeded model: the unit tests pin
/// `normalise` and `safe_join` in isolation, and this is the same claim
/// made against a file a hostile tool actually wrote.
#[test]
fn a_member_that_climbs_out_of_the_destination_writes_nothing_outside_it() {
    let dir = tempfile::tempdir().unwrap();
    let archive = dir.path().join("hostile.zip");
    {
        use std::io::Write;
        let file = std::fs::File::create(&archive).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        let options = zip::write::SimpleFileOptions::default();
        zip.start_file("../../escaped.txt", options).unwrap();
        zip.write_all(b"should never be written").unwrap();
        zip.start_file("innocent.txt", options).unwrap();
        zip.write_all(b"fine").unwrap();
        zip.finish().unwrap();
    }

    let out = dir.path().join("deep").join("out");
    let report = StdArchives.extract(
        &archive,
        &ExtractRequest {
            members: Vec::new(),
            dest: out.clone(),
            strip_prefix: None,
            collision: Collision::Overwrite,
        },
        &mut NoProgress,
    );

    // Whether the escaping member is dropped from the listing or refused
    // by the plan, the one thing that must be true is that nothing
    // landed outside the destination.
    assert!(
        !dir.path().join("escaped.txt").exists(),
        "an archive member wrote outside the folder it was extracted into"
    );
    assert!(!dir.path().join("deep/escaped.txt").exists());
    if let Ok(report) = report {
        assert!(report.failed.is_empty(), "{:?}", report.failed);
        assert!(out.join("innocent.txt").exists(), "the safe member still extracts");
    }
}

// --- encrypted archives ---------------------------------------------

/// Writes a zip whose one member is AES-encrypted, plus one that is not
/// — a mixture, because a zip encrypts per entry rather than as a whole
/// and the reader has to cope with both in one file.
fn encrypted_zip(dir: &Path, password: &str) -> PathBuf {
    use std::io::Write;
    let archive = dir.join("secret.zip");
    let file = std::fs::File::create(&archive).unwrap();
    let mut zip = zip::ZipWriter::new(file);

    let locked = zip::write::SimpleFileOptions::default()
        .with_aes_encryption(zip::AesMode::Aes256, password);
    zip.start_file("secret.txt", locked).unwrap();
    zip.write_all(b"the hidden thing").unwrap();

    let open = zip::write::SimpleFileOptions::default();
    zip.start_file("public.txt", open).unwrap();
    zip.write_all(b"nothing to hide").unwrap();

    zip.finish().unwrap();
    archive
}

/// A zip's central directory is not encrypted, so the *names* are
/// readable without a password. Refusing to list one would hide
/// information the format gives away anyway, and would leave someone
/// unable to see what they are being asked a password for.
#[test]
fn an_encrypted_zip_still_lists_its_contents_without_a_password() {
    let dir = tempfile::tempdir().unwrap();
    let archive = encrypted_zip(dir.path(), "hunter2");

    let index = StdArchives.index(&archive).unwrap();
    let mut names: Vec<&str> = index.members().iter().map(|m| m.path.as_str()).collect();
    names.sort();
    assert_eq!(names, ["public.txt", "secret.txt"]);
    assert!(
        index.get("secret.txt").is_some_and(|m| m.encrypted),
        "the listing has to say which rows are locked"
    );
    assert!(
        index.get("public.txt").is_some_and(|m| !m.encrypted),
        "and which are not — a zip encrypts per entry, not as a whole"
    );
}

#[test]
fn reading_an_encrypted_member_without_a_password_asks_for_one() {
    let dir = tempfile::tempdir().unwrap();
    let archive = encrypted_zip(dir.path(), "hunter2");

    let err = StdArchives
        .read_member(&archive, "secret.txt")
        .expect_err("an encrypted member must not read as empty");
    assert!(
        matches!(err, hyprforge_archive::ArchiveError::PasswordRequired { .. }),
        "the caller has to be able to tell this from a damaged archive: {err}"
    );
    assert!(err.to_string().contains("password"), "{err}");
}

#[test]
fn the_unencrypted_half_of_a_mixed_zip_needs_no_password() {
    let dir = tempfile::tempdir().unwrap();
    let archive = encrypted_zip(dir.path(), "hunter2");

    assert_eq!(
        StdArchives.read_member(&archive, "public.txt").unwrap(),
        b"nothing to hide",
        "asking for a password to read an entry that has none would be asking for nothing"
    );
}

#[test]
fn the_right_password_reads_an_encrypted_member() {
    let dir = tempfile::tempdir().unwrap();
    let archive = encrypted_zip(dir.path(), "hunter2");

    let unlock = Unlock::with(hyprforge_archive::Secret::new("hunter2".to_string()));
    assert_eq!(
        StdArchives.read_member_with(&archive, "secret.txt", &unlock).unwrap(),
        b"the hidden thing"
    );
}

/// A wrong password is "ask again", never "this archive is damaged" —
/// which would send someone looking for a backup of a perfectly good
/// file.
#[test]
fn a_wrong_password_asks_again_rather_than_calling_the_archive_damaged() {
    let dir = tempfile::tempdir().unwrap();
    let archive = encrypted_zip(dir.path(), "hunter2");

    let unlock = Unlock::with(hyprforge_archive::Secret::new("wrong".to_string()));
    let err = StdArchives
        .read_member_with(&archive, "secret.txt", &unlock)
        .expect_err("a wrong password must not return plausible bytes");
    assert!(
        matches!(err, hyprforge_archive::ArchiveError::PasswordRequired { .. }),
        "{err}"
    );
    let message = err.to_string().to_lowercase();
    assert!(!message.contains("damaged"), "{message}");
}

#[test]
fn extracting_an_encrypted_zip_with_the_password_writes_every_member() {
    let dir = tempfile::tempdir().unwrap();
    let archive = encrypted_zip(dir.path(), "hunter2");
    let out = dir.path().join("out");

    let unlock = Unlock::with(hyprforge_archive::Secret::new("hunter2".to_string()));
    let report = StdArchives
        .extract_with(
            &archive,
            &ExtractRequest {
                members: Vec::new(),
                dest: out.clone(),
                strip_prefix: None,
                collision: Collision::Overwrite,
            },
            &unlock,
            &mut NoProgress,
        )
        .unwrap();

    assert!(report.failed.is_empty(), "{:?}", report.failed);
    assert_eq!(std::fs::read(out.join("secret.txt")).unwrap(), b"the hidden thing");
    assert_eq!(std::fs::read(out.join("public.txt")).unwrap(), b"nothing to hide");
}

/// Without the password the encrypted member is a failed row and the
/// rest still comes out — the same "one bad member does not sink the
/// extraction" rule everything else here follows.
#[test]
fn extracting_without_the_password_still_writes_what_it_can() {
    let dir = tempfile::tempdir().unwrap();
    let archive = encrypted_zip(dir.path(), "hunter2");
    let out = dir.path().join("out");

    let report = StdArchives
        .extract(
            &archive,
            &ExtractRequest {
                members: Vec::new(),
                dest: out.clone(),
                strip_prefix: None,
                collision: Collision::Overwrite,
            },
            &mut NoProgress,
        )
        .unwrap();

    assert_eq!(report.failed.len(), 1, "{:?}", report.failed);
    assert_eq!(report.failed[0].member, "secret.txt");
    // The *reason*, so the caller can open a prompt instead of showing
    // an error — see `MemberFailure`.
    assert_eq!(
        report.failed[0].reason,
        hyprforge_archive::backend::FailureReason::NeedsPassword
    );
    assert!(
        out.join("public.txt").exists(),
        "the member that needed no password still had to be written"
    );
}

/// Two rewrites of one archive at the same time.
///
/// What a library can promise on its own is that the file left behind is
/// a complete archive one writer wrote, and that neither leaves its
/// half-written bytes there or removes the other's temporary out from
/// under it. It cannot promise both edits survive — each reads the whole
/// archive and writes a whole new one, so when they overlap the later
/// rename wins. Serialising that belongs to whoever is scheduling the
/// work.
///
/// "When they overlap" is not guaranteed either: two threads spawned
/// together can still run one after the other, and then the second
/// reads what the first wrote and both edits survive. That is a correct
/// outcome, so the assertion allows it. This test used to demand that
/// exactly one edit won, which only held while the scheduler happened to
/// interleave them.
///
/// Under load it also found a real race: an edit listed the archive and
/// then unpacked it by opening the path twice, and a rename between the
/// two left it repacking a member that was never unpacked — `NotFound`,
/// three runs in two hundred. `write::edit` reads a snapshot now, and
/// `an_archive_replaced_mid_edit_is_edited_as_it_was_when_the_edit_began`
/// in `src/write.rs` puts the rename in that window deterministically.
///
/// This existed as a much worse bug before the temporary file was made
/// unique: with a fixed `.<name>.hyprforge-new`, the edit that returned
/// `Ok` had its change lost and the edit that returned `Err` had its
/// change applied.
#[test]
fn two_rewrites_at_once_leave_a_whole_archive_and_an_honest_answer() {
    let dir = tempfile::tempdir().unwrap();
    let tree = sample_tree(dir.path());
    let archive = dir.path().join("sample.zip");
    StdArchives
        .create(&archive, Format::Zip, &sources(&tree), &mut NoProgress)
        .unwrap();

    let (one, two) = (archive.clone(), archive.clone());
    let first = std::thread::spawn(move || {
        StdArchives.edit(&one, &[Edit::Remove("payload/readme.md".into())], &mut NoProgress)
    });
    let second = std::thread::spawn(move || {
        StdArchives.edit(&two, &[Edit::Remove("payload/docs/guide.txt".into())], &mut NoProgress)
    });
    let (first, second) = (first.join().unwrap(), second.join().unwrap());

    // Neither may report a failure it did not have, nor a success it did
    // not have — which is exactly what the shared temporary produced.
    assert!(first.is_ok() && second.is_ok(), "{first:?} {second:?}");

    // Whatever is there reads as a complete archive written by one of the
    // two: either edit alone (they overlapped and the later rename won)
    // or both (they ran one after the other). Never neither — that would
    // mean a rewrite reported `Ok` and left no trace.
    let index = StdArchives.index(&archive).expect("still a readable archive");
    let removed_readme = index.get("payload/readme.md").is_none();
    let removed_guide = index.get("payload/docs/guide.txt").is_none();
    assert!(
        removed_readme || removed_guide,
        "both edits reported success and neither took effect"
    );
    assert!(
        index.get("payload/docs/deep/more.txt").is_some(),
        "and the members neither edit touched are still there"
    );

    // Nothing left lying beside it.
    let leftovers: Vec<String> = std::fs::read_dir(dir.path())
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.contains("hyprforge-new"))
        .collect();
    assert!(leftovers.is_empty(), "{leftovers:?}");
}

/// The guard on the coupling that was there before `MemberFailure` had
/// a `reason`: a caller decided "does this need a password" by searching
/// the message for the word. Rewording the sentence would have turned a
/// password prompt into a dead end, silently. Nothing should have to
/// read the prose to act.
#[test]
fn a_callers_decision_never_depends_on_the_wording_of_a_failure() {
    use hyprforge_archive::backend::FailureReason;

    let dir = tempfile::tempdir().unwrap();
    let archive = encrypted_zip(dir.path(), "hunter2");
    let out = dir.path().join("out");

    let report = StdArchives
        .extract(
            &archive,
            &ExtractRequest {
                members: Vec::new(),
                dest: out,
                strip_prefix: None,
                collision: Collision::Overwrite,
            },
            &mut NoProgress,
        )
        .unwrap();

    let locked = report.failed.iter().find(|f| f.member == "secret.txt").unwrap();
    assert_eq!(locked.reason, FailureReason::NeedsPassword);
    // The message is for showing, and is free to change. The test says
    // so rather than pinning it.
    assert!(!locked.message.is_empty());
}

// --- an edit is a rewrite, and must not lose anything on the way -----

fn edit(archive: &Path, edits: &[Edit]) -> hyprforge_archive::Result<()> {
    StdArchives.edit(archive, edits, &mut NoProgress)
}

fn extract_all(archive: &Path, out: &Path, collision: Collision) -> hyprforge_archive::ExtractReport {
    StdArchives
        .extract(
            archive,
            &ExtractRequest {
                members: Vec::new(),
                dest: out.to_path_buf(),
                strip_prefix: None,
                collision,
            },
            &mut NoProgress,
        )
        .unwrap()
}

/// An edit unpacks and repacks, and the writers here write plain
/// archives — so renaming one member of an unlocked `secrets.zip` used
/// to write every member back in the clear under the same name. The
/// refusal has to come before anything is unpacked, and the right
/// password must not get around it.
#[test]
fn editing_an_encrypted_zip_is_refused_and_leaves_it_byte_for_byte() {
    let dir = tempfile::tempdir().unwrap();
    let archive = encrypted_zip(dir.path(), "hunter2");
    let before = std::fs::read(&archive).unwrap();

    let unlock = Unlock::with(hyprforge_archive::Secret::new("hunter2".to_string()));
    let error = StdArchives
        .edit_with(&archive, &[Edit::Remove("public.txt".into())], &unlock, &mut NoProgress)
        .expect_err("an encrypted archive must not be rewritten in the clear");

    assert!(
        matches!(error, hyprforge_archive::ArchiveError::EncryptedEdit { .. }),
        "{error:?}"
    );
    assert_eq!(std::fs::read(&archive).unwrap(), before);
    let left: Vec<String> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(left, ["secret.zip"], "no scratch copy of the plaintext is left behind");
}

/// The 7z half: a 7z encrypts blocks rather than entries, and its
/// listing has no per-entry flag of its own — it has to be worked out
/// from the block's coders.
#[test]
fn editing_an_encrypted_7z_is_refused_and_leaves_it_byte_for_byte() {
    let dir = tempfile::tempdir().unwrap();
    let archive = dir.path().join("secret.7z");
    {
        let mut writer = sevenz_rust2::ArchiveWriter::create(&archive).unwrap();
        writer.set_content_methods(vec![
            sevenz_rust2::encoder_options::AesEncoderOptions::new("hunter2".into()).into(),
            sevenz_rust2::EncoderMethod::LZMA2.into(),
        ]);
        // Header left readable, so the listing works without a
        // password and the edit has every chance to go wrong.
        writer.set_encrypt_header(false);
        for name in ["a.txt", "b.txt"] {
            writer
                .push_archive_entry(
                    sevenz_rust2::ArchiveEntry::new_file(name),
                    Some(&b"private"[..]),
                )
                .unwrap();
        }
        writer.finish().unwrap();
    }
    let before = std::fs::read(&archive).unwrap();

    let unlock = Unlock::with(hyprforge_archive::Secret::new("hunter2".to_string()));
    let error = StdArchives
        .edit_with(&archive, &[Edit::Remove("b.txt".into())], &unlock, &mut NoProgress)
        .expect_err("an encrypted 7z must not be rewritten in the clear");

    assert!(
        matches!(error, hyprforge_archive::ArchiveError::EncryptedEdit { .. }),
        "{error:?}"
    );
    assert_eq!(std::fs::read(&archive).unwrap(), before);
}

/// A member whose bytes do not match its CRC unpacks short, and an edit
/// used to pack the short copy into the rewritten archive and report
/// success — losing the member's real contents for good, in a file the
/// person was only renaming something else in.
#[test]
fn an_edit_of_an_archive_with_a_member_that_will_not_read_fails_and_changes_nothing() {
    use std::io::Write;
    let dir = tempfile::tempdir().unwrap();
    let archive = dir.path().join("sample.zip");
    {
        let mut zip = zip::ZipWriter::new(std::fs::File::create(&archive).unwrap());
        let stored = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        zip.start_file("damaged.txt", stored).unwrap();
        zip.write_all(b"MARKER-the-original-contents").unwrap();
        zip.start_file("fine.txt", stored).unwrap();
        zip.write_all(b"fine").unwrap();
        zip.finish().unwrap();
    }
    // Corrupt the stored bytes in place, so the CRC no longer matches.
    let mut bytes = std::fs::read(&archive).unwrap();
    let at = bytes.windows(6).position(|w| w == b"MARKER").unwrap();
    bytes[at] = b'X';
    std::fs::write(&archive, &bytes).unwrap();

    let error = edit(&archive, &[Edit::Remove("fine.txt".into())])
        .expect_err("an edit that could not read a member must not succeed");
    assert!(!error.cancelled());
    assert_eq!(std::fs::read(&archive).unwrap(), bytes, "the archive is untouched");
}

/// The tar half, where it did reach the archive: a member cut short
/// unpacks as far as the bytes go, and the short copy was packed back in
/// as though it were the member.
#[test]
fn an_edit_of_a_tar_whose_member_is_cut_short_fails_and_changes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let archive = dir.path().join("short.tar");
    {
        let mut tar = tar::Builder::new(Vec::new());
        for (name, contents) in [("a.txt", vec![b'a'; 10]), ("big.bin", vec![b'b'; 100_000])] {
            let mut header = tar::Header::new_gnu();
            header.set_size(contents.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            tar.append_data(&mut header, name, &contents[..]).unwrap();
        }
        let mut bytes = tar.into_inner().unwrap();
        // Cut inside `big.bin`'s data: its header promised 100,000 bytes.
        bytes.truncate(512 * 3 + 20_000);
        std::fs::write(&archive, bytes).unwrap();
    }
    let before = std::fs::read(&archive).unwrap();

    edit(&archive, &[Edit::Remove("a.txt".into())])
        .expect_err("an edit that could not read a member must not succeed");
    assert_eq!(std::fs::read(&archive).unwrap(), before, "the archive is untouched");
}

/// A zip edit went through a scratch directory and a writer that set no
/// permissions, so every member came back 0644 and a script lost `+x`.
#[test]
fn an_edit_keeps_a_members_permissions() {
    use std::io::Write;
    let dir = tempfile::tempdir().unwrap();
    let archive = dir.path().join("tools.zip");
    {
        let mut zip = zip::ZipWriter::new(std::fs::File::create(&archive).unwrap());
        let exec = zip::write::SimpleFileOptions::default().unix_permissions(0o755);
        zip.start_file("run.sh", exec).unwrap();
        zip.write_all(b"#!/bin/sh\n").unwrap();
        zip.start_file("notes.txt", zip::write::SimpleFileOptions::default()).unwrap();
        zip.write_all(b"x").unwrap();
        zip.finish().unwrap();
    }

    edit(&archive, &[Edit::Remove("notes.txt".into())]).unwrap();

    let index = StdArchives.index(&archive).unwrap();
    let mode = index.get("run.sh").and_then(|m| m.mode).unwrap();
    assert_eq!(mode & 0o777, 0o755);
}

/// The same for a new archive: a zip made from an executable file kept
/// the bytes and dropped the bit.
#[test]
fn creating_a_zip_keeps_a_files_permissions() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let script = dir.path().join("run.sh");
    std::fs::write(&script, b"#!/bin/sh\n").unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    let archive = dir.path().join("tools.zip");
    StdArchives
        .create(
            &archive,
            Format::Zip,
            &[Source { path: script, as_member: "run.sh".into() }],
            &mut NoProgress,
        )
        .unwrap();

    let index = StdArchives.index(&archive).unwrap();
    assert_eq!(index.get("run.sh").and_then(|m| m.mode).unwrap() & 0o777, 0o755);
}

/// An edit through a symlinked archive replaced the *link* with a
/// regular file, leaving the real archive unedited and the link gone.
#[test]
fn editing_a_symlinked_archive_edits_the_archive_and_keeps_the_link() {
    let dir = tempfile::tempdir().unwrap();
    let tree = sample_tree(dir.path());
    let real = dir.path().join("real.zip");
    StdArchives.create(&real, Format::Zip, &sources(&tree), &mut NoProgress).unwrap();
    let link = dir.path().join("link.zip");
    std::os::unix::fs::symlink("real.zip", &link).unwrap();

    edit(&link, &[Edit::Remove("payload/readme.md".into())]).unwrap();

    assert!(std::fs::symlink_metadata(&link).unwrap().file_type().is_symlink());
    assert!(StdArchives.index(&real).unwrap().get("payload/readme.md").is_none());
}

// --- a tar can say the same name twice ------------------------------

/// A tar with `notes.txt` written twice, the way `tar rf` leaves one.
fn appended_tar(dir: &Path) -> PathBuf {
    let archive = dir.join("appended.tar");
    let mut tar = tar::Builder::new(std::fs::File::create(&archive).unwrap());
    for contents in [&b"old"[..], &b"new"[..]] {
        let mut header = tar::Header::new_gnu();
        header.set_size(contents.len() as u64);
        header.set_mode(0o644);
        header.set_cksum();
        tar.append_data(&mut header, "notes.txt", contents).unwrap();
    }
    tar.finish().unwrap();
    archive
}

/// Last-one-wins, and the *contents* are what say which one won: the
/// first copy landed on an empty destination, and the second then met
/// the extraction's own write as a collision — kept apart under Rename,
/// thrown away under Skip.
#[test]
fn a_repeated_tar_member_extracts_as_its_last_version_under_every_collision_rule() {
    for collision in [Collision::Overwrite, Collision::Skip, Collision::Rename] {
        let dir = tempfile::tempdir().unwrap();
        let archive = appended_tar(dir.path());
        let out = dir.path().join("out");

        let report = extract_all(&archive, &out, collision);

        assert!(report.failed.is_empty(), "{collision:?}: {:?}", report.failed);
        assert_eq!(std::fs::read(out.join("notes.txt")).unwrap(), b"new", "{collision:?}");
        let names: Vec<String> = std::fs::read_dir(&out)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, ["notes.txt"], "{collision:?}");
        assert!(report.skipped.is_empty(), "{collision:?}");
    }
}

/// The collision rule still applies to what was there *before* the
/// extraction: a person's own `notes.txt` is not the archive's to
/// replace under Skip, however many times the archive names it.
#[test]
fn a_repeated_tar_member_still_respects_a_file_that_was_already_there() {
    let dir = tempfile::tempdir().unwrap();
    let archive = appended_tar(dir.path());
    let out = dir.path().join("out");
    std::fs::create_dir(&out).unwrap();
    std::fs::write(out.join("notes.txt"), b"mine").unwrap();

    extract_all(&archive, &out, Collision::Skip);
    assert_eq!(std::fs::read(out.join("notes.txt")).unwrap(), b"mine");

    extract_all(&archive, &out, Collision::Rename);
    assert_eq!(std::fs::read(out.join("notes.txt")).unwrap(), b"mine");
    assert_eq!(std::fs::read(out.join("notes (1).txt")).unwrap(), b"new");
    assert!(!out.join("notes (2).txt").exists());
}

// --- hard links -----------------------------------------------------

/// A tar hard link names its target from the archive's root, not from
/// the link's own folder. Written as a symlink, `other/b.txt -> dir/a.txt`
/// pointed at `other/dir/a.txt`, which does not exist.
#[test]
fn a_tar_hard_link_extracts_as_the_file_it_links_to() {
    let dir = tempfile::tempdir().unwrap();
    let archive = dir.path().join("linked.tar");
    {
        let mut tar = tar::Builder::new(std::fs::File::create(&archive).unwrap());
        let mut header = tar::Header::new_gnu();
        header.set_size(5);
        header.set_mode(0o644);
        header.set_cksum();
        tar.append_data(&mut header, "dir/a.txt", &b"hello"[..]).unwrap();

        let mut link = tar::Header::new_gnu();
        link.set_entry_type(tar::EntryType::Link);
        link.set_size(0);
        link.set_mode(0o644);
        tar.append_link(&mut link, "other/b.txt", "dir/a.txt").unwrap();
        tar.finish().unwrap();
    }
    let out = dir.path().join("out");

    let report = extract_all(&archive, &out, Collision::Overwrite);

    assert!(report.failed.is_empty(), "{:?}", report.failed);
    assert_eq!(std::fs::read(out.join("other/b.txt")).unwrap(), b"hello");
    assert!(!std::fs::symlink_metadata(out.join("other/b.txt")).unwrap().file_type().is_symlink());
}

/// A hard link is not a way out either: its target is checked the way
/// every member path is.
#[test]
fn a_tar_hard_link_to_somewhere_outside_the_archive_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let outside = dir.path().join("outside.txt");
    std::fs::write(&outside, b"private").unwrap();
    let archive = dir.path().join("linked.tar");
    {
        let mut tar = tar::Builder::new(std::fs::File::create(&archive).unwrap());
        let mut link = tar::Header::new_gnu();
        link.set_entry_type(tar::EntryType::Link);
        link.set_size(0);
        link.set_mode(0o644);
        tar.append_link(&mut link, "stolen.txt", outside.to_str().unwrap()).unwrap();
        tar.finish().unwrap();
    }
    let out = dir.path().join("out");

    let report = extract_all(&archive, &out, Collision::Overwrite);

    assert_eq!(report.failed.len(), 1, "{:?}", report.failed);
    assert!(!out.join("stolen.txt").exists());
}

// --- what an extraction writes through ------------------------------

/// Overwrite replaces what is at the destination; it does not write
/// *through* a symlink sitting there to wherever the link points.
#[test]
fn overwriting_never_writes_through_a_symlink_at_the_destination() {
    let dir = tempfile::tempdir().unwrap();
    let elsewhere = dir.path().join("elsewhere.txt");
    std::fs::write(&elsewhere, b"untouched").unwrap();
    let out = dir.path().join("out");
    std::fs::create_dir(&out).unwrap();
    std::os::unix::fs::symlink(&elsewhere, out.join("notes.txt")).unwrap();

    let archive = appended_tar(dir.path());
    extract_all(&archive, &out, Collision::Overwrite);

    assert_eq!(std::fs::read(&elsewhere).unwrap(), b"untouched");
    assert!(!std::fs::symlink_metadata(out.join("notes.txt")).unwrap().file_type().is_symlink());
    assert_eq!(std::fs::read(out.join("notes.txt")).unwrap(), b"new");
}

/// What the process umask reads as — the test's own copy, from the
/// same place the crate reads it, so the assertion is about the
/// extraction and not about this machine's settings.
fn umask() -> u32 {
    let status = std::fs::read_to_string("/proc/self/status").unwrap();
    let line = status.lines().find(|l| l.starts_with("Umask:")).unwrap();
    u32::from_str_radix(line["Umask:".len()..].trim(), 8).unwrap()
}

/// An archive's stated mode went straight onto the extracted file, so a
/// member marked 0666 came out writable by every user on the machine,
/// whatever the person's umask said.
#[test]
fn an_extracted_file_is_never_more_open_than_the_umask_allows() {
    let dir = tempfile::tempdir().unwrap();
    let archive = dir.path().join("open.tar");
    {
        let mut tar = tar::Builder::new(std::fs::File::create(&archive).unwrap());
        let mut header = tar::Header::new_gnu();
        header.set_size(1);
        header.set_mode(0o666);
        header.set_cksum();
        tar.append_data(&mut header, "shared.txt", &b"x"[..]).unwrap();
        tar.finish().unwrap();
    }
    let out = dir.path().join("out");
    extract_all(&archive, &out, Collision::Overwrite);

    use std::os::unix::fs::PermissionsExt;
    let mode = std::fs::metadata(out.join("shared.txt")).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o666 & !umask());
}

// --- what compressing a folder takes with it ------------------------

/// A folder holding a link to somewhere else — `~/.ssh`, say. Following
/// it put the private files into the archive; a link back up (`a -> ..`)
/// made the walk exponential. Links are stored as links.
#[test]
fn compressing_a_folder_stores_its_symlinks_as_links_and_never_follows_them() {
    let dir = tempfile::tempdir().unwrap();
    let secrets = dir.path().join("secrets");
    std::fs::create_dir(&secrets).unwrap();
    std::fs::write(secrets.join("id_ed25519"), b"PRIVATE").unwrap();
    let tree = dir.path().join("payload");
    std::fs::create_dir(&tree).unwrap();
    std::fs::write(tree.join("readme.md"), b"hi").unwrap();
    std::os::unix::fs::symlink(&secrets, tree.join("ssh")).unwrap();
    std::os::unix::fs::symlink("..", tree.join("up")).unwrap();

    for (format, name) in [(Format::Zip, "s.zip"), (Format::Tar(Compression::Gzip), "s.tar.gz")] {
        let archive = dir.path().join(name);
        StdArchives.create(&archive, format, &sources(&tree), &mut NoProgress).unwrap();
        let index = StdArchives.index(&archive).unwrap();
        let names: Vec<&str> = index.members().iter().map(|m| m.path.as_str()).collect();
        assert!(
            !names.iter().any(|n| n.contains("id_ed25519") || n.starts_with("payload/up/")),
            "{name}: {names:?}"
        );
        assert_eq!(
            index.get("payload/ssh").and_then(|m| m.link_target.clone()).as_deref(),
            secrets.to_str(),
            "{name}"
        );
        assert_eq!(
            index.get("payload/up").and_then(|m| m.link_target.clone()).as_deref(),
            Some(".."),
            "{name}"
        );
    }

    // 7z has nowhere here to put a link, so it leaves them out rather
    // than following them.
    let archive = dir.path().join("s.7z");
    StdArchives.create(&archive, Format::SevenZ, &sources(&tree), &mut NoProgress).unwrap();
    let index = StdArchives.index(&archive).unwrap();
    let names: Vec<&str> = index.members().iter().map(|m| m.path.as_str()).collect();
    assert!(names.contains(&"payload/readme.md"), "{names:?}");
    assert!(!names.iter().any(|n| n.contains("id_ed25519") || n.contains("ssh") || n.contains("up")), "{names:?}");
}

/// A socket (or a FIFO, which would have blocked forever in `open`)
/// inside a folder is not something an archive can hold. It is left out
/// and the rest of the folder still compresses.
#[test]
fn compressing_a_folder_leaves_out_what_is_not_a_file_a_folder_or_a_link() {
    let dir = tempfile::tempdir().unwrap();
    let tree = dir.path().join("payload");
    std::fs::create_dir(&tree).unwrap();
    std::fs::write(tree.join("readme.md"), b"hi").unwrap();
    let _listener = std::os::unix::net::UnixListener::bind(tree.join("daemon.sock")).unwrap();

    for (format, name) in every_writable_format() {
        let archive = dir.path().join(name);
        StdArchives
            .create(&archive, format, &sources(&tree), &mut NoProgress)
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        let index = StdArchives.index(&archive).unwrap();
        assert!(index.get("payload/readme.md").is_some(), "{name}");
        assert!(index.get("payload/daemon.sock").is_none(), "{name}");
    }
}

// --- what browsing an archive costs ---------------------------------

/// A GNU long-name header is read whole before the member it names is
/// handed over — and its size is whatever the archive claims. Two
/// megabytes of name compress to almost nothing, and two gigabytes do
/// too. One past the limit is enough to show the claim is refused
/// rather than believed.
#[test]
fn a_tar_header_claiming_an_enormous_name_is_refused_not_read() {
    use std::io::Write;
    let dir = tempfile::tempdir().unwrap();
    let archive = dir.path().join("huge-name.tar.gz");
    let file = std::fs::File::create(&archive).unwrap();
    let mut gz = flate2::write::GzEncoder::new(file, flate2::Compression::fast());

    let name_len: u64 = 2 * 1024 * 1024;
    let mut long = tar::Header::new_gnu();
    long.set_entry_type(tar::EntryType::GNULongName);
    long.as_gnu_mut().unwrap().name[..13].copy_from_slice(b"././@LongLink");
    long.set_size(name_len);
    long.set_mode(0o644);
    long.set_cksum();
    gz.write_all(long.as_bytes()).unwrap();
    gz.write_all(&vec![b'a'; name_len as usize]).unwrap();

    let mut file_header = tar::Header::new_gnu();
    file_header.set_path("short.txt").unwrap();
    file_header.set_size(0);
    file_header.set_mode(0o644);
    file_header.set_cksum();
    gz.write_all(file_header.as_bytes()).unwrap();
    gz.write_all(&[0u8; 1024]).unwrap();
    gz.finish().unwrap();

    let error = StdArchives.index(&archive).expect_err("a two-megabyte name is not a name");
    assert!(
        matches!(error, hyprforge_archive::ArchiveError::Damaged { .. }),
        "{error:?}"
    );
}

/// The ordinary case the limit must leave alone: a path longer than a
/// tar header's hundred bytes, which every real tool spells with exactly
/// the header the limit is about.
#[test]
fn a_tar_member_with_an_ordinarily_long_path_still_lists_and_extracts() {
    let dir = tempfile::tempdir().unwrap();
    let deep: String = (0..30).map(|i| format!("folder-{i:02}")).collect::<Vec<_>>().join("/");
    let tree = dir.path().join("payload");
    std::fs::create_dir_all(tree.join(&deep)).unwrap();
    std::fs::write(tree.join(&deep).join("leaf.txt"), b"leaf").unwrap();
    let archive = dir.path().join("deep.tar.xz");
    StdArchives
        .create(&archive, Format::Tar(Compression::Xz), &sources(&tree), &mut NoProgress)
        .unwrap();

    let member = format!("payload/{deep}/leaf.txt");
    assert!(member.len() > 300);
    assert!(StdArchives.index(&archive).unwrap().get(&member).is_some());
    let out = dir.path().join("out");
    let report = extract_all(&archive, &out, Collision::Overwrite);
    assert!(report.failed.is_empty(), "{:?}", report.failed);
    assert_eq!(std::fs::read(out.join(&member)).unwrap(), b"leaf");
}

/// A zip symlink's target is the member's *contents*, so its length is
/// whatever the archive says. Listing read it whole into the index.
#[test]
fn a_zip_symlink_with_an_enormous_target_lists_without_one() {
    let dir = tempfile::tempdir().unwrap();
    let archive = dir.path().join("links.zip");
    {
        let mut zip = zip::ZipWriter::new(std::fs::File::create(&archive).unwrap());
        let huge = "a".repeat(1024 * 1024);
        zip.add_symlink("huge-link", huge, zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.add_symlink("ok-link", "target.txt", zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.finish().unwrap();
    }

    let index = StdArchives.index(&archive).unwrap();
    assert_eq!(index.get("huge-link").and_then(|m| m.link_target.clone()), None);
    assert_eq!(
        index.get("ok-link").and_then(|m| m.link_target.clone()).as_deref(),
        Some("target.txt")
    );
}
