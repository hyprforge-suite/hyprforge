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
