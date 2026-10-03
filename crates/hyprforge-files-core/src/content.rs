//! Searching what files *say*: `content:"text"`, answered during the
//! search walk ([`crate::search::walk`]) with no index.
//!
//! An index is the usual answer and was the reason this was first left
//! out — but an index is a daemon, a database and a staleness problem,
//! and a person searching one project for `TODO` does not need any of
//! them. What they need is for the question to cost a bounded amount
//! and to say what it did not look at. So this is a predicate, applied
//! last — after the name, the extension, the size and the date have
//! already said no to everything they can — and bounded on its own,
//! inside the walk's bounds:
//!
//! - **Only regular files.** A folder, a symbolic link, a pipe or a
//!   device is never opened. A pipe is the dangerous one: opening it for
//!   reading waits for a writer that may never come, so the real opener
//!   ([`open_regular`]) asks `lstat` first and opens without blocking.
//! - **Never an archive.** A `.zip` found on the way is a file whose
//!   contents are compressed — reading its bytes for "TODO" answers
//!   nothing — and a walk that started *inside* one has no file on disk
//!   to read at all. Both are counted, never quietly skipped.
//! - **Not a binary.** The first block is read and a NUL byte in it ends
//!   the file there, as `grep` decides. UTF-16 text has NULs too and is
//!   skipped for the same reason; it is counted as binary, which is what
//!   it looks like from here.
//! - **Not a large file.** Anything bigger than [`ContentBudget::file`]
//!   is not opened, and a file that *grows* past it while being read (a
//!   log) is not read past it.
//! - **Not more than [`ContentBudget::total`] in all.** The walk ends
//!   with [`crate::search::End::ReadLimit`] when it is spent, the way it
//!   ends at its folder limit.
//! - **Streamed, never loaded.** One buffer of [`CHUNK`] bytes per walk,
//!   reused for every file; a match that straddles two reads is found
//!   because the tail of one read is kept in front of the next.
//!
//! Matching ignores ASCII case — `todo` finds `TODO` — and compares
//! anything outside ASCII exactly. Folding non-ASCII case as a stream is
//! a Unicode table and a change of length (`ß` is `ss`); the honest
//! narrower promise is the one kept.
//!
//! A file this could not answer for — too large, binary, unreadable, a
//! link — matches *no* content filter, negated or not. `-content:TODO`
//! is "files that do not say TODO", and a binary is not one of those; it
//! is a file nobody read. The same reasoning as a date filter's for a
//! file whose time could not be read (see [`crate::query`]).

use std::io::{self, Read};
use std::path::Path;

/// How much reading one search may do — see the module doc. Part of
/// [`crate::search::Budget`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContentBudget {
    /// A file larger than this is not opened, and none is read past it.
    pub file: u64,
    /// Bytes read across the whole search.
    pub total: u64,
}

impl Default for ContentBudget {
    /// Measured on this machine — see `crates/hyprforge-files/DESIGN.md`,
    /// phase F's content search. A source file is kilobytes; 4 MiB keeps
    /// every one of them and leaves out logs, dumps and generated
    /// bundles, which are where a content search's time would otherwise
    /// go. The total, 1 GiB, was 0.6s warm and up to 4.9s on a first
    /// read here; this monorepo's whole text is 9 MiB, so a project is
    /// never near it, and a search of all of `~` (2.7 GiB of text below
    /// the cap) stops at it and says so rather than reading for seconds
    /// more.
    fn default() -> Self {
        ContentBudget { file: 4 << 20, total: 1 << 30 }
    }
}

/// One `content:` filter, ready to compare: its text as bytes with ASCII
/// lowercased, and whether it was negated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Needle {
    pub negated: bool,
    pub bytes: Vec<u8>,
}

impl Needle {
    pub fn new(text: &str, negated: bool) -> Needle {
        Needle { negated, bytes: text.as_bytes().to_ascii_lowercase() }
    }
}

/// A file a host was able to open for reading, and how long it says it is.
pub struct Opened {
    pub len: u64,
    pub reader: Box<dyn Read>,
}

/// Why a host would not hand over a file's bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refused {
    /// A link, a pipe, a device — anything but a regular file.
    NotRegular,
    /// Permission, or it went away between the listing and now.
    Unreadable,
    /// Inside an archive, which has no file on disk to read.
    InArchive,
}

/// How a host opens a file for content search. The real one is
/// [`open_regular`]; a walk inside an archive is handed [`in_archive`].
pub type Opener<'a> = &'a dyn Fn(&Path) -> Result<Opened, Refused>;

/// What a search's reading came to — every file it did not read, and why.
/// The rail says each non-zero count; see `browser/searching.rs`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ContentReport {
    /// Files whose contents were compared.
    pub read: usize,
    pub bytes: u64,
    /// Bigger than [`ContentBudget::file`], or grew past it.
    pub too_large: usize,
    /// A NUL in the first block.
    pub binary: usize,
    /// Could not be opened or read.
    pub unreadable: usize,
    /// Archives met on the way — compressed, so their bytes say nothing
    /// a search could match. Never opened.
    pub archives: usize,
    /// Files inside an archive the search started in, which have no file
    /// on disk to read.
    pub packed: usize,
    /// Symbolic links and other non-regular files, never followed.
    pub links: usize,
}

impl ContentReport {
    /// Every file that matched by name and could not be answered for.
    pub fn skipped(&self) -> usize {
        self.too_large + self.binary + self.unreadable + self.archives + self.packed + self.links
    }
}

/// What reading one file decided.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// Every content filter holds.
    Holds,
    /// It was read, and at least one does not.
    Fails,
    /// It could not be read — counted in the report, matched by nothing.
    Unknown,
    /// The search's reading budget is spent. Ends the walk.
    Spent,
}

/// How much is read at a time. 64 KiB: big enough that a syscall per read
/// is noise, small enough that the one buffer a walk holds is too.
pub const CHUNK: usize = 64 << 10;

/// The first block, sniffed for a NUL — `grep`'s rule, at its size.
pub const SNIFF: usize = 8 << 10;

/// One walk's reader: its buffer, its budget and its report. Built once
/// per walk and reused for every file, so the memory a content search
/// holds is one [`CHUNK`] plus the longest needle, however many files it
/// reads.
pub struct Scanner {
    budget: ContentBudget,
    buf: Vec<u8>,
    pub report: ContentReport,
}

impl Scanner {
    pub fn new(budget: ContentBudget) -> Scanner {
        Scanner { budget, buf: Vec::new(), report: ContentReport::default() }
    }

    /// Whether the search has read all it may.
    pub fn spent(&self) -> bool {
        self.report.bytes >= self.budget.total
    }

    /// Reads the file at `path` — `entry` as the listing described it —
    /// and decides `needles` against it.
    pub fn check(&mut self, entry: &crate::types::Entry, needles: &[Needle], open: Opener<'_>) -> Verdict {
        if entry.is_dir {
            // A folder has no contents to compare; it matches no content
            // filter either way, and is not worth counting as skipped.
            return Verdict::Fails;
        }
        if entry.is_symlink {
            self.report.links += 1;
            return Verdict::Unknown;
        }
        if entry.kind == crate::types::EntryKind::Archive {
            self.report.archives += 1;
            return Verdict::Unknown;
        }
        if matches!(entry.size, crate::types::EntrySize::Bytes(len) if len > self.budget.file) {
            self.report.too_large += 1;
            return Verdict::Unknown;
        }
        if self.spent() {
            return Verdict::Spent;
        }
        let opened = match open(&entry.path) {
            Ok(opened) => opened,
            Err(Refused::NotRegular) => {
                self.report.links += 1;
                return Verdict::Unknown;
            }
            Err(Refused::Unreadable) => {
                self.report.unreadable += 1;
                return Verdict::Unknown;
            }
            Err(Refused::InArchive) => {
                self.report.packed += 1;
                return Verdict::Unknown;
            }
        };
        // The length now, not the listing's: it may have grown since.
        if opened.len > self.budget.file {
            self.report.too_large += 1;
            return Verdict::Unknown;
        }
        let remaining = self.budget.total - self.report.bytes;
        let limit = self.budget.file.min(remaining);
        let mut reader = opened.reader;
        match scan(&mut reader, needles, limit, &mut self.buf) {
            Ok((outcome, bytes)) => {
                self.report.bytes += bytes;
                match outcome {
                    Scan::Binary => {
                        self.report.binary += 1;
                        Verdict::Unknown
                    }
                    Scan::Cut if limit < self.budget.file => {
                        // Stopped by the search's budget, not the file's
                        // size: nothing more will be read by anyone.
                        Verdict::Spent
                    }
                    Scan::Cut => {
                        self.report.too_large += 1;
                        Verdict::Unknown
                    }
                    Scan::Decided(found) => {
                        self.report.read += 1;
                        let holds = needles.iter().zip(found).all(|(n, f)| f != n.negated);
                        if holds { Verdict::Holds } else { Verdict::Fails }
                    }
                }
            }
            Err(_) => {
                self.report.unreadable += 1;
                Verdict::Unknown
            }
        }
    }
}

/// How reading one file ended.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Scan {
    /// Whether each needle was found, in order.
    Decided(Vec<bool>),
    Binary,
    /// The limit came before the end, with something still undecided.
    Cut,
}

/// Fills `buf` from `reader` until it is full or the file ends. `read` may
/// hand back less than asked for, and a decompressing or network reader
/// routinely does; one short read is not the end of a file.
fn fill(reader: &mut dyn Read, buf: &mut [u8]) -> io::Result<usize> {
    let mut filled = 0;
    while filled < buf.len() {
        match reader.read(&mut buf[filled..]) {
            Ok(0) => break,
            Ok(n) => filled += n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
    Ok(filled)
}

/// Reads at most `limit` bytes of `reader` in [`CHUNK`]s, looking for
/// every needle, stopping early once all of them are found. Returns how
/// it ended and how many bytes it read.
fn scan(reader: &mut dyn Read, needles: &[Needle], limit: u64, buf: &mut Vec<u8>) -> io::Result<(Scan, u64)> {
    let overlap = needles.iter().map(|n| n.bytes.len()).max().unwrap_or(0).saturating_sub(1);
    buf.resize(CHUNK + overlap, 0);
    let mut found = vec![false; needles.len()];
    let mut read: u64 = 0;
    // Bytes carried over from the end of the last chunk, at the front.
    let mut kept = 0;
    let mut first = true;
    loop {
        if found.iter().all(|f| *f) {
            return Ok((Scan::Decided(found), read));
        }
        let want = (CHUNK as u64).min(limit - read) as usize;
        if want == 0 {
            // At the limit with something undecided — unless the file
            // ends exactly here, which one more byte tells. Without
            // asking, a file exactly the size of the cap would be
            // called too large having been read in full.
            let mut probe = [0u8; 1];
            return Ok(if fill(reader, &mut probe)? == 0 { (Scan::Decided(found), read) } else { (Scan::Cut, read) });
        }
        let got = fill(reader, &mut buf[kept..kept + want])?;
        read += got as u64;
        if first {
            first = false;
            if memchr::memchr(0, &buf[..got.min(SNIFF)]).is_some() {
                return Ok((Scan::Binary, read));
            }
        }
        let window = &buf[..kept + got];
        for (needle, done) in needles.iter().zip(found.iter_mut()) {
            if !*done && contains(window, &needle.bytes) {
                *done = true;
            }
        }
        if got < want {
            // The file ended inside this chunk.
            return Ok((Scan::Decided(found), read));
        }
        let keep = overlap.min(window.len());
        let from = window.len() - keep;
        buf.copy_within(from..from + keep, 0);
        kept = keep;
    }
}

/// Whether `haystack` holds `needle` (already ASCII-lowercased), ignoring
/// ASCII case. Candidates are found by the first byte in either case with
/// `memchr2`, and each is compared in full — no lowercased copy of the
/// haystack is ever made.
fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    let Some(&first) = needle.first() else { return true };
    if needle.len() > haystack.len() {
        return false;
    }
    let last_start = haystack.len() - needle.len();
    let upper = first.to_ascii_uppercase();
    let mut at = 0;
    while at <= last_start {
        let rest = &haystack[at..=last_start];
        let Some(offset) = memchr::memchr2(first, upper, rest) else { return false };
        let start = at + offset;
        if haystack[start..start + needle.len()].eq_ignore_ascii_case(needle) {
            return true;
        }
        at = start + 1;
    }
    false
}

/// The real opener: a regular file on disk, opened without following a
/// link and without blocking — see the module doc for why a pipe matters.
///
/// `lstat` first, so a link or a pipe is refused without being opened.
/// The kind is asked again of the open handle, because the name can be
/// replaced between the two; a pipe swapped in there is opened
/// non-blocking (where this platform's flag value is known) and refused
/// by the second check rather than waited on.
pub fn open_regular(path: &Path) -> Result<Opened, Refused> {
    use std::os::unix::fs::OpenOptionsExt;
    match std::fs::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_file() => {}
        Ok(_) => return Err(Refused::NotRegular),
        Err(_) => return Err(Refused::Unreadable),
    }
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(O_NONBLOCK)
        .open(path)
        .map_err(|_| Refused::Unreadable)?;
    let meta = file.metadata().map_err(|_| Refused::Unreadable)?;
    if !meta.file_type().is_file() {
        return Err(Refused::NotRegular);
    }
    Ok(Opened { len: meta.len(), reader: Box::new(file) })
}

/// `O_NONBLOCK`, without a `libc` dependency for one constant. 0o4000 on
/// every Linux architecture this suite builds for; the few that differ
/// (alpha, mips, sparc, parisc) get 0 — an ordinary open, still guarded
/// by the `lstat` above. On a regular file the flag changes nothing.
#[cfg(all(
    target_os = "linux",
    any(
        target_arch = "x86_64",
        target_arch = "x86",
        target_arch = "aarch64",
        target_arch = "arm",
        target_arch = "riscv64",
        target_arch = "powerpc64",
        target_arch = "loongarch64"
    )
))]
const O_NONBLOCK: i32 = 0o4000;
#[cfg(not(all(
    target_os = "linux",
    any(
        target_arch = "x86_64",
        target_arch = "x86",
        target_arch = "aarch64",
        target_arch = "arm",
        target_arch = "riscv64",
        target_arch = "powerpc64",
        target_arch = "loongarch64"
    )
)))]
const O_NONBLOCK: i32 = 0;

/// The opener for a walk inside an archive: nothing there is a file on
/// disk, and decompressing members to search them is the index-sized job
/// this feature is not.
pub fn in_archive(_: &Path) -> Result<Opened, Refused> {
    Err(Refused::InArchive)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Entry, EntryKind};
    use std::path::PathBuf;

    fn needles(texts: &[&str]) -> Vec<Needle> {
        texts.iter().map(|t| match t.strip_prefix('-') {
            Some(t) => Needle::new(t, true),
            None => Needle::new(t, false),
        }).collect()
    }

    fn scanned(bytes: &[u8], texts: &[&str], limit: u64) -> Scan {
        let mut buf = Vec::new();
        scan(&mut &bytes[..], &needles(texts), limit, &mut buf).unwrap().0
    }

    /// A reader that hands back at most `step` bytes a call — how a pipe,
    /// a network filesystem or a decompressor behaves.
    struct Trickle<'a> {
        bytes: &'a [u8],
        step: usize,
    }

    impl Read for Trickle<'_> {
        fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
            let n = self.step.min(out.len()).min(self.bytes.len());
            out[..n].copy_from_slice(&self.bytes[..n]);
            self.bytes = &self.bytes[n..];
            Ok(n)
        }
    }

    fn file(name: &str, len: u64) -> Entry {
        crate::backend::mock::MockBackend::file(Path::new("/d"), name, len)
    }

    #[test]
    fn case_is_ignored_for_ascii_letters() {
        assert_eq!(scanned(b"// TODO: later", &["todo"], 1 << 20), Scan::Decided(vec![true]));
        assert_eq!(scanned(b"nothing here", &["todo"], 1 << 20), Scan::Decided(vec![false]));
    }

    #[test]
    fn text_outside_ascii_is_compared_exactly() {
        assert_eq!(scanned("Größe".as_bytes(), &["größe"], 1 << 20), Scan::Decided(vec![true]));
        assert_eq!(scanned("GRÖSSE".as_bytes(), &["größe"], 1 << 20), Scan::Decided(vec![false]));
    }

    /// The property streaming has to keep: a match cut in two by the
    /// chunk boundary is still a match.
    #[test]
    fn a_match_across_two_reads_is_found() {
        let mut bytes = vec![b'x'; CHUNK - 3];
        bytes.extend_from_slice(b"needle");
        bytes.extend(vec![b'y'; 100]);
        assert_eq!(scanned(&bytes, &["NEEDLE"], 1 << 20), Scan::Decided(vec![true]));
    }

    /// `read` returning a few bytes at a time must not read as the end of
    /// the file, nor hide a NUL from the sniff.
    #[test]
    fn short_reads_are_neither_the_end_nor_a_hiding_place() {
        let mut bytes = b"hello".to_vec();
        bytes.extend(vec![b'.'; 3000]);
        bytes.extend_from_slice(b"\0binary");
        let mut buf = Vec::new();
        let mut trickle = Trickle { bytes: &bytes, step: 7 };
        assert_eq!(scan(&mut trickle, &needles(&["binary"]), 1 << 20, &mut buf).unwrap().0, Scan::Binary);
        let text = [b"a".repeat(CHUNK * 2), b"needle".to_vec()].concat();
        let mut trickle = Trickle { bytes: &text, step: 1000 };
        assert_eq!(scan(&mut trickle, &needles(&["needle"]), 1 << 20, &mut buf).unwrap().0, Scan::Decided(vec![true]));
    }

    #[test]
    fn a_nul_in_the_first_block_is_a_binary() {
        assert_eq!(scanned(b"\x7fELF\0\0\0needle", &["needle"], 1 << 20), Scan::Binary);
    }

    #[test]
    fn reading_stops_once_every_needle_is_found() {
        let bytes = [b"needle".to_vec(), vec![b'z'; CHUNK * 4]].concat();
        let mut buf = Vec::new();
        let (scan, read) = scan(&mut &bytes[..], &needles(&["needle"]), 1 << 30, &mut buf).unwrap();
        assert_eq!(scan, Scan::Decided(vec![true]));
        assert_eq!(read, CHUNK as u64, "one chunk, not the whole file");
    }

    /// Resource, not result: the buffer is one chunk and the longest
    /// needle, however big the file.
    #[test]
    fn the_buffer_never_grows_past_one_chunk_and_a_needle() {
        let bytes = vec![b'a'; CHUNK * 10];
        let mut buf = Vec::new();
        scan(&mut &bytes[..], &needles(&["absent-needle"]), 1 << 30, &mut buf).unwrap();
        assert_eq!(buf.len(), CHUNK + "absent-needle".len() - 1);
    }

    #[test]
    fn a_file_longer_than_the_limit_is_cut_not_decided() {
        let bytes = vec![b'a'; CHUNK * 3];
        assert_eq!(scanned(&bytes, &["b"], CHUNK as u64), Scan::Cut);
        assert_eq!(scanned(&bytes, &["b"], bytes.len() as u64), Scan::Decided(vec![false]), "exactly at the limit is read in full");
    }

    fn opener(contents: &'static [u8]) -> impl Fn(&Path) -> Result<Opened, Refused> {
        move |_| Ok(Opened { len: contents.len() as u64, reader: Box::new(contents) })
    }

    #[test]
    fn a_negated_needle_holds_only_for_a_file_that_was_read() {
        let mut scanner = Scanner::new(ContentBudget::default());
        let open = opener(b"plain text");
        assert_eq!(scanner.check(&file("a.txt", 10), &needles(&["-todo"]), &open), Verdict::Holds);
        let binary = opener(b"\0\0");
        assert_eq!(scanner.check(&file("b.bin", 2), &needles(&["-todo"]), &binary), Verdict::Unknown);
        assert_eq!(scanner.report.binary, 1);
        assert_eq!(scanner.report.read, 1);
    }

    #[test]
    fn folders_links_archives_and_large_files_are_never_opened() {
        let mut scanner = Scanner::new(ContentBudget { file: 100, total: 1 << 20 });
        let never = |_: &Path| -> Result<Opened, Refused> { panic!("opened") };
        let mut folder = file("dir", 0);
        folder.is_dir = true;
        folder.kind = EntryKind::Folder;
        assert_eq!(scanner.check(&folder, &needles(&["x"]), &never), Verdict::Fails);
        let mut link = file("l.txt", 1);
        link.is_symlink = true;
        assert_eq!(scanner.check(&link, &needles(&["x"]), &never), Verdict::Unknown);
        assert_eq!(scanner.check(&file("a.zip", 1), &needles(&["x"]), &never), Verdict::Unknown);
        assert_eq!(scanner.check(&file("big.log", 101), &needles(&["x"]), &never), Verdict::Unknown);
        let r = scanner.report;
        assert_eq!((r.links, r.archives, r.too_large, r.read, r.bytes), (1, 1, 1, 0, 0));
    }

    /// A file that grew since the listing is judged by what the open
    /// handle says, not by the stale size.
    #[test]
    fn a_file_that_grew_past_the_cap_is_too_large() {
        let mut scanner = Scanner::new(ContentBudget { file: 4, total: 1 << 20 });
        let grown = opener(b"0123456789");
        assert_eq!(scanner.check(&file("log", 2), &needles(&["9"]), &grown), Verdict::Unknown);
        assert_eq!(scanner.report.too_large, 1);
    }

    #[test]
    fn the_total_budget_ends_the_search() {
        let mut scanner = Scanner::new(ContentBudget { file: 1 << 20, total: 10 });
        let open = opener(b"0123456789abcdef");
        assert_eq!(scanner.check(&file("a", 16), &needles(&["f"]), &open), Verdict::Spent);
        assert!(scanner.spent());
        assert_eq!(scanner.check(&file("b", 16), &needles(&["f"]), &open), Verdict::Spent);
        assert_eq!(scanner.report.bytes, 10, "never past the budget");
    }

    #[test]
    fn inside_an_archive_nothing_is_opened_and_each_file_is_counted() {
        let mut scanner = Scanner::new(ContentBudget::default());
        assert_eq!(scanner.check(&file("a.txt", 1), &needles(&["x"]), &in_archive), Verdict::Unknown);
        assert_eq!(scanner.report.packed, 1);
    }

    #[test]
    fn the_real_opener_reads_a_file_and_refuses_a_link_and_a_pipe() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.txt");
        std::fs::write(&path, "hello TODO").unwrap();
        let entry = Entry { path: path.clone(), ..file("a.txt", 10) };
        let mut scanner = Scanner::new(ContentBudget::default());
        assert_eq!(scanner.check(&entry, &needles(&["todo"]), &open_regular), Verdict::Holds);

        let link = dir.path().join("l");
        std::os::unix::fs::symlink(&path, &link).unwrap();
        assert_eq!(open_regular(&link).err(), Some(Refused::NotRegular));

        // A pipe nobody writes to: opening it for reading would wait
        // forever. Refused without being opened.
        let fifo = dir.path().join("p");
        let made = std::process::Command::new("mkfifo").arg(&fifo).status();
        if made.is_ok_and(|s| s.success()) {
            assert_eq!(open_regular(&fifo).err(), Some(Refused::NotRegular));
        } else {
            eprintln!("HYPRFORGE-SKIP: mkfifo is not available, so the pipe half was not checked");
        }
        assert_eq!(open_regular(&PathBuf::from("/no/such/file")).err(), Some(Refused::Unreadable));
    }
}
