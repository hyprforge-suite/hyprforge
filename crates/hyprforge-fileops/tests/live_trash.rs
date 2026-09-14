//! Does this crate agree with the trash directory somebody else already
//! wrote?
//!
//! Every other test here builds its own trash in a `tempfile` directory
//! and is therefore only ever checking this code against itself. The
//! `.trashinfo` format is a claim about *other implementations* — the
//! entries in a real trash were written by GNOME Files, gvfs, or
//! whatever else the user has run — and the compiler cannot see any of
//! it. That is what this tier is for.
//!
//! **Read-only, absolutely.** It runs against the real trash of a person
//! who is using this machine, and their trash is the one place on the
//! system that exists specifically to hold things they have not decided
//! to lose yet. Nothing here trashes, restores, or removes anything;
//! writing is covered by the tempdir tests.

use std::path::Path;

/// A trash with nothing in it proves nothing, and must say so rather
/// than passing silently — libtest has no skipped state, so a test that
/// returns early prints `ok` exactly like one that checked something.
/// `check.sh` greps for this marker.
fn skip(reason: &str) {
    eprintln!("HYPRFORGE-SKIP: {reason}");
}

#[test]
#[ignore = "reads the real trash of whoever is running this"]
fn every_entry_another_implementation_wrote_is_one_this_crate_can_read() {
    let home = hyprforge_fileops::trash::home_trash_dir();
    if !home.join("info").is_dir() {
        skip("no home trash directory on this machine");
        return;
    }

    let info_files = match std::fs::read_dir(home.join("info")) {
        Ok(entries) => entries
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|e| e == "trashinfo"))
            .collect::<Vec<_>>(),
        Err(e) => {
            skip(&format!("the home trash could not be listed ({e})"));
            return;
        }
    };

    if info_files.is_empty() {
        skip("the trash is empty, so there is nothing anybody else wrote to check against");
        return;
    }

    let items = hyprforge_fileops::trash::list(&home)
        .expect("a readable trash directory must list, not error");

    // The real assertion: *every* file on disk became an item. Asserting
    // only that the call returned `Ok` would pass on a reader that
    // silently skipped everything it could not parse, which is the exact
    // failure this crate is written not to have.
    assert_eq!(
        items.len(),
        info_files.len(),
        "{} .trashinfo files on disk but only {} parsed — a reader that drops what it \
         cannot understand is the mistake this crate exists not to make",
        info_files.len(),
        items.len()
    );

    for item in &items {
        assert!(
            Path::new(&item.original_path).is_absolute(),
            "the home trash records absolute paths; got {:?}",
            item.original_path
        );
        // Percent-decoding actually happened. A path still carrying a
        // literal `%20` is the tell that it did not, and that is the
        // single most likely way this reader is subtly wrong — the files
        // this machine's own trash holds include one called
        // "Monkey Around.mp4", stored as "Monkey%20Around.mp4".
        assert!(
            !item.original_path.to_string_lossy().contains("%20"),
            "{:?} still carries an encoded space, so it was never decoded",
            item.original_path
        );
        assert!(
            !item.deleted_at.is_empty(),
            "every entry carries a deletion date; {:?} did not",
            item.original_path
        );
    }

    eprintln!(
        "checked {} entries written by another implementation",
        items.len()
    );
}
