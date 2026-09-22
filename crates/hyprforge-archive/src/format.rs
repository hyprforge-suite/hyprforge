//! What kind of archive a file is.
//!
//! Two questions that look like one. The *container* — zip, tar, 7z, or
//! a bare compressed stream — is decided by reading the first few bytes,
//! because a name is a claim and the bytes are the thing. The
//! *compression* a tar wears is decided the same way, from the outside
//! in.
//!
//! The one place a name still gets a say is which files this crate
//! offers to open at all ([`Format::plausible_by_name`]): a browser
//! cannot read the first six bytes of every entry in a directory to
//! decide which rows are archives — that is the same "one open per file
//! during a listing" cost `EntryKind::classify` refuses to pay. So the
//! name filters, and the bytes decide.
//!
//! # The `.gz` that is not a `.tar.gz`
//!
//! `dump.sql.gz` and `linux-6.6.tar.gz` have identical magic bytes, and
//! the extension is not reliable either — plenty of tarballs are named
//! `.gz` and plenty of single files are named `.tgz` by accident. The
//! only honest way to tell is to decompress the first 512 bytes and look
//! for tar's `ustar` magic at offset 257, which is what [`sniff`] does.
//! It costs one block of decompression, once, and it is the difference
//! between opening a tarball and showing someone a single member called
//! `linux-6.6.tar`.

use std::io::Read;
use std::path::Path;

/// The stream compression wrapped around a tar, or standing on its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Compression {
    None,
    Gzip,
    Bzip2,
    Xz,
    Zstd,
}

impl Compression {
    /// The extension this compression is normally spelled with, for
    /// naming a file being created.
    pub fn extension(self) -> &'static str {
        match self {
            Compression::None => "",
            Compression::Gzip => "gz",
            Compression::Bzip2 => "bz2",
            Compression::Xz => "xz",
            Compression::Zstd => "zst",
        }
    }
}

/// What this crate can open.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Format {
    Zip,
    /// A tar, possibly compressed.
    Tar(Compression),
    SevenZ,
    /// A single compressed file with no archive inside it — `dump.sql.gz`.
    ///
    /// Browsed as an archive holding exactly one member, named by
    /// stripping the compression extension, because that is what it
    /// *is*: a container with one thing in it. The alternative — treat
    /// it as an ordinary file — means the only way to see what is in a
    /// compressed log is to decompress it first, which is the job this
    /// is meant to save.
    ///
    /// Never [`Compression::None`]: an uncompressed stream with no
    /// archive structure is a plain file, and there is nothing here to
    /// browse.
    Compressed(Compression),
}

impl Format {
    /// Whether a browser should offer to open this name as an archive,
    /// judged by name alone.
    ///
    /// A filter, never an answer — a `.zip` that is really a JPEG fails
    /// [`sniff`] and is reported as such. Deliberately narrower than
    /// `EntryKind::Archive`: `.rar` classifies as an archive for its
    /// icon and is not listed here, because nothing in this crate can
    /// open one and offering to would be a dead end.
    pub fn plausible_by_name(path: &Path) -> bool {
        Format::by_name(path).is_some()
    }

    /// The format a name claims. Used to pick a writer when *creating*
    /// an archive (where there are no bytes to sniff yet), and as the
    /// name filter above.
    pub fn by_name(path: &Path) -> Option<Format> {
        let name = path.file_name()?.to_str()?.to_ascii_lowercase();

        // Longest first: `.tar.gz` must not be read as `.gz`.
        for (suffix, format) in [
            (".tar.gz", Format::Tar(Compression::Gzip)),
            (".tar.bz2", Format::Tar(Compression::Bzip2)),
            (".tar.xz", Format::Tar(Compression::Xz)),
            (".tar.zst", Format::Tar(Compression::Zstd)),
            (".tgz", Format::Tar(Compression::Gzip)),
            (".tbz", Format::Tar(Compression::Bzip2)),
            (".tbz2", Format::Tar(Compression::Bzip2)),
            (".txz", Format::Tar(Compression::Xz)),
            (".tzst", Format::Tar(Compression::Zstd)),
            (".tar", Format::Tar(Compression::None)),
            (".zip", Format::Zip),
            (".7z", Format::SevenZ),
            (".gz", Format::Compressed(Compression::Gzip)),
            (".bz2", Format::Compressed(Compression::Bzip2)),
            (".xz", Format::Compressed(Compression::Xz)),
            (".zst", Format::Compressed(Compression::Zstd)),
        ] {
            if name.ends_with(suffix) {
                return Some(format);
            }
        }
        None
    }

    /// The extension a newly created archive of this format gets, with
    /// no leading dot.
    pub fn extension(self) -> &'static str {
        match self {
            Format::Zip => "zip",
            Format::SevenZ => "7z",
            Format::Tar(Compression::None) => "tar",
            Format::Tar(c) => match c {
                Compression::Gzip => "tar.gz",
                Compression::Bzip2 => "tar.bz2",
                Compression::Xz => "tar.xz",
                Compression::Zstd => "tar.zst",
                Compression::None => unreachable!("handled above"),
            },
            Format::Compressed(c) => c.extension(),
        }
    }

    /// What a listing calls this format — the `zstd` in mockup `1j`'s
    /// summary line.
    ///
    /// The *codec*, not the file extension, for a tar: `release.tar.zst`
    /// is a tar and what is interesting about it beside a byte count is
    /// what compressed it. A zip and a 7z are named for themselves,
    /// because the container is the answer there.
    pub fn label(self) -> &'static str {
        match self {
            Format::Zip => "zip",
            Format::SevenZ => "7z",
            Format::Tar(Compression::None) => "tar",
            Format::Tar(c) | Format::Compressed(c) => match c {
                Compression::Gzip => "gzip",
                Compression::Bzip2 => "bzip2",
                Compression::Xz => "xz",
                Compression::Zstd => "zstd",
                Compression::None => "tar",
            },
        }
    }

    /// Whether this crate can write this format — every one it can read,
    /// as it happens, but the two are separate questions and a caller
    /// offering a "Compress to…" list should ask this one.
    pub fn writable(self) -> bool {
        match self {
            Format::Zip | Format::SevenZ | Format::Tar(_) => true,
            // Rewriting a single-member stream is possible and pointless:
            // "add a second file to dump.sql.gz" has no answer in the
            // format. Creating one is a compression, not an archiving,
            // and belongs to whoever asked for it by name.
            Format::Compressed(_) => false,
        }
    }
}

/// The magic bytes of each container. Offset 0 unless stated.
const ZIP_LOCAL: &[u8] = b"PK\x03\x04";
/// An empty zip has no local file header — only the end-of-central-
/// directory record, which starts the file. A zip with nothing in it is
/// a real thing to open (and shows an empty listing), not a broken one.
const ZIP_EMPTY: &[u8] = b"PK\x05\x06";
const ZIP_SPANNED: &[u8] = b"PK\x07\x08";
const SEVENZ: &[u8] = b"7z\xbc\xaf\x27\x1c";
const GZIP: &[u8] = b"\x1f\x8b";
const BZIP2: &[u8] = b"BZh";
const XZ: &[u8] = b"\xfd7zXZ\x00";
const ZSTD: &[u8] = b"\x28\xb5\x2f\xfd";
/// tar's is at offset 257, and comes in two spellings: POSIX ustar
/// (`ustar\0` plus a two-digit version) and GNU (`ustar  \0`). Matching
/// the first five characters covers both without caring which.
const TAR_OFFSET: usize = 257;
const TAR_MAGIC: &[u8] = b"ustar";

/// Enough bytes for every check above, including tar's at 257.
const PEEK: usize = 512;

/// What `bytes` actually is, ignoring what it is called.
///
/// `None` means "not an archive this crate knows", which is an ordinary
/// answer and not an error: it is what a `.zip` that is really a text
/// file gives, and the caller's job is to say so rather than to fail.
pub fn sniff_bytes(bytes: &[u8]) -> Option<Format> {
    fn starts(bytes: &[u8], magic: &[u8]) -> bool {
        bytes.len() >= magic.len() && &bytes[..magic.len()] == magic
    }

    if starts(bytes, ZIP_LOCAL) || starts(bytes, ZIP_EMPTY) || starts(bytes, ZIP_SPANNED) {
        return Some(Format::Zip);
    }
    if starts(bytes, SEVENZ) {
        return Some(Format::SevenZ);
    }
    // An uncompressed tar is recognised by its own magic and nothing
    // else — there is no header at offset 0 to go on.
    if is_tar(bytes) {
        return Some(Format::Tar(Compression::None));
    }
    for (magic, compression) in [
        (GZIP, Compression::Gzip),
        (BZIP2, Compression::Bzip2),
        (XZ, Compression::Xz),
        (ZSTD, Compression::Zstd),
    ] {
        if starts(bytes, magic) {
            // Whether a tar is inside is a question for `sniff`, which
            // has the file to decompress a block of. From bytes alone
            // the honest answer is the compression and no more.
            return Some(Format::Compressed(compression));
        }
    }
    None
}

/// Whether a decompressed block looks like the start of a tar.
fn is_tar(block: &[u8]) -> bool {
    block.len() >= TAR_OFFSET + TAR_MAGIC.len()
        && &block[TAR_OFFSET..TAR_OFFSET + TAR_MAGIC.len()] == TAR_MAGIC
}

/// What the file at `path` is, by reading it.
///
/// This is the one that resolves the `.gz` ambiguity the module doc
/// describes: a compressed stream gets one block decompressed and
/// checked for tar's magic, so `linux.tar.gz` comes back as
/// [`Format::Tar`] and `dump.sql.gz` as [`Format::Compressed`] — under
/// whatever either of them is named.
pub fn sniff(path: &Path) -> std::io::Result<Option<Format>> {
    let mut file = std::fs::File::open(path)?;
    let mut head = vec![0u8; PEEK];
    let read = read_upto(&mut file, &mut head)?;
    head.truncate(read);

    let Some(outer) = sniff_bytes(&head) else {
        return Ok(None);
    };
    let Format::Compressed(compression) = outer else {
        return Ok(Some(outer));
    };

    // Reopen rather than seek: every decoder below wants a stream
    // starting at byte zero, and a fresh handle says that without
    // anyone having to remember to rewind.
    let file = std::fs::File::open(path)?;
    let mut block = vec![0u8; PEEK];
    let read = match crate::stream::decoder(compression, Box::new(file)) {
        Ok(mut decoder) => read_upto(&mut decoder, &mut block).unwrap_or(0),
        // A stream that will not decompress its own first block is
        // damaged, or is a compression this build cannot do. Either way
        // it is still that compression — reporting it as a
        // single-member container lets the caller open it and fail with
        // a real message, which beats claiming it is not an archive.
        Err(_) => 0,
    };
    block.truncate(read);

    Ok(Some(if is_tar(&block) {
        Format::Tar(compression)
    } else {
        Format::Compressed(compression)
    }))
}

/// Fills as much of `buf` as the reader will give, stopping at the end
/// of the stream rather than failing.
///
/// `Read::read` is allowed to return fewer bytes than asked for whenever
/// it likes, and a decompressor routinely does — it returns what one
/// internal block yielded. A single `read` call here would see 200 bytes
/// of a perfectly good tar and conclude, from the 57 bytes it was short,
/// that the magic at offset 257 was not there. `read_exact` is the other
/// wrong answer: a file shorter than `PEEK` is not an error, it is a
/// small file.
fn read_upto(reader: &mut impl Read, buf: &mut [u8]) -> std::io::Result<usize> {
    let mut filled = 0;
    while filled < buf.len() {
        match reader.read(&mut buf[filled..]) {
            Ok(0) => break,
            Ok(n) => filled += n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        }
    }
    Ok(filled)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn a_tar_suffix_wins_over_the_compression_suffix_it_ends_with() {
        assert_eq!(
            Format::by_name(&PathBuf::from("linux-6.6.tar.gz")),
            Some(Format::Tar(Compression::Gzip)),
            "`.tar.gz` read as `.gz` would offer one member called linux-6.6.tar"
        );
        assert_eq!(
            Format::by_name(&PathBuf::from("dump.sql.gz")),
            Some(Format::Compressed(Compression::Gzip))
        );
    }

    #[test]
    fn a_name_this_crate_cannot_open_is_not_offered_as_an_archive() {
        // Classified `EntryKind::Archive` for its icon; unopenable here.
        assert!(!Format::plausible_by_name(&PathBuf::from("game.rar")));
        assert!(!Format::plausible_by_name(&PathBuf::from("notes.txt")));
        assert!(Format::plausible_by_name(&PathBuf::from("notes.zip")));
    }

    #[test]
    fn an_empty_zip_is_recognised_rather_than_read_as_damaged() {
        // 22 bytes, the whole of an end-of-central-directory record.
        let mut empty = ZIP_EMPTY.to_vec();
        empty.extend_from_slice(&[0u8; 18]);
        assert_eq!(sniff_bytes(&empty), Some(Format::Zip));
    }

    #[test]
    fn the_bytes_decide_and_not_the_name() {
        let dir = tempfile::tempdir().unwrap();
        let liar = dir.path().join("photo.zip");
        std::fs::write(&liar, b"this is plainly not a zip file").unwrap();
        assert_eq!(sniff(&liar).unwrap(), None);
    }

    #[test]
    fn a_file_shorter_than_one_peek_is_not_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let tiny = dir.path().join("tiny.zip");
        std::fs::write(&tiny, b"PK").unwrap();
        assert_eq!(sniff(&tiny).unwrap(), None, "two bytes is not yet a zip");
    }

    #[test]
    fn an_uncompressed_tar_is_recognised_by_the_magic_at_offset_257() {
        let mut block = vec![0u8; PEEK];
        block[TAR_OFFSET..TAR_OFFSET + 5].copy_from_slice(TAR_MAGIC);
        assert_eq!(sniff_bytes(&block), Some(Format::Tar(Compression::None)));
    }
}
