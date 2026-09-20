//! The same questions `File::MimeInfo` asks itself.
//!
//! `perl-file-mimeinfo` is the implementation `xdg-open` reaches for
//! when it is installed, so "does this agree with it" is the whole
//! measure of whether this crate can stand in for it. Its distribution
//! ships a test suite (`t/01_normal.t`, `t/02_magic.t`, `t/11mimeinfo.t`
//! in File-MimeInfo 0.37), and these are its cases, asked of this code.
//!
//! The *cases* are ported, not the files: the fixture database below is
//! built here, in the shapes the reference suite uses, rather than
//! vendoring someone else's test data into this repository. Where a
//! case pins something this crate deliberately does differently, it
//! says so and asserts the difference instead of quietly dropping the
//! case.
//!
//! Nothing here touches the machine's own database — every lookup is
//! against the fixture — so these run in tier 1 like any unit test. The
//! live comparison against the installed database and `gio` is
//! `live_mime_database.rs`.

use hyprforge_mime::lookup::{How, Lookup};
use hyprforge_mime::types::Types;
use std::path::{Path, PathBuf};

/// A miniature `mime` directory in the shape the reference suite's
/// `t/mime` has: the older `globs` file, a binary `magic`, a
/// `subclasses` file, and one type's XML for descriptions.
fn fixture() -> (tempfile::TempDir, Vec<PathBuf>) {
    let dir = tempfile::tempdir().unwrap();
    let mime = dir.path().join("mime");
    std::fs::create_dir_all(mime.join("text")).unwrap();

    std::fs::write(
        mime.join("globs"),
        "# This file is a test file -- it is incomplete !\n\
         application/x-perl:*.pl\n\
         application/x-gzip:*.gz\n\
         application/x-compressed-tar:*.tar.gz\n\
         text/x-install:INSTALL\n\
         text/x-makefile:[Mm]akefile\n\
         text/plain:*.asc\n\
         text/plain:*.txt\n\
         text/x-patch:*.patch\n\
         image/png:*.png\n",
    )
    .unwrap();

    // The reference suite's own magic fixture, rule for rule: a strong
    // perl rule at 80, a weak patch rule at 50, and a nested ELF rule
    // at 40.
    let mut magic = b"MIME-Magic\0\n".to_vec();
    let rule = |magic: &mut Vec<u8>, indent: u32, offset: usize, value: &[u8], range: usize| {
        if indent > 0 {
            magic.extend(indent.to_string().into_bytes());
        }
        magic.extend(format!(">{offset}=").into_bytes());
        magic.extend((value.len() as u16).to_be_bytes());
        magic.extend(value);
        if range > 0 {
            magic.extend(format!("+{range}").into_bytes());
        }
        magic.push(b'\n');
    };
    magic.extend(b"[80:application/x-perl]\n");
    rule(&mut magic, 0, 0, b"eval \"exec /usr/local/bin/perl", 0);
    rule(&mut magic, 0, 1, b"/bin/perl", 16);
    magic.extend(b"[50:text/x-patch]\n");
    rule(&mut magic, 0, 0, b"diff ", 0);
    rule(&mut magic, 0, 0, b"Index:", 0);
    magic.extend(b"[40:application/x-executable]\n");
    rule(&mut magic, 0, 0, b"\x7fELF", 0);
    rule(&mut magic, 1, 5, b"\x01\x01", 0);
    std::fs::write(mime.join("magic"), &magic).unwrap();

    std::fs::write(mime.join("subclasses"), "text/x-patch text/plain\n").unwrap();
    std::fs::write(
        mime.join("text").join("plain.xml"),
        "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n\
         <mime-type type=\"text/plain\">\n\
         <!--Created automatically by update-mime-database. DO NOT EDIT!-->\n\
         \x20 <comment>Plain Text</comment>\n\
         \x20 <comment xml:lang=\"nl\">Platte tekst</comment>\n\
         </mime-type>\n",
    )
    .unwrap();

    let dirs = vec![dir.path().to_path_buf()];
    (dir, dirs)
}

fn write(dir: &tempfile::TempDir, name: &str, contents: &[u8]) -> PathBuf {
    let path = dir.path().join(name);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(&path, contents).unwrap();
    path
}

/// `t/01_normal.t`'s filename cases, which are the ones people actually
/// hit: a double extension, an unusual case, a literal name, a glob
/// with a character class in it.
#[test]
fn filenames_are_typed_the_way_the_reference_types_them() {
    let (_dir, dirs) = fixture();
    let lookup = Lookup::load_from(&dirs);
    for (name, expected) in [
        ("script.pl", "application/x-perl"),
        ("script.old.pl", "application/x-perl"),
        ("script.PL", "application/x-perl"),
        ("script.tar.pl", "application/x-perl"),
        ("script.gz", "application/x-gzip"),
        ("script.tar.gz", "application/x-compressed-tar"),
        ("INSTALL", "text/x-install"),
        ("script.foo.bar.gz", "application/x-gzip"),
        ("script.foo.tar.gz", "application/x-compressed-tar"),
        ("makefile", "text/x-makefile"),
        ("./makefile", "text/x-makefile"),
    ] {
        assert_eq!(
            lookup.globs.type_of(Path::new(name)),
            Some(expected),
            "{name} should be {expected}"
        );
    }
}

/// `t/default/*`: what a file is when no rule of any kind recognises
/// it. The empty-file case is the one worth naming — the reference
/// calls it `text/plain`, and so does this, where `gio` says
/// `application/x-zerosize` and leaves it with no application at all.
#[test]
fn unrecognised_contents_fall_back_the_way_the_reference_does() {
    let (dir, dirs) = fixture();
    let lookup = Lookup::load_from(&dirs);
    for (name, contents, expected) in [
        ("default/binary_file", b"\x00\x01\x02\x03\x04binary".as_slice(), "application/octet-stream"),
        ("default/plain_text", b"Some plain text.\n".as_slice(), "text/plain"),
        ("default/empty_file", b"".as_slice(), "text/plain"),
        ("default/utf8_text", "cafés, naïveté, \u{4e2d}\u{6587}\n".as_bytes(), "text/plain"),
        // The reference calls this one `encoding_breakage`: bytes that
        // are not valid UTF-8. Note the `\x04` and `\x13` in the second
        // half — the first sixteen bytes contain no control character
        // but a carriage return, and on those alone this is *text*.
        // That is not a quirk of the sample: it is why both
        // implementations judge thirty-two bytes rather than a few.
        (
            "default/encoding_breakage",
            b"\xb7\x5b\x69\x9a\xb9\x0d\x25\x27\x95\x4b\x45\x2b\xa5\x5e\x69\xca\
              \xb1\x04\x46\xc0\xac\x91\x8e\x6a\x77\xe1\x13\x6c\x5b\xf6\x58\x7e"
                .as_slice(),
            "application/octet-stream",
        ),
    ] {
        let path = write(&dir, name, contents);
        assert_eq!(lookup.of_file(&path, true).mime, expected, "{name}");
    }
}

/// `t/02_magic.t`: contents decide when the name is wrong or absent.
/// The perl rule is priority 80, so it beats a `.txt` name — the file
/// in the reference suite is literally called
/// `application_x-perl.txt`.
#[test]
fn contents_are_typed_the_way_the_reference_types_them() {
    let (dir, dirs) = fixture();
    let lookup = Lookup::load_from(&dirs);

    let perl_named_txt = write(&dir, "magic/application_x-perl.txt", b"#!/usr/bin/perl\n\nprint \"Hello world\"\n");
    let found = lookup.of_file(&perl_named_txt, true);
    assert_eq!(found.mime, "application/x-perl", "a strong rule beats the filename");
    assert_eq!(found.how, How::StrongMagic);

    // A weak rule (50) on a file with no name to go on.
    let patch = write(&dir, "magic/text_x-patch", b"Index: update-mime-database.c\n==========\n");
    assert_eq!(lookup.of_file(&patch, true).mime, "text/x-patch");

    // Nested rules: ELF, and specifically the 32-bit little-endian
    // flavour the child rule asks for.
    let elf = write(&dir, "magic/application_x-executable", b"\x7fELF\x00\x01\x01\x00padding");
    assert_eq!(lookup.of_file(&elf, true).mime, "application/x-executable");
    let not_elf = write(&dir, "magic/nearly", b"\x7fELF\x00\x09\x09\x00padding");
    assert_ne!(
        lookup.of_file(&not_elf, true).mime,
        "application/x-executable",
        "the child rule did not match, so neither did the parent"
    );
}

/// `t/01_normal.t`'s inode cases: a directory and a symlink are what
/// they are, whatever they are called.
#[test]
fn inode_types_come_first_as_they_do_in_the_reference() {
    let (dir, dirs) = fixture();
    let lookup = Lookup::load_from(&dirs);

    let folder = dir.path().join("a-directory.txt");
    std::fs::create_dir(&folder).unwrap();
    assert_eq!(lookup.of_file(&folder, true).mime, "inode/directory");

    let target = write(&dir, "target.pl", b"#!/usr/bin/perl\n");
    let link = dir.path().join("symlink");
    std::os::unix::fs::symlink(&target, &link).unwrap();
    assert_eq!(lookup.of_file(&link, false).mime, "inode/symlink", "not dereferenced");
    assert_eq!(
        lookup.of_file(&link, true).mime,
        "application/x-perl",
        "dereferenced, it is what it points at"
    );
}

/// `describe`, including the language case the reference tests with
/// Dutch.
#[test]
fn descriptions_are_read_and_translated_as_the_reference_does() {
    let (_dir, dirs) = fixture();
    let mut types = Types::load_from(&dirs);
    assert_eq!(types.describe_from(&dirs, "text/plain", None), Some("Plain Text"));
    assert_eq!(types.describe_from(&dirs, "text/plain", Some("nl")), Some("Plain Text"), "already cached");

    let mut fresh = Types::load_from(&dirs);
    assert_eq!(fresh.describe_from(&dirs, "text/plain", Some("nl")), Some("Platte tekst"));
    assert_eq!(fresh.describe_from(&dirs, "image/png", None), None, "no XML for it in the fixture");
}

/// A type with no rules of its own is still a kind of its parent —
/// `t/mime/subclasses` exists in the reference fixture for this.
#[test]
fn subclasses_are_read_from_the_database() {
    let (_dir, dirs) = fixture();
    let lookup = Lookup::load_from(&dirs);
    assert!(lookup.is_subclass_of("text/x-patch", "text/plain"));
    assert!(lookup.is_subclass_of("application/x-perl", "application/octet-stream"));
    assert!(!lookup.is_subclass_of("application/x-gzip", "text/plain"));
}

/// The one case where this deliberately differs, asserted rather than
/// dropped: the reference has no answer at all for a file that is not
/// there, and this answers from the name when the name says something.
/// A file manager asking about a listing it just read wants the type,
/// not silence.
#[test]
fn a_missing_file_is_answered_from_its_name_where_the_reference_gives_up() {
    let (dir, dirs) = fixture();
    let lookup = Lookup::load_from(&dirs);
    let missing = dir.path().join("not-here.pl");
    assert_eq!(lookup.of_file(&missing, true).mime, "application/x-perl");

    let nameless = dir.path().join("not-here");
    assert_eq!(
        lookup.of_file(&nameless, true).mime,
        "application/octet-stream",
        "with nothing to go on, the bytes fallback — the reference says nothing here"
    );
}
