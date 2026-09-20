//! The installed shared MIME database, as it actually is.
//!
//! Everything in the unit tests is a claim about a fixture this code
//! wrote. These are the claims about *somebody else's* data: that the
//! files are where the spec says, in the format this parses, and that
//! the answers they give are the ones the desktop gives. The compiler
//! cannot see any of that, and neither can a fixture.
//!
//! Read-only, like every live tier here: it reads `globs2`, the desktop
//! entries and `mimeapps.list`, and writes nothing — changing a person's
//! default application while they are using the machine is exactly the
//! kind of thing the live-tier rule exists to prevent.
//!
//! ```text
//! cargo test -p hyprforge-mime --test live_mime_database -- --ignored --nocapture
//! ```

use hyprforge_mime::MimeDb;
use std::path::Path;

const SKIP_MARKER: &str = "HYPRFORGE-SKIP:";

/// The database this tier is about. Absent — a machine without
/// `shared-mime-info` — is a skip, not a pass: see CLAUDE.md on a test
/// that cannot run reporting as one that did.
fn installed_db() -> Option<MimeDb> {
    let db = MimeDb::load();
    if !db.knows_types() {
        eprintln!(
            "{SKIP_MARKER} no shared MIME database found (install shared-mime-info) — \
             nothing to ask about"
        );
        return None;
    }
    Some(db)
}

/// The bug this crate was written for, asked of the real database: the
/// types that content sniffing gets wrong are the ones `globs2` gets
/// right. `file --mime-type` calls these octet-stream, zip and zstd.
#[test]
#[ignore]
fn the_types_content_sniffing_gets_wrong_are_right_by_name() {
    let Some(db) = installed_db() else { return };
    for (name, expected) in [
        ("part.stl", "model/stl"),
        ("part.3mf", "model/3mf"),
        ("scene.blend", "application/x-blender"),
        ("notes.md", "text/markdown"),
        ("sheet.csv", "text/csv"),
    ] {
        assert_eq!(
            db.type_of(Path::new(name)),
            Some(expected),
            "{name} should be {expected} by its name alone"
        );
    }
}

/// The parsers agree with the system's own tools. `gio` reads the same
/// database through an implementation this project did not write, which
/// is the point — if these two disagree, this crate is the one that is
/// wrong.
///
/// The files have plausible content, because an *empty* file is the one
/// case where the two deliberately differ — see
/// [`an_empty_file_is_typed_by_its_name_where_the_system_gives_up`].
#[test]
#[ignore]
fn the_type_matches_what_gio_says() {
    let Some(db) = installed_db() else { return };
    let dir = tempfile::tempdir().unwrap();
    for (name, content) in [
        ("part.stl", b"solid test\nfacet normal 0 0 0\n".as_slice()),
        ("part.3mf", b"PK\x03\x04junkjunkjunk".as_slice()),
        ("page.html", b"<html><body>hi</body></html>".as_slice()),
        ("photo.png", b"\x89PNG\r\n\x1a\n0000".as_slice()),
    ] {
        let path = dir.path().join(name);
        std::fs::write(&path, content).unwrap();
        let Some(theirs) = gio_type(&path) else {
            eprintln!("{SKIP_MARKER} gio is not installed — nothing to compare against");
            return;
        };
        assert_eq!(
            db.type_of(&path).map(str::to_string),
            Some(theirs.clone()),
            "{name}: gio says {theirs}"
        );
    }
}

/// An empty file is a real case — `contours.svg` in a real Downloads
/// folder here was one — and this is where reading the name rather than
/// the bytes is deliberately *better* than what the system's own tools
/// do.
///
/// `gio` calls an empty file `application/x-zerosize` whatever it is
/// called, and `xdg-mime` calls it `inode/x-empty`; neither has an
/// application, so both send it to a browser. An empty `.svg` is still
/// an SVG, and it should open in whatever opens SVGs.
#[test]
#[ignore]
fn an_empty_file_is_typed_by_its_name_where_the_system_gives_up() {
    let Some(db) = installed_db() else { return };
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("empty.svg");
    std::fs::write(&path, b"").unwrap();
    assert_eq!(db.type_of(&path), Some("image/svg+xml"));
    if let Some(theirs) = gio_type(&path) {
        assert_ne!(theirs, "image/svg+xml", "if gio learns this, the doc above needs revisiting");
    }
}

/// Whatever opens HTML on this machine, there is one, it is installed,
/// and it is offered. A machine with no browser at all would be odd
/// enough to say so rather than to assert through.
#[test]
#[ignore]
fn a_common_type_has_an_installed_application_to_open_it() {
    let Some(db) = installed_db() else { return };
    let apps = db.apps_for("text/html");
    if apps.is_empty() {
        eprintln!("{SKIP_MARKER} nothing registered for text/html on this machine");
        return;
    }
    assert!(apps.iter().all(|app| app.installed), "only installed applications are offered");
    assert!(apps.iter().all(|app| !app.name.is_empty()), "each one has something to show");
    assert!(
        apps.iter().all(|app| app.id.ends_with(".desktop")),
        "the identity is the entry's file name, which is what mimeapps.list uses"
    );
}

/// The reading half of the thing the file manager is about to offer:
/// whatever the user's own choice is, it is found and it is described.
#[test]
#[ignore]
fn the_users_own_default_is_read_back() {
    let Some(db) = installed_db() else { return };
    let Some(default) = db.default_for("text/html") else {
        eprintln!("{SKIP_MARKER} no default recorded for text/html on this machine");
        return;
    };
    eprintln!("text/html opens in {} ({})", default.name, default.id);
    assert!(!default.name.is_empty());
}

/// `gio info`'s answer, or `None` when gio is not installed.
///
/// Bounded, like every other call to another process in this project:
/// `gio` can stall on a filesystem that is not answering, and a test
/// that hangs is worse than one that fails.
fn gio_type(path: &Path) -> Option<String> {
    let out = hyprforge_process::output(
        std::process::Command::new("gio")
            .args(["info", "-a", "standard::content-type"])
            .arg(path),
        hyprforge_process::TIMEOUT,
    )
    .ok()?;
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .find_map(|line| line.trim().strip_prefix("standard::content-type:"))
        .map(|value| value.trim().to_string())
}
