//! Does anything else on this machine agree with what this crate writes?
//!
//! The round-trip tests prove this crate can read what it wrote, which
//! is necessary and is not the interesting claim: a writer that emits
//! something only its own reader accepts passes every one of them. These
//! hand an archive to `unzip`, `tar` and `7z` — three implementations
//! nobody here wrote — and read back archives those tools produced.
//!
//! Gated, and gated on the tool each one actually asks. `#[ignore]` so
//! they only run when `check.sh` asks for them, and a tool that is not
//! installed prints a `HYPRFORGE-SKIP:` marker rather than returning
//! quietly — the rule CLAUDE.md states about a test that cannot run
//! never reporting as one that passed.
//!
//! Read-only with respect to the machine: everything happens inside a
//! temporary directory, and no test starts a daemon or touches anything
//! the person running it owns.

use hyprforge_archive::backend::{ArchiveBackend, Collision, ExtractRequest, NoProgress};
use hyprforge_archive::{Compression, Format, Source, StdArchives};
use std::path::{Path, PathBuf};
use std::process::Command;

/// Runs `tool` with `args` in `dir`, or `None` if the tool is not
/// installed.
///
/// Bounded, because nothing in this suite may wait on another process
/// without one — and an archive tool handed a file it does not
/// understand is exactly the sort of thing that sits waiting for input
/// at a prompt nobody can see.
fn run(dir: &Path, tool: &str, args: &[&str]) -> Option<std::process::Output> {
    if Command::new("sh")
        .arg("-c")
        .arg(format!("command -v {tool}"))
        .output()
        .ok()?
        .status
        .success()
    {
        // `</dev/null` through the stdin handle rather than the shell:
        // a tool that would prompt gets end-of-file instead and exits.
        Command::new(tool)
            .args(args)
            .current_dir(dir)
            .stdin(std::process::Stdio::null())
            .output()
            .ok()
    } else {
        None
    }
}

fn missing(tool: &str) {
    eprintln!("HYPRFORGE-SKIP: {tool} isn't installed to check against");
}

/// The tree every test packs, and the two files it then looks for.
fn sample(dir: &Path) -> PathBuf {
    let tree = dir.join("payload");
    std::fs::create_dir_all(tree.join("docs")).unwrap();
    std::fs::write(tree.join("readme.md"), b"# hello").unwrap();
    std::fs::write(tree.join("docs/guide.txt"), b"a guide, of sorts").unwrap();
    tree
}

fn ours(dir: &Path, name: &str, format: Format) -> PathBuf {
    let tree = sample(dir);
    let archive = dir.join(name);
    StdArchives
        .create(
            &archive,
            format,
            &[Source {
                path: tree,
                as_member: "payload".to_string(),
            }],
            &mut NoProgress,
        )
        .unwrap();
    archive
}

/// What this crate says is inside an archive, sorted — for comparing
/// against a tool's own listing.
fn our_members(archive: &Path) -> Vec<String> {
    let index = StdArchives.index(archive).unwrap();
    let mut names: Vec<String> = index
        .members()
        .iter()
        .filter(|m| !m.is_dir)
        .map(|m| m.path.clone())
        .collect();
    names.sort();
    names
}

// --- what we write, read by somebody else ---------------------------

#[test]
#[ignore = "asks the archive tools installed on this machine"]
fn a_zip_this_crate_wrote_passes_unzips_own_integrity_check() {
    let dir = tempfile::tempdir().unwrap();
    let archive = ours(dir.path(), "ours.zip", Format::Zip);

    let Some(output) = run(dir.path(), "unzip", &["-t", "ours.zip"]) else {
        return missing("unzip");
    };
    assert!(
        output.status.success(),
        "unzip rejected an archive this crate wrote:\n{}",
        String::from_utf8_lossy(&output.stdout)
    );
    let listing = String::from_utf8_lossy(&output.stdout).into_owned();
    assert!(listing.contains("payload/docs/guide.txt"), "{listing}");
    let _ = archive;
}

#[test]
#[ignore = "asks the archive tools installed on this machine"]
fn a_tarball_this_crate_wrote_is_listed_by_tar_with_the_same_members() {
    for (name, format) in [
        ("ours.tar", Format::Tar(Compression::None)),
        ("ours.tar.gz", Format::Tar(Compression::Gzip)),
        ("ours.tar.xz", Format::Tar(Compression::Xz)),
        ("ours.tar.zst", Format::Tar(Compression::Zstd)),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let archive = ours(dir.path(), name, format);

        // `-a` so tar picks the decompressor from the name, which is
        // also a check that this crate named the file correctly.
        let Some(output) = run(dir.path(), "tar", &["-taf", name]) else {
            return missing("tar");
        };
        assert!(
            output.status.success(),
            "tar could not read the {name} this crate wrote:\n{}",
            String::from_utf8_lossy(&output.stderr)
        );

        let mut theirs: Vec<String> = String::from_utf8_lossy(&output.stdout)
            .lines()
            .filter(|line| !line.ends_with('/'))
            .map(|line| line.trim_start_matches("./").to_string())
            .collect();
        theirs.sort();
        assert_eq!(theirs, our_members(&archive), "tar and this crate disagree about {name}");
    }
}

#[test]
#[ignore = "asks the archive tools installed on this machine"]
fn a_7z_this_crate_wrote_passes_7zs_own_integrity_check() {
    let dir = tempfile::tempdir().unwrap();
    ours(dir.path(), "ours.7z", Format::SevenZ);

    let Some(output) = run(dir.path(), "7z", &["t", "ours.7z"]) else {
        return missing("7z");
    };
    assert!(
        output.status.success(),
        "7z rejected an archive this crate wrote:\n{}",
        String::from_utf8_lossy(&output.stdout)
    );
}

// --- what somebody else writes, read by us --------------------------

#[test]
#[ignore = "asks the archive tools installed on this machine"]
fn an_archive_the_system_tool_wrote_reads_back_with_its_contents_intact() {
    for (tool, args, name) in [
        ("zip", vec!["-qr", "theirs.zip", "payload"], "theirs.zip"),
        ("tar", vec!["-czf", "theirs.tar.gz", "payload"], "theirs.tar.gz"),
        ("7z", vec!["a", "-bso0", "theirs.7z", "payload"], "theirs.7z"),
    ] {
        let dir = tempfile::tempdir().unwrap();
        sample(dir.path());

        let Some(output) = run(dir.path(), tool, &args) else {
            missing(tool);
            continue;
        };
        if !output.status.success() {
            panic!(
                "{tool} could not write its own archive:\n{}",
                String::from_utf8_lossy(&output.stderr)
            );
        }

        let archive = dir.path().join(name);
        assert_eq!(
            our_members(&archive),
            ["payload/docs/guide.txt", "payload/readme.md"],
            "this crate read {name} differently from what {tool} put in it"
        );
        assert_eq!(
            StdArchives.read_member(&archive, "payload/docs/guide.txt").unwrap(),
            b"a guide, of sorts",
            "a member written by {tool} came back changed"
        );

        // And the whole thing, out to disk.
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
        assert!(report.failed.is_empty(), "{tool}: {:?}", report.failed);
        assert_eq!(std::fs::read(out.join("payload/readme.md")).unwrap(), b"# hello");
    }
}

/// A symlink is the member type most likely to differ between
/// implementations, and the one with a security consequence if it is
/// read wrongly — see `extract`'s guard.
#[test]
#[ignore = "asks the archive tools installed on this machine"]
fn a_symlink_inside_a_tarball_someone_else_made_is_recognised_as_one() {
    let dir = tempfile::tempdir().unwrap();
    let tree = sample(dir.path());
    std::os::unix::fs::symlink("readme.md", tree.join("link-to-readme")).unwrap();

    let Some(output) = run(dir.path(), "tar", &["-cf", "theirs.tar", "payload"]) else {
        return missing("tar");
    };
    assert!(output.status.success());

    let index = StdArchives.index(&dir.path().join("theirs.tar")).unwrap();
    let link = index
        .get("payload/link-to-readme")
        .expect("the symlink member is missing from the listing");
    assert_eq!(
        link.link_target.as_deref(),
        Some("readme.md"),
        "a symlink read as an ordinary file would be extracted as one"
    );
}
