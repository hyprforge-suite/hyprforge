//! What a file is, decided by its **contents**.
//!
//! The other half of the question [`crate::globs`] answers by name. A
//! file with no extension, or one whose extension lies, can still be
//! recognised by what is in it: `%PDF-` at offset 0, `\x89PNG` at offset
//! 0, `solid ` for an ASCII STL.
//!
//! # The format
//!
//! `/usr/share/mime/magic` is binary, and deliberately so — it is read
//! at every content lookup on the system. After the header
//! `MIME-Magic\0\n` it is a sequence of sections:
//!
//! ```text
//! [<priority>:<mime type>]\n
//! [indent]>offset=LLvalue[&mask][~word][+range]\n
//! ```
//!
//! `LL` is two bytes, big-endian, giving the length of `value` — which
//! is raw bytes and may itself contain a newline. That single detail is
//! why this is parsed byte by byte and not with `lines()`: splitting on
//! `\n` cuts rules in half, quietly, and the ones it cuts are the
//! interesting ones.
//!
//! # Nesting
//!
//! A rule indented one deeper than the line above is a *further*
//! condition on it. A rule matches when its own pattern matches and,
//! if it has children, at least one child matches too. That is what
//! makes `>0=\x00\x05<?xml` plus `1>0=...DocBook...` mean "an XML file,
//! and specifically a DocBook one" rather than two unrelated claims.
//!
//! # Why this exists when `globs2` already answered
//!
//! Most of the time the name is right and this is not consulted at all
//! — see [`crate::lookup`] for the order. It earns its place on the
//! files that have no name to go on: something saved as `download`,
//! a file being examined before it is renamed, `stdin`.

use std::io::Read;

/// How many bytes are read from a file to match against. The database's
/// deepest rule on this machine sits well inside this; the value is the
/// one `xdgmime` uses, so a file this cannot recognise is one the rest
/// of the desktop cannot recognise either.
const SNIFF_BYTES: usize = 16 * 1024;

/// One condition: these bytes, at this offset.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Rule {
    /// How deep this rule is nested under the one before it.
    indent: u32,
    /// Where the value starts.
    offset: usize,
    value: Vec<u8>,
    /// Compared as `data & mask == value & mask` when present.
    mask: Option<Vec<u8>>,
    /// How many bytes past `offset` to keep looking.
    range: usize,
}

impl Rule {
    fn matches(&self, data: &[u8]) -> bool {
        // `..=`: a range of 0 still tests the offset itself, which is
        // the common case and the one an exclusive range would skip.
        (self.offset..=self.offset + self.range).any(|start| {
            let Some(window) = data.get(start..start + self.value.len()) else { return false };
            match &self.mask {
                None => window == self.value,
                Some(mask) => window
                    .iter()
                    .zip(&self.value)
                    .zip(mask)
                    .all(|((byte, value), mask)| byte & mask == value & mask),
            }
        })
    }
}

/// Every rule for one type, with the priority the database gives it.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Section {
    /// 0-100. 80 and above is "strong enough to overrule a filename" —
    /// see [`Magic::lookup`].
    priority: u32,
    mime: String,
    rules: Vec<Rule>,
}

impl Section {
    fn matches(&self, data: &[u8]) -> bool {
        matches_level(&self.rules, data, 0)
    }
}

/// A rule at `indent` matches when its own pattern matches and, if it
/// has children, one of them matches too — see the module doc.
fn matches_level(rules: &[Rule], data: &[u8], indent: u32) -> bool {
    let mut i = 0;
    while i < rules.len() {
        if rules[i].indent != indent {
            i += 1;
            continue;
        }
        if rules[i].matches(data) {
            let children_start = i + 1;
            let children_end = rules[children_start..]
                .iter()
                .position(|rule| rule.indent <= indent)
                .map(|offset| children_start + offset)
                .unwrap_or(rules.len());
            if children_start == children_end {
                return true;
            }
            if matches_level(&rules[children_start..children_end], data, indent + 1) {
                return true;
            }
        }
        i += 1;
    }
    false
}

/// The content rules, in the order the database gives them.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Magic {
    sections: Vec<Section>,
}

/// What a content match found, and how much it should be trusted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Match {
    pub mime: String,
    pub priority: u32,
}

impl Magic {
    /// Parses one `magic` file.
    ///
    /// A file that does not start with the header is not this format
    /// and is ignored rather than guessed at. Anything unparseable
    /// after that ends the read: the format is length-prefixed, so once
    /// a length is misread there is no way back to a known position —
    /// and a wrong guess about where the next rule starts is worse than
    /// stopping with what was understood.
    pub fn parse(bytes: &[u8]) -> Magic {
        let mut magic = Magic::default();
        let Some(mut rest) = bytes.strip_prefix(b"MIME-Magic\0\n") else {
            return magic;
        };
        while !rest.is_empty() {
            if rest[0] != b'[' {
                break;
            }
            let Some(close) = rest.iter().position(|b| *b == b']') else { break };
            let header = &rest[1..close];
            rest = &rest[close + 1..];
            if rest.first() == Some(&b'\n') {
                rest = &rest[1..];
            }
            let Some((priority, mime)) = split_header(header) else { break };
            let mut section = Section { priority, mime, rules: Vec::new() };
            while !rest.is_empty() && rest[0] != b'[' {
                match parse_rule(rest) {
                    Some((rule, tail)) => {
                        section.rules.push(rule);
                        rest = tail;
                    }
                    None => {
                        rest = &[];
                        break;
                    }
                }
            }
            if !section.rules.is_empty() {
                magic.sections.push(section);
            }
        }
        magic
    }

    /// Reads every `magic` file under the given directories. Later
    /// directories add their sections after the earlier ones, which is
    /// the order the priority sort then works over.
    pub fn load_from(dirs: &[std::path::PathBuf]) -> Magic {
        let mut magic = Magic::default();
        for dir in dirs {
            if let Ok(bytes) = std::fs::read(dir.join("mime").join("magic")) {
                magic.sections.extend(Magic::parse(&bytes).sections);
            }
        }
        // Highest priority first, so the first match is the best one.
        // A stable sort, so two rules of equal priority keep the
        // database's own order — which is how `application/xml` stays
        // behind the specific XML formats that share its opening bytes.
        magic.sections.sort_by_key(|section| std::cmp::Reverse(section.priority));
        magic
    }

    /// The best content match for these bytes.
    pub fn of_data(&self, data: &[u8]) -> Option<Match> {
        self.sections
            .iter()
            .find(|section| section.matches(data))
            .map(|section| Match { mime: section.mime.clone(), priority: section.priority })
    }

    /// The best content match for a file, reading only its first
    /// [`SNIFF_BYTES`].
    pub fn of_file(&self, path: &std::path::Path) -> Option<Match> {
        self.of_data(&head(path)?)
    }

    /// Every content match, best first — `mimetype --all`.
    pub fn all_of_data(&self, data: &[u8]) -> Vec<Match> {
        self.sections
            .iter()
            .filter(|section| section.matches(data))
            .map(|section| Match { mime: section.mime.clone(), priority: section.priority })
            .collect()
    }

    pub fn is_empty(&self) -> bool {
        self.sections.is_empty()
    }
}

/// The first [`SNIFF_BYTES`] of a file, for content matching.
///
/// **The one way to read a file for typing.** Bounded on purpose: this
/// is asked about files a person just pointed at, which can be a 40GB
/// disk image on a slow mount, and no magic rule in the database
/// reaches anywhere near that far in.
///
/// It exists as its own function because the bound was written once and
/// then bypassed three times: `Lookup::of_file`, `Lookup::all_of_file`
/// and the `mimetype` command each read the *whole* file instead, so
/// nothing a person actually double-clicked was bounded at all. An
/// 800MB file cost 766MB of resident memory to answer "what is this".
/// That is the failure the project's own rule about testing the
/// resource rather than the result is named after, and the tests here
/// had asked only what type came back.
pub fn head(path: &std::path::Path) -> Option<Vec<u8>> {
    let mut file = std::fs::File::open(path).ok()?;
    let mut buffer = vec![0u8; SNIFF_BYTES];
    let read = read_up_to(&mut file, &mut buffer)?;
    buffer.truncate(read);
    Some(buffer)
}

/// Fills `buffer` as far as the file goes, tolerating short reads.
///
/// `read` is allowed to return fewer bytes than asked for without being
/// at the end of the file, and a single call would then sniff a
/// fraction of what it meant to.
fn read_up_to(file: &mut std::fs::File, buffer: &mut [u8]) -> Option<usize> {
    let mut filled = 0;
    while filled < buffer.len() {
        match file.read(&mut buffer[filled..]) {
            Ok(0) => break,
            Ok(n) => filled += n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => return None,
        }
    }
    Some(filled)
}

/// `90:application/pdf` from inside the brackets.
fn split_header(header: &[u8]) -> Option<(u32, String)> {
    let header = std::str::from_utf8(header).ok()?;
    let (priority, mime) = header.split_once(':')?;
    Some((priority.trim().parse().ok()?, mime.trim().to_string()))
}

/// One rule line, and whatever follows it.
///
/// Returns `None` when the line cannot be read, which stops the parse —
/// see [`Magic::parse`] for why that is better than skipping ahead.
fn parse_rule(bytes: &[u8]) -> Option<(Rule, &[u8])> {
    // [indent]>
    let gt = bytes.iter().position(|b| *b == b'>')?;
    let indent: u32 = match gt {
        0 => 0,
        _ => std::str::from_utf8(&bytes[..gt]).ok()?.trim().parse().ok()?,
    };
    let rest = &bytes[gt + 1..];

    // offset=
    let eq = rest.iter().position(|b| *b == b'=')?;
    let offset: usize = std::str::from_utf8(&rest[..eq]).ok()?.trim().parse().ok()?;
    let rest = &rest[eq + 1..];

    // Two bytes of length, big-endian, then that many bytes of value —
    // which may contain newlines, so the length is the only safe guide.
    let length = usize::from(u16::from_be_bytes([*rest.first()?, *rest.get(1)?]));
    let value = rest.get(2..2 + length)?.to_vec();
    let mut rest = &rest[2 + length..];

    let mut mask = None;
    if rest.first() == Some(&b'&') {
        mask = Some(rest.get(1..1 + length)?.to_vec());
        rest = &rest[1 + length..];
    }
    let mut word_size = 1usize;
    if rest.first() == Some(&b'~') {
        let (number, tail) = take_number(&rest[1..])?;
        word_size = number.max(1);
        rest = tail;
    }
    let mut range = 0usize;
    if rest.first() == Some(&b'+') {
        let (number, tail) = take_number(&rest[1..])?;
        range = number;
        rest = tail;
    }
    // The line ends here; anything else on it is not something this
    // understands, so stop rather than guess.
    match rest.first() {
        Some(b'\n') => rest = &rest[1..],
        None => {}
        Some(_) => return None,
    }

    let (value, mask) = swap_words(value, mask, word_size);
    Some((Rule { indent, offset, value, mask, range }, rest))
}

fn take_number(bytes: &[u8]) -> Option<(usize, &[u8])> {
    let end = bytes.iter().position(|b| !b.is_ascii_digit()).unwrap_or(bytes.len());
    let number = std::str::from_utf8(&bytes[..end]).ok()?.parse().ok()?;
    Some((number, &bytes[end..]))
}

/// Byte-swaps a value stored for a different word size.
///
/// The database writes multi-byte words big-endian and marks them with
/// `~2` or `~4`; on a little-endian machine the bytes in the file are
/// the other way round. Swapping the *pattern* once here beats swapping
/// the file's bytes at every comparison.
fn swap_words(value: Vec<u8>, mask: Option<Vec<u8>>, word_size: usize) -> (Vec<u8>, Option<Vec<u8>>) {
    if word_size <= 1 || !cfg!(target_endian = "little") || !value.len().is_multiple_of(word_size) {
        return (value, mask);
    }
    let swap = |bytes: Vec<u8>| -> Vec<u8> {
        bytes.chunks(word_size).flat_map(|word| word.iter().rev().copied()).collect()
    };
    (swap(value), mask.map(swap))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds a magic file the way the real one is written: a two-byte
    /// big-endian length before every value.
    /// `indent, offset, value, mask, range` — one rule line, as the
    /// file spells it.
    type TestRule<'a> = (u32, usize, &'a [u8], Option<&'a [u8]>, usize);

    fn magic_file(sections: &[(u32, &str, &[TestRule<'_>])]) -> Vec<u8> {
        let mut out = b"MIME-Magic\0\n".to_vec();
        for (priority, mime, rules) in sections {
            out.extend(format!("[{priority}:{mime}]\n").into_bytes());
            for (indent, offset, value, mask, range) in *rules {
                if *indent > 0 {
                    out.extend(indent.to_string().into_bytes());
                }
                out.extend(format!(">{offset}=").into_bytes());
                out.extend((value.len() as u16).to_be_bytes());
                out.extend(*value);
                if let Some(mask) = mask {
                    out.push(b'&');
                    out.extend(*mask);
                }
                if *range > 0 {
                    out.extend(format!("+{range}").into_bytes());
                }
                out.push(b'\n');
            }
        }
        out
    }

    fn pdf_and_png() -> Magic {
        Magic::parse(&magic_file(&[
            (50, "application/pdf", &[(0, 0, b"%PDF-".as_slice(), None, 0)]),
            (50, "image/png", &[(0, 0, b"\x89PNG".as_slice(), None, 0)]),
        ]))
    }

    #[test]
    fn a_files_first_bytes_name_its_type() {
        let magic = pdf_and_png();
        assert_eq!(magic.of_data(b"%PDF-1.7 ...").unwrap().mime, "application/pdf");
        assert_eq!(magic.of_data(b"\x89PNG\r\n\x1a\n").unwrap().mime, "image/png");
        assert_eq!(magic.of_data(b"hello"), None);
        assert_eq!(magic.of_data(b""), None, "nothing to go on");
    }

    /// The detail that makes this a byte parser: a value may contain a
    /// newline, and splitting the file into lines would cut the rule in
    /// half and lose every rule after it.
    #[test]
    fn a_value_containing_a_newline_is_read_whole() {
        let magic = Magic::parse(&magic_file(&[
            (50, "text/x-two-lines", &[(0, 0, b"first\nsecond".as_slice(), None, 0)]),
            (50, "image/png", &[(0, 0, b"\x89PNG".as_slice(), None, 0)]),
        ]));
        assert_eq!(magic.of_data(b"first\nsecond and more").unwrap().mime, "text/x-two-lines");
        assert_eq!(
            magic.of_data(b"\x89PNG").unwrap().mime,
            "image/png",
            "the section after it survived the parse"
        );
    }

    #[test]
    fn a_rule_can_look_within_a_range_rather_than_at_one_offset() {
        let magic = Magic::parse(&magic_file(&[(
            50,
            "text/x-somewhere",
            &[(0, 0, b"needle".as_slice(), None, 20)],
        )]));
        assert!(magic.of_data(b"..........needle").is_some(), "found later in the range");
        assert!(magic.of_data(b"needle").is_some(), "and at the offset itself");
        assert!(
            magic.of_data(b"..............................needle").is_none(),
            "past the range"
        );
    }

    #[test]
    fn a_mask_ignores_the_bits_it_clears() {
        let magic = Magic::parse(&magic_file(&[(
            50,
            "x/masked",
            &[(0, 0, b"\xf0".as_slice(), Some(b"\xf0".as_slice()), 0)],
        )]));
        assert!(magic.of_data(b"\xff").is_some(), "high nibble matches, low ignored");
        assert!(magic.of_data(b"\xf0").is_some());
        assert!(magic.of_data(b"\x0f").is_none());
    }

    /// Nesting is "and also": the XML rule plus a DocBook rule under it
    /// means DocBook, not two separate claims.
    #[test]
    fn a_nested_rule_is_a_further_condition_on_its_parent() {
        let magic = Magic::parse(&magic_file(&[(
            50,
            "application/docbook+xml",
            &[
                (0, 0, b"<?xml".as_slice(), None, 0),
                (1, 0, b"-//OASIS//DTD DocBook".as_slice(), None, 200),
            ],
        )]));
        assert!(
            magic.of_data(b"<?xml version='1.0'?><!DOCTYPE book PUBLIC \"-//OASIS//DTD DocBook XML\">").is_some()
        );
        assert!(magic.of_data(b"<?xml version='1.0'?><html/>").is_none(), "XML, but not DocBook");
    }

    /// Several children are alternatives: any one of them is enough.
    #[test]
    fn any_one_child_satisfies_its_parent() {
        let magic = Magic::parse(&magic_file(&[(
            50,
            "x/either",
            &[
                (0, 0, b"HEAD".as_slice(), None, 0),
                (1, 4, b"one".as_slice(), None, 0),
                (1, 4, b"two".as_slice(), None, 0),
            ],
        )]));
        assert!(magic.of_data(b"HEADone").is_some());
        assert!(magic.of_data(b"HEADtwo").is_some());
        assert!(magic.of_data(b"HEADthree").is_none());
    }

    #[test]
    fn the_strongest_rule_is_the_one_reported() {
        let magic = Magic::load_from_parsed(vec![
            Magic::parse(&magic_file(&[(20, "x/weak", &[(0, 0, b"AB".as_slice(), None, 0)])])),
            Magic::parse(&magic_file(&[(90, "x/strong", &[(0, 0, b"AB".as_slice(), None, 0)])])),
        ]);
        let found = magic.of_data(b"ABCD").unwrap();
        assert_eq!(found.mime, "x/strong");
        assert_eq!(found.priority, 90);
        let all: Vec<String> = magic.all_of_data(b"ABCD").into_iter().map(|m| m.mime).collect();
        assert_eq!(all, ["x/strong", "x/weak"], "and --all lists both, best first");
    }

    #[test]
    fn a_file_that_is_not_this_format_is_ignored_rather_than_guessed_at() {
        assert!(Magic::parse(b"not a magic file at all").is_empty());
        assert!(Magic::parse(b"").is_empty());
    }

    /// A truncated file costs the rules after the damage, not the ones
    /// already understood.
    #[test]
    fn a_truncated_file_keeps_what_was_read() {
        let mut bytes = magic_file(&[
            (50, "application/pdf", &[(0, 0, b"%PDF-".as_slice(), None, 0)]),
            (50, "image/png", &[(0, 0, b"\x89PNG".as_slice(), None, 0)]),
        ]);
        bytes.truncate(bytes.len() - 3);
        let magic = Magic::parse(&bytes);
        assert_eq!(magic.of_data(b"%PDF-1.7").unwrap().mime, "application/pdf");
    }

    #[test]
    fn a_word_sized_value_is_swapped_once_rather_than_per_comparison() {
        let (value, mask) = swap_words(vec![0x12, 0x34, 0x56, 0x78], Some(vec![0xff, 0x00, 0xff, 0x00]), 2);
        if cfg!(target_endian = "little") {
            assert_eq!(value, vec![0x34, 0x12, 0x78, 0x56]);
            assert_eq!(mask, Some(vec![0x00, 0xff, 0x00, 0xff]));
        } else {
            assert_eq!(value, vec![0x12, 0x34, 0x56, 0x78], "big-endian needs no swap");
        }
    }

    /// The bound is the property, not an implementation detail: a file
    /// far larger than the sniff window costs the window, not the file.
    #[test]
    fn a_files_contents_are_read_from_disk_and_bounded() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("anonymous");
        let mut contents = b"%PDF-1.7\n".to_vec();
        contents.extend(std::iter::repeat_n(b'x', SNIFF_BYTES * 4));
        std::fs::write(&path, &contents).unwrap();

        let read = head(&path).expect("readable");
        assert_eq!(read.len(), SNIFF_BYTES, "a big file costs the window, not its size");
        assert_eq!(pdf_and_png().of_file(&path).unwrap().mime, "application/pdf");
        assert_eq!(pdf_and_png().of_file(&dir.path().join("missing")), None);
        assert_eq!(head(&dir.path().join("missing")), None);
    }

    /// A file smaller than the window reads as itself, not as a
    /// window-sized buffer of trailing zeroes — which would make every
    /// short file look like padded binary.
    #[test]
    fn a_small_file_reads_as_exactly_itself() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("small");
        std::fs::write(&path, b"hello").unwrap();
        assert_eq!(head(&path).unwrap(), b"hello");
    }

    impl Magic {
        /// Test-only: the merge-and-sort half of [`Magic::load_from`],
        /// without writing files to disk to exercise it.
        fn load_from_parsed(parts: Vec<Magic>) -> Magic {
            let mut magic = Magic::default();
            for part in parts {
                magic.sections.extend(part.sections);
            }
            magic.sections.sort_by_key(|section| std::cmp::Reverse(section.priority));
            magic
        }
    }
}
