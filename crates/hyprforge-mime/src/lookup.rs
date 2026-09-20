//! Putting the name and the contents together: what *is* this file.
//!
//! # The order, and why it is this one
//!
//! ```text
//! 1. Is it a file at all?      inode/directory, inode/socket, ...
//! 2. Strong magic (>= 80)      \x89PNG is a PNG whatever it is called
//! 3. The name                  part.3mf is a model, though it is a zip
//! 4. Weaker magic (< 80)       a last look at the contents
//! 5. Fall back                 empty / text / bytes
//! ```
//!
//! Every step of that is load bearing, and the two in the middle are the
//! ones implementations get wrong in opposite directions:
//!
//! - **Contents only** — what `file --mime-type` does, and what
//!   `xdg-open` falls back to on a desktop it does not recognise — calls
//!   a `.stl` `application/octet-stream`, a `.3mf` `application/zip` and
//!   a `.blend` `application/zstd`. All true about the bytes; none has a
//!   default application, so the file opens in a web browser.
//! - **Name only** believes a `.txt` that is really a JPEG, and has
//!   nothing at all to say about a file called `download`.
//!
//! Strong magic first is what settles the disagreement: a rule the
//! database marks 80 or above is one nobody should doubt (a PNG header,
//! a PDF header), so it beats a filename. Everything below that yields
//! to the name, because a name is a statement of intent and a weak magic
//! rule is a guess.
//!
//! This is the same order `File::MimeInfo::Magic` uses, which is the
//! order `xdg-open` would follow if the perl package happened to be
//! installed — so a machine with this crate on it answers the same
//! question the same way, rather than a third way.

use crate::globs::Globs;
use crate::magic::Magic;
use crate::types::{self, Types};
use std::path::Path;

/// How the answer was reached. Kept because the difference matters to a
/// caller: a type from a filename is a statement about intent, one from
/// contents is a statement about bytes, and the fallbacks are neither.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum How {
    /// It is not an ordinary file.
    Inode,
    /// A magic rule the database marks 80 or above.
    StrongMagic,
    /// A filename rule.
    Name,
    /// A weaker magic rule.
    Magic,
    /// Nothing recognised it: empty, text, or bytes.
    Fallback,
}

/// What a file turned out to be.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    pub mime: String,
    pub how: How,
}

/// The whole database: names, contents, and the type graph.
#[derive(Debug, Clone, Default)]
pub struct Lookup {
    pub globs: Globs,
    pub magic: Magic,
    pub types: Types,
}

/// Nothing recognised it and it reads as text.
pub const TEXT: &str = "text/plain";
/// Nothing recognised it and it does not.
pub const BYTES: &str = "application/octet-stream";

/// A magic rule at or above this priority beats the filename. The
/// database's own scale, and the threshold every implementation uses.
const STRONG: u32 = 80;

impl Lookup {
    /// Reads every part of the database from the given data directories.
    pub fn load_from(data_dirs: &[std::path::PathBuf]) -> Lookup {
        Lookup {
            globs: Globs::load_from(data_dirs),
            magic: Magic::load_from(data_dirs),
            types: Types::load_from(data_dirs),
        }
    }

    /// What this file is, reading it if the name does not settle it.
    ///
    /// `follow` decides whether a symlink is reported as itself or as
    /// what it points at.
    pub fn of_file(&self, path: &Path, follow: bool) -> Found {
        if let Some(inode) = types::inode_type(path, follow) {
            return Found { mime: inode.to_string(), how: How::Inode };
        }
        match std::fs::read(path).ok().map(|data| self.of_data_and_name(&data, Some(path))) {
            Some(found) => found,
            // Unreadable — no permission, or it went away between the
            // listing and the question. The name is all there is, and
            // saying "bytes" about a file nobody could read would be a
            // claim rather than an answer.
            None => match self.globs.type_of(path) {
                Some(mime) => Found { mime: mime.to_string(), how: How::Name },
                None => Found { mime: BYTES.to_string(), how: How::Fallback },
            },
        }
    }

    /// The same question for bytes already in hand, with the name if
    /// there is one — a paste, a download, or standard input.
    pub fn of_data_and_name(&self, data: &[u8], path: Option<&Path>) -> Found {
        if let Some(found) = self.magic.of_data(data).filter(|m| m.priority >= STRONG) {
            return Found { mime: found.mime, how: How::StrongMagic };
        }
        if let Some(mime) = path.and_then(|path| self.globs.type_of(path)) {
            return Found { mime: mime.to_string(), how: How::Name };
        }
        if let Some(found) = self.magic.of_data(data) {
            return Found { mime: found.mime, how: How::Magic };
        }
        Found { mime: fallback(data).to_string(), how: How::Fallback }
    }

    /// Every type that matches, best first — `mimetype --all`.
    ///
    /// The name's matches and the contents' matches both appear, in the
    /// order above, without repeats. Useful when a caller would rather
    /// see the ambiguity than have it resolved for them.
    pub fn all_of_file(&self, path: &Path, follow: bool) -> Vec<String> {
        if let Some(inode) = types::inode_type(path, follow) {
            return vec![inode.to_string()];
        }
        let data = std::fs::read(path).unwrap_or_default();
        let mut found: Vec<String> = Vec::new();
        let magic = self.magic.all_of_data(&data);
        for m in magic.iter().filter(|m| m.priority >= STRONG) {
            push_once(&mut found, m.mime.clone());
        }
        for mime in self.globs.all_matches(path) {
            push_once(&mut found, mime.to_string());
        }
        for m in magic.iter().filter(|m| m.priority < STRONG) {
            push_once(&mut found, m.mime.clone());
        }
        if found.is_empty() {
            found.push(fallback(&data).to_string());
        }
        found
    }

    /// Whether an application registered for `parent` can be expected to
    /// open a `mime` — the subclass question, asked the way a caller
    /// means it.
    pub fn is_subclass_of(&self, mime: &str, parent: &str) -> bool {
        self.types.is_subclass_of(mime, parent)
    }
}

/// Keeps the first mention of a type and drops later ones.
fn push_once(found: &mut Vec<String>, mime: String) {
    if !found.contains(&mime) {
        found.push(mime);
    }
}

/// What to call something nothing recognised.
///
/// An empty file is `text/plain`, which surprises people who expect
/// `application/x-zerosize`: that name exists, and `gio` uses it, but
/// nothing is registered to open it, so a desktop that answers with it
/// has turned an empty `.svg` into a file with no application. This
/// follows `File::MimeInfo` instead, which calls an empty file text —
/// and an empty file with a known extension never reaches here at all,
/// because the name is consulted first.
///
/// Otherwise the text test, because a shell script, a config file and a
/// README with no extension are all openable by anything that takes
/// `text/plain`, and calling them `application/octet-stream` closes
/// that door for no reason.
pub fn fallback(data: &[u8]) -> &'static str {
    if data.is_empty() || looks_like_text(data) {
        return TEXT;
    }
    BYTES
}

/// Whether these bytes read as text.
///
/// A control character that is not whitespace means binary. Valid UTF-8
/// is not required — a Latin-1 file is still text, and demanding UTF-8
/// would call it bytes.
///
/// Only the first 32 bytes, which is what `File::MimeInfo` looks at.
/// It matters more than it sounds: a long text file with one stray
/// control byte in the middle stays text, and a binary file whose first
/// 32 bytes happen to be printable is called text by both
/// implementations. Agreeing with the tool a desktop would otherwise
/// have used is worth more here than being cleverer than it.
fn looks_like_text(data: &[u8]) -> bool {
    data.iter().take(32).all(|byte| match byte {
        // Perl's `\s`: space, tab, newline, carriage return, form feed,
        // vertical tab. Escape is *not* whitespace and does mean binary.
        b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c => true,
        // C0 controls and DEL. Everything at 0x80 and above is left
        // alone: that is where non-ASCII text lives.
        0x00..=0x1f | 0x7f => false,
        _ => true,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lookup() -> Lookup {
        let mut magic = Vec::new();
        magic.extend(b"MIME-Magic\0\n");
        // A strong rule: a PNG header is a PNG whatever the file is
        // called.
        magic.extend(b"[90:image/png]\n>0=");
        magic.extend(4u16.to_be_bytes());
        magic.extend(b"\x89PNG\n");
        // A weak rule: "solid " opens an ASCII STL, and also plenty of
        // ordinary sentences.
        magic.extend(b"[50:model/stl]\n>0=");
        magic.extend(6u16.to_be_bytes());
        magic.extend(b"solid \n");
        let mut types = Types::default();
        types.add_subclasses("model/3mf application/zip\n");
        Lookup {
            globs: Globs::parse("50:model/3mf:*.3mf\n50:text/plain:*.txt\n50:image/jpeg:*.jpg\n"),
            magic: Magic::parse(&magic),
            types,
        }
    }

    fn write(dir: &tempfile::TempDir, name: &str, contents: &[u8]) -> std::path::PathBuf {
        let path = dir.path().join(name);
        std::fs::write(&path, contents).unwrap();
        path
    }

    /// The case the whole crate exists for: the name knows, and the
    /// contents only say "it is a zip".
    #[test]
    fn a_name_beats_a_weak_guess_about_the_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let found = lookup().of_file(&write(&dir, "part.3mf", b"PK\x03\x04zip data"), true);
        assert_eq!(found, Found { mime: "model/3mf".to_string(), how: How::Name });
    }

    /// And the other way: a strong magic rule beats a name, because a
    /// file called `notes.txt` that begins `\x89PNG` is a PNG.
    #[test]
    fn strong_magic_beats_a_name_that_lies() {
        let dir = tempfile::tempdir().unwrap();
        let found = lookup().of_file(&write(&dir, "notes.txt", b"\x89PNG\r\n\x1a\n"), true);
        assert_eq!(found, Found { mime: "image/png".to_string(), how: How::StrongMagic });
    }

    /// A weak rule does not: "solid " starts an ASCII STL and also
    /// plenty of English sentences, so the name is the better evidence.
    #[test]
    fn a_weak_rule_yields_to_the_name() {
        let dir = tempfile::tempdir().unwrap();
        let found = lookup().of_file(&write(&dir, "notes.txt", b"solid ground underfoot"), true);
        assert_eq!(found.mime, "text/plain");
        assert_eq!(found.how, How::Name);
    }

    /// With no name to go on, the weak rule is still better than
    /// nothing — this is the file called `download`.
    #[test]
    fn a_file_with_no_useful_name_falls_back_to_its_contents() {
        let dir = tempfile::tempdir().unwrap();
        let found = lookup().of_file(&write(&dir, "download", b"solid model\n"), true);
        assert_eq!(found, Found { mime: "model/stl".to_string(), how: How::Magic });
    }

    #[test]
    fn something_that_is_not_a_file_is_answered_first() {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("photos.jpg");
        std::fs::create_dir(&folder).unwrap();
        assert_eq!(
            lookup().of_file(&folder, true),
            Found { mime: "inode/directory".to_string(), how: How::Inode },
            "a folder called photos.jpg is still a folder"
        );
    }

    #[test]
    fn nothing_recognised_is_text_or_bytes() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            lookup().of_file(&write(&dir, "nothing", b""), true).mime,
            TEXT,
            "an empty file is text, not a type nothing can open"
        );
        assert_eq!(lookup().of_file(&write(&dir, "readme", b"hello there\n"), true).mime, TEXT);
        assert_eq!(lookup().of_file(&write(&dir, "blob", b"\x00\x01\x02binary"), true).mime, BYTES);
    }

    /// An empty file with a name the database knows keeps that name's
    /// answer — the step order puts globs before the fallback. `gio`
    /// says `application/x-zerosize` for these and so has nothing to
    /// open them with.
    #[test]
    fn an_empty_file_still_has_the_type_its_name_gives() {
        let dir = tempfile::tempdir().unwrap();
        let found = lookup().of_file(&write(&dir, "empty.jpg", b""), true);
        assert_eq!(found, Found { mime: "image/jpeg".to_string(), how: How::Name });
    }

    #[test]
    fn text_with_accents_is_still_text() {
        assert!(looks_like_text("café — naïve\n".as_bytes()));
        assert!(looks_like_text(&[0xe9, 0xe8, b'\n']), "latin-1, not utf-8, still text");
        assert!(!looks_like_text(b"\x00\x01"));
        assert!(!looks_like_text(b"\x1b[0;31mred\x1b[0m"), "escape means binary, not whitespace");
    }

    /// Only the first 32 bytes are judged, which is what the tool this
    /// agrees with does.
    #[test]
    fn a_stray_control_byte_later_on_does_not_make_a_file_binary() {
        let mut data = b"a text file, for the first thirty-two bytes at least".to_vec();
        data.push(0x00);
        assert!(looks_like_text(&data));
    }

    /// A file that cannot be read is not a file full of bytes: the name
    /// is all there is, and it is used.
    #[test]
    fn an_unreadable_file_is_typed_by_its_name() {
        let dir = tempfile::tempdir().unwrap();
        let path = write(&dir, "secret.3mf", b"PK\x03\x04");
        let mut perms = std::fs::metadata(&path).unwrap().permissions();
        std::os::unix::fs::PermissionsExt::set_mode(&mut perms, 0o000);
        std::fs::set_permissions(&path, perms).unwrap();
        // Running as root defeats the point of the test, and CI may.
        if std::fs::read(&path).is_ok() {
            eprintln!("HYPRFORGE-SKIP: this user can read a mode 000 file (root?)");
            return;
        }
        assert_eq!(lookup().of_file(&path, true).mime, "model/3mf");
    }

    #[test]
    fn every_match_can_be_listed_rather_than_resolved() {
        let dir = tempfile::tempdir().unwrap();
        let all = lookup().all_of_file(&write(&dir, "part.3mf", b"solid "), true);
        assert_eq!(all, ["model/3mf", "model/stl"], "the name first, then the weak rule");
    }

    #[test]
    fn data_without_a_name_is_answered_too() {
        let found = lookup().of_data_and_name(b"\x89PNG\r\n", None);
        assert_eq!(found.mime, "image/png");
        assert_eq!(lookup().of_data_and_name(b"plain words", None).mime, TEXT);
    }
}
