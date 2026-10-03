//! What the search box understands: free text, and filters written as
//! `key:value` — mockup `1e`'s chips.
//!
//! ```text
//! report ext:pdf modified:<30d          a PDF called *report*, from this month
//! kind:image size:>5M                   big pictures
//! -kind:folder name:*.tar.*             files only, named like a tarball
//! ```
//!
//! Pure, like [`crate::jump`]: no clock, no filesystem. Parsing is
//! [`parse`]; deciding whether an entry matches is a [`Matcher`], which is
//! a [`Query`] with its relative times ("seven days ago", "today") pinned
//! to one instant. That split is what lets a recursive search over a
//! hundred thousand entries ask every one of them the same question,
//! rather than each comparison reading the clock again and the boundary
//! drifting through the walk — and it is what lets a test say what "now"
//! is.
//!
//! # Plain text is what it always was
//!
//! A query with no filters in it matches exactly as the search box did
//! before any of this existed: the whole text, lowercased, as a
//! substring of the name. `foo  bar` with two spaces still means two
//! spaces. Only once a filter is present does the rest of the text get
//! split into words, because there is no other sensible reading of
//! `foo ext:rs bar`.
//!
//! # A key nobody knows is said, not swallowed
//!
//! `colour:red` is not a filter. Ignoring it would make the box quietly
//! do less than it was asked, and treating it as nothing would match
//! everything — both are the "could not read it, so there is nothing
//! there" mistake CLAUDE.md names. It is searched as the literal text
//! `colour:red` (a file really can be called that) *and* reported as a
//! [`Problem`], which the view puts beside the field. A filter whose key
//! is known and whose value is not (`size:huge`) is reported and not
//! applied: there is no literal reading of it anyone meant.
//!
//! Only a token that *looks* like a filter is treated as one: letters
//! before the colon and something after it. `12:30`, `Re:` and `C:` are
//! text.
//!
//! # Git is not here
//!
//! Mockup `1e` draws a `git:dirty` chip. Git is deferred out of every
//! phase (see `crates/hyprforge-files/DESIGN.md`, decision 3), so `git:`
//! is an unknown key like any other until that subsystem exists.

use crate::types::{Entry, EntryKind, EntrySize};
use std::time::{Duration, SystemTime};

/// Every key this module understands, in the order a hint lists them.
pub const KEYS: [&str; 6] = ["ext", "kind", "size", "modified", "name", "is"];

/// What was typed into the search box, understood.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Query {
    /// The free text, lowercased — matched as a substring of the name.
    /// Empty when there is none.
    pub text: String,
    /// Every filter, in the order typed. All must hold.
    pub filters: Vec<Filter>,
    /// What could not be understood — see the module doc.
    pub problems: Vec<Problem>,
}

/// One `key:value`, understood. What a chip draws.
#[derive(Debug, Clone, PartialEq)]
pub struct Filter {
    /// Written with a leading `-`: the entry must *not* match.
    pub negated: bool,
    pub test: Test,
    /// The token as typed, `-` included — the chip's label, and how a
    /// saved search writes it back.
    pub source: String,
}

/// What a filter asks of an entry.
#[derive(Debug, Clone, PartialEq)]
pub enum Test {
    /// The extension is one of these — lowercased, no dot. `tar.gz`
    /// matches `x.tar.gz`; `gz` matches it too.
    Ext(Vec<String>),
    /// The kind is one of these.
    Kind(Vec<Kind>),
    /// A file's size, compared. A folder never matches: its size is a
    /// count of things in it, not bytes.
    Size(SizeTest),
    /// When it was last modified.
    Modified(When),
    /// The name matches this — a substring, or a glob when it holds `*`
    /// or `?`. Lowercased.
    Name(String),
    /// A dotfile. Asking for one shows it whatever the dotfile switch
    /// says, and lets a recursive search into hidden folders.
    Hidden,
    /// A symbolic link.
    Link,
}

/// `kind:` — [`EntryKind`] as a person names it, plus `file`, which is
/// "anything but a folder".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Folder,
    File,
    Image,
    Video,
    Audio,
    Document,
    Archive,
    Code,
}

impl Kind {
    const ALL: [(&'static str, Kind); 8] = [
        ("folder", Kind::Folder),
        ("file", Kind::File),
        ("image", Kind::Image),
        ("video", Kind::Video),
        ("audio", Kind::Audio),
        ("document", Kind::Document),
        ("archive", Kind::Archive),
        ("code", Kind::Code),
    ];

    fn from_word(word: &str) -> Option<Kind> {
        // A few plurals and near-misses people type without thinking:
        // `kind:images`, `kind:dir`, `kind:doc`.
        let word = match word {
            "dir" | "directory" | "folders" | "dirs" => "folder",
            "files" => "file",
            "picture" | "pictures" | "photo" | "photos" | "images" => "image",
            "videos" | "movie" | "movies" => "video",
            "music" | "sound" => "audio",
            "doc" | "docs" | "documents" => "document",
            "archives" => "archive",
            "source" => "code",
            other => other,
        };
        Kind::ALL.iter().find(|(name, _)| *name == word).map(|(_, kind)| *kind)
    }

    fn holds(self, entry: &Entry) -> bool {
        match self {
            Kind::Folder => entry.is_dir,
            Kind::File => !entry.is_dir,
            Kind::Image => entry.kind == EntryKind::Image,
            Kind::Video => entry.kind == EntryKind::Video,
            Kind::Audio => entry.kind == EntryKind::Audio,
            Kind::Document => entry.kind == EntryKind::Document,
            Kind::Archive => entry.kind == EntryKind::Archive,
            Kind::Code => entry.kind == EntryKind::Code,
        }
    }
}

/// `size:>10M`, as numbers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SizeTest {
    pub cmp: Cmp,
    pub bytes: u64,
    /// The unit it was written in, in bytes — how wide [`Cmp::About`]'s
    /// range is.
    pub unit: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cmp {
    Less,
    LessOrEqual,
    Greater,
    GreaterOrEqual,
    /// No operator written. For a size, "exactly" would match no real
    /// file, so it means a range one unit wide from the number given:
    /// `size:2M` is 2 MiB up to (not including) 3 MiB — the files the
    /// Size column prints as "2.x MiB".
    About,
}

/// `modified:` — relative to the moment a [`Matcher`] is made.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum When {
    /// `<7d`: within the last seven days.
    Within(Duration),
    /// `>30d`: longer ago than thirty days.
    OlderThan(Duration),
    /// Since local midnight.
    Today,
    /// The local day before today.
    Yesterday,
}

/// A part of the query that was not understood.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Problem {
    /// `colour:red`: not a key. Searched as text instead.
    UnknownKey { key: String },
    /// `size:huge`: a key, with a value it cannot take. Not applied.
    BadValue { key: String, value: String },
}

impl Problem {
    /// One sentence for the view, saying what happened and what would
    /// work instead.
    pub fn message(&self) -> String {
        match self {
            Problem::UnknownKey { key } => format!(
                "\u{201c}{key}:\u{201d} isn't a filter, so it was searched as text. Filters: {}.",
                KEYS.iter().map(|k| format!("{k}:")).collect::<Vec<_>>().join(" ")
            ),
            Problem::BadValue { key, value } => format!("\u{201c}{key}:{value}\u{201d} isn't applied \u{2014} {}", expected(key)),
        }
    }
}

/// What each key takes, for the hint beside a value it could not.
fn expected(key: &str) -> &'static str {
    match key {
        "ext" => "write an extension, like ext:rs or ext:jpg,png.",
        "kind" => "kind takes folder, file, image, video, audio, document, archive or code.",
        "size" => "write a comparison, like size:>10M or size:<500k.",
        "modified" => "write an age, like modified:<7d or modified:>1y, or modified:today.",
        "name" => "write part of a name, or a pattern like name:*.rs.",
        _ => "is takes hidden or link.",
    }
}

impl Query {
    /// Nothing to search for: every entry matches.
    pub fn is_empty(&self) -> bool {
        self.text.is_empty() && self.filters.is_empty()
    }

    /// Whether this asks for dotfiles by name — `is:hidden`, not negated.
    pub fn wants_hidden(&self) -> bool {
        self.filters.iter().any(|f| !f.negated && f.test == Test::Hidden)
    }

    /// The query written back out the way it would be typed: filters
    /// first, in order, then the free text. What a saved search stores.
    pub fn to_text(&self, raw_text: &str) -> String {
        let mut parts: Vec<&str> = self.filters.iter().map(|f| f.source.as_str()).collect();
        let raw = raw_text.trim();
        if !raw.is_empty() {
            parts.push(raw);
        }
        parts.join(" ")
    }

    /// [`Self::matcher`], pinned to the moment it is called.
    pub fn matcher_now(&self) -> Matcher {
        self.matcher(chrono::Local::now())
    }

    /// This query with its relative times pinned to `now`.
    pub fn matcher(&self, now: chrono::DateTime<chrono::Local>) -> Matcher {
        Matcher {
            text: self.text.clone(),
            hidden: self.wants_hidden(),
            filters: self
                .filters
                .iter()
                .map(|f| (f.negated, Pinned::of(&f.test, now)))
                .collect(),
        }
    }
}

/// Reads `text` — see the module doc for the rules.
pub fn parse(text: &str) -> Query {
    let mut filters = Vec::new();
    let mut problems = Vec::new();
    let mut words: Vec<String> = Vec::new();
    for token in tokens(text) {
        match read_token(&token) {
            Read::Text => words.push(token),
            Read::Filter(filter) => filters.push(filter),
            Read::Unknown(problem) => {
                problems.push(problem);
                words.push(token);
            }
            Read::Bad(problem) => problems.push(problem),
        }
    }
    // No filters: the text exactly as typed, so a plain search behaves as
    // it always has — see the module doc.
    let text = if filters.is_empty() && problems.is_empty() {
        text.to_lowercase()
    } else {
        words.join(" ").to_lowercase()
    };
    Query { text, filters, problems }
}

/// Takes every *finished* filter out of `typed` — one followed by a
/// space — and hands back those filters and what is left to stay in the
/// field. How a typed token becomes a chip: the field keeps only what is
/// still being written.
///
/// The token being typed (no space after it yet) stays in the field, so
/// `ext:r` does not become a chip before the `s` arrives. Text, unknown
/// keys and values that did not parse stay too — they are not chips, and
/// moving them would hide what needs fixing.
pub fn take_chips(typed: &str) -> (Vec<Filter>, String) {
    let mut chips = Vec::new();
    let mut rest = String::new();
    let mut remaining = typed;
    while let Some(start) = remaining.find(|c: char| !c.is_whitespace()) {
        let (gap, after) = remaining.split_at(start);
        let (token, tail) = split_token(after);
        let finished = tail.starts_with(char::is_whitespace);
        match read_token(token) {
            Read::Filter(filter) if finished => {
                chips.push(filter);
                // The space after a chip goes with it; the one before it
                // stays with the text it separated.
                remaining = &tail[tail.chars().next().map_or(0, char::len_utf8)..];
                rest.push_str(gap);
                continue;
            }
            _ => {
                rest.push_str(gap);
                rest.push_str(token);
            }
        }
        remaining = tail;
    }
    rest.push_str(remaining);
    // A run of spaces left where a chip came out of the middle reads as
    // a typo; one is what was meant.
    if !chips.is_empty() {
        rest = collapse_spaces(&rest);
    }
    (chips, rest)
}

/// Every filter in `text` as a chip, finished or not, and the text that
/// is left — for loading a saved search, where nothing is still being
/// typed.
pub fn all_chips(text: &str) -> (Vec<Filter>, String) {
    let (chips, rest) = take_chips(&format!("{text} "));
    (chips, rest.trim_end().to_string())
}

fn collapse_spaces(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut last_space = false;
    for c in text.chars() {
        if c == ' ' {
            if !last_space {
                out.push(c);
            }
            last_space = true;
        } else {
            out.push(c);
            last_space = false;
        }
    }
    out.trim_start().to_string()
}

/// The first token of `text` (which starts with a non-space) and what
/// follows it. A value may be quoted to hold spaces: `name:"to do"`.
fn split_token(text: &str) -> (&str, &str) {
    let mut in_quotes = false;
    for (i, c) in text.char_indices() {
        if c == '"' {
            in_quotes = !in_quotes;
        } else if c.is_whitespace() && !in_quotes {
            return text.split_at(i);
        }
    }
    (text, "")
}

fn tokens(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut remaining = text;
    while let Some(start) = remaining.find(|c: char| !c.is_whitespace()) {
        let (token, tail) = split_token(&remaining[start..]);
        out.push(token.to_string());
        remaining = tail;
    }
    out
}

enum Read {
    Text,
    Filter(Filter),
    Unknown(Problem),
    Bad(Problem),
}

fn read_token(token: &str) -> Read {
    let (negated, body) = match token.strip_prefix('-') {
        Some(rest) if rest.contains(':') => (true, rest),
        _ => (false, token),
    };
    let Some((key, value)) = body.split_once(':') else { return Read::Text };
    // Letters before the colon and something after it, or it is text:
    // `12:30`, `Re:`, `C:`, `http://` all stay words. `http` is letters,
    // but `//x` is a value, so a pasted URL is reported as an unknown key
    // and still searched for literally — the honest reading.
    if key.len() < 2 || !key.chars().all(|c| c.is_ascii_alphabetic()) || value.is_empty() {
        return Read::Text;
    }
    let key = key.to_ascii_lowercase();
    let value = value.trim_matches('"');
    let test = match key.as_str() {
        "ext" => ext_values(value).map(Test::Ext),
        "kind" | "type" => kinds(value).map(Test::Kind),
        "size" => size(value).map(Test::Size),
        "modified" | "mod" | "date" => when(value).map(Test::Modified),
        "name" => (!value.is_empty()).then(|| Test::Name(value.to_lowercase())),
        "is" => match value.to_ascii_lowercase().as_str() {
            "hidden" | "dotfile" => Some(Test::Hidden),
            "link" | "symlink" => Some(Test::Link),
            _ => None,
        },
        _ => return Read::Unknown(Problem::UnknownKey { key }),
    };
    match test {
        Some(test) => Read::Filter(Filter { negated, test, source: token.to_string() }),
        None => Read::Bad(Problem::BadValue { key, value: value.to_string() }),
    }
}

/// `rs`, `.rs`, `jpg,png`, `jpg|png`.
fn ext_values(value: &str) -> Option<Vec<String>> {
    let exts: Vec<String> = value
        .split([',', '|'])
        .map(|e| e.trim().trim_start_matches('.').to_lowercase())
        .filter(|e| !e.is_empty())
        .collect();
    (!exts.is_empty()).then_some(exts)
}

/// `image`, `image|video`, `image,video`. One unknown word fails the
/// whole value rather than quietly narrowing it.
fn kinds(value: &str) -> Option<Vec<Kind>> {
    let kinds: Option<Vec<Kind>> =
        value.split([',', '|']).map(|w| Kind::from_word(&w.trim().to_ascii_lowercase())).collect();
    kinds.filter(|k| !k.is_empty())
}

/// `>10M`, `<=500k`, `2G`, `1.5MB`, `100` (bytes). Binary units, because
/// that is what the Size column shows: `10M` is the file the column calls
/// "10.0 MiB".
fn size(value: &str) -> Option<SizeTest> {
    let (cmp, rest) = comparison(value);
    let rest = rest.trim();
    let split = rest.find(|c: char| !(c.is_ascii_digit() || c == '.')).unwrap_or(rest.len());
    let (number, unit) = rest.split_at(split);
    let number: f64 = number.parse().ok()?;
    let unit = match unit.trim().to_ascii_lowercase().as_str() {
        "" | "b" => 1u64,
        "k" | "kb" | "kib" => 1 << 10,
        "m" | "mb" | "mib" => 1 << 20,
        "g" | "gb" | "gib" => 1 << 30,
        "t" | "tb" | "tib" => 1 << 40,
        _ => return None,
    };
    if !number.is_finite() || number < 0.0 {
        return None;
    }
    Some(SizeTest { cmp, bytes: (number * unit as f64) as u64, unit })
}

fn comparison(value: &str) -> (Cmp, &str) {
    for (prefix, cmp) in [(">=", Cmp::GreaterOrEqual), ("<=", Cmp::LessOrEqual), (">", Cmp::Greater), ("<", Cmp::Less), ("=", Cmp::About)] {
        if let Some(rest) = value.strip_prefix(prefix) {
            return (cmp, rest);
        }
    }
    (Cmp::About, value)
}

/// `<7d` (within seven days), `>1y` (older than a year), `today`,
/// `yesterday`. Units: `min`, `h`, `d`, `w`, `mo`, `y` — `m` alone is
/// refused rather than guessed between minutes and months.
fn when(value: &str) -> Option<When> {
    match value.to_ascii_lowercase().as_str() {
        "today" => return Some(When::Today),
        "yesterday" => return Some(When::Yesterday),
        _ => {}
    }
    let (cmp, rest) = comparison(value);
    let rest = rest.trim().to_ascii_lowercase();
    let split = rest.find(|c: char| !c.is_ascii_digit()).unwrap_or(rest.len());
    let (number, unit) = rest.split_at(split);
    let number: u64 = number.parse().ok()?;
    let seconds = match unit {
        "min" | "mins" => 60,
        "h" | "hr" | "hrs" | "hour" | "hours" => 3_600,
        "d" | "day" | "days" => 86_400,
        "w" | "wk" | "week" | "weeks" => 7 * 86_400,
        "mo" | "month" | "months" => 30 * 86_400,
        "y" | "yr" | "year" | "years" => 365 * 86_400,
        _ => return None,
    };
    let age = Duration::from_secs(number.checked_mul(seconds)?);
    match cmp {
        // No operator reads as "within": `modified:7d` is what people
        // type for "the last week".
        Cmp::Less | Cmp::LessOrEqual | Cmp::About => Some(When::Within(age)),
        Cmp::Greater | Cmp::GreaterOrEqual => Some(When::OlderThan(age)),
    }
}

/// A [`Query`] ready to ask entries about — see the module doc.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Matcher {
    text: String,
    hidden: bool,
    filters: Vec<(bool, Pinned)>,
}

/// A [`Test`] with its times resolved.
#[derive(Debug, Clone, PartialEq)]
enum Pinned {
    Ext(Vec<String>),
    Kind(Vec<Kind>),
    Size(SizeTest),
    /// Modified at or after the first, and before the second.
    Between(Option<SystemTime>, Option<SystemTime>),
    Name(NamePattern),
    Hidden,
    Link,
}

#[derive(Debug, Clone, PartialEq)]
enum NamePattern {
    Contains(String),
    Glob(String),
}

impl Pinned {
    fn of(test: &Test, now: chrono::DateTime<chrono::Local>) -> Pinned {
        use chrono::{Days, TimeZone};
        let instant: SystemTime = now.into();
        let midnight = |date: chrono::NaiveDate| -> Option<SystemTime> {
            // `earliest`: on a day that starts in a DST gap, the first
            // instant that exists. `None` only on a calendar chrono
            // cannot place, where the bound is dropped rather than wrong.
            chrono::Local.from_local_datetime(&date.and_hms_opt(0, 0, 0)?).earliest().map(Into::into)
        };
        match test {
            Test::Ext(exts) => Pinned::Ext(exts.clone()),
            Test::Kind(kinds) => Pinned::Kind(kinds.clone()),
            Test::Size(size) => Pinned::Size(*size),
            Test::Modified(When::Within(age)) => Pinned::Between(instant.checked_sub(*age), None),
            Test::Modified(When::OlderThan(age)) => Pinned::Between(None, instant.checked_sub(*age)),
            Test::Modified(When::Today) => Pinned::Between(midnight(now.date_naive()), None),
            Test::Modified(When::Yesterday) => {
                let today = now.date_naive();
                Pinned::Between(today.checked_sub_days(Days::new(1)).and_then(midnight), midnight(today))
            }
            Test::Name(pattern) if pattern.contains(['*', '?']) => Pinned::Name(NamePattern::Glob(pattern.clone())),
            Test::Name(pattern) => Pinned::Name(NamePattern::Contains(pattern.clone())),
            Test::Hidden => Pinned::Hidden,
            Test::Link => Pinned::Link,
        }
    }

    fn holds(&self, entry: &Entry, name: &str) -> bool {
        match self {
            Pinned::Ext(exts) => {
                !entry.is_dir
                    && exts.iter().any(|ext| {
                        name.len() > ext.len() + 1
                            && name.ends_with(ext.as_str())
                            && name.as_bytes()[name.len() - ext.len() - 1] == b'.'
                    })
            }
            Pinned::Kind(kinds) => kinds.iter().any(|k| k.holds(entry)),
            Pinned::Size(SizeTest { cmp, bytes, unit }) => {
                let EntrySize::Bytes(size) = entry.size else { return false };
                match cmp {
                    Cmp::Less => size < *bytes,
                    Cmp::LessOrEqual => size <= *bytes,
                    Cmp::Greater => size > *bytes,
                    Cmp::GreaterOrEqual => size >= *bytes,
                    Cmp::About => size >= *bytes && size < bytes.saturating_add(*unit),
                }
            }
            // An entry whose time the filesystem would not give matches
            // no date filter — it is not "old", it is unknown.
            Pinned::Between(from, until) => entry.modified.is_some_and(|m| {
                from.is_none_or(|from| m >= from) && until.is_none_or(|until| m < until)
            }),
            Pinned::Name(NamePattern::Contains(part)) => name.contains(part.as_str()),
            Pinned::Name(NamePattern::Glob(pattern)) => glob(pattern.as_bytes(), name.as_bytes()),
            Pinned::Hidden => entry.hidden,
            Pinned::Link => entry.is_symlink,
        }
    }
}

impl Matcher {
    /// Whether `entry` is something this query is looking for.
    pub fn matches(&self, entry: &Entry) -> bool {
        if self.text.is_empty() && self.filters.is_empty() {
            return true;
        }
        // Lowercased once per entry, not once per filter that reads it.
        let name = entry.name.to_lowercase();
        (self.text.is_empty() || name.contains(self.text.as_str()))
            && self.filters.iter().all(|(negated, test)| test.holds(entry, &name) != *negated)
    }

    /// Nothing asked: every entry matches.
    pub fn is_empty(&self) -> bool {
        self.text.is_empty() && self.filters.is_empty()
    }

    /// Whether the query asks for dotfiles — see [`Test::Hidden`].
    pub fn wants_hidden(&self) -> bool {
        self.hidden
    }
}

/// `*` any run, `?` one character — the shell's glob, over the whole
/// name. Iterative with one backtrack point, so a pattern of many stars
/// against a long name is linear-ish rather than exponential.
fn glob(pattern: &[u8], name: &[u8]) -> bool {
    let (mut p, mut n) = (0, 0);
    let mut star: Option<(usize, usize)> = None;
    while n < name.len() {
        if p < pattern.len() && (pattern[p] == b'?' || pattern[p] == name[n]) {
            p += 1;
            n += 1;
        } else if p < pattern.len() && pattern[p] == b'*' {
            star = Some((p, n));
            p += 1;
        } else if let Some((sp, sn)) = star {
            p = sp + 1;
            n = sn + 1;
            star = Some((sp, sn + 1));
        } else {
            return false;
        }
    }
    pattern[p..].iter().all(|&c| c == b'*')
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use std::path::PathBuf;

    fn file(name: &str, bytes: u64) -> Entry {
        Entry {
            name: name.to_string(),
            path: PathBuf::from("/d").join(name),
            is_dir: false,
            size: EntrySize::Bytes(bytes),
            modified: None,
            is_symlink: false,
            link_broken: false,
            hidden: name.starts_with('.'),
            kind: EntryKind::classify(false, name),
            mode: 0o644,
            uid: 1000,
            owner: None,
            origin: None,
            packed: None,
        }
    }

    fn folder(name: &str) -> Entry {
        Entry { is_dir: true, size: EntrySize::UNCOUNTED, kind: EntryKind::Folder, ..file(name, 0) }
    }

    fn now() -> chrono::DateTime<chrono::Local> {
        chrono::Local.with_ymd_and_hms(2026, 10, 3, 15, 0, 0).unwrap()
    }

    fn matches(query: &str, entry: &Entry) -> bool {
        parse(query).matcher(now()).matches(entry)
    }

    fn aged(mut entry: Entry, hours: u64) -> Entry {
        entry.modified = Some(SystemTime::from(now()) - Duration::from_secs(hours * 3600));
        entry
    }

    // --- plain text is what it always was --------------------------------

    /// The property the brief names: a bare word searches this folder
    /// exactly as the box did before it understood anything else.
    #[test]
    fn plain_text_is_the_old_case_insensitive_substring() {
        let query = parse("ReadMe");
        assert!(query.filters.is_empty() && query.problems.is_empty());
        assert_eq!(query.text, "readme");
        assert!(matches("ReadMe", &file("README.md", 1)));
        assert!(!matches("ReadMe", &file("notes.md", 1)));
    }

    #[test]
    fn plain_text_keeps_its_spaces_exactly() {
        assert_eq!(parse("foo  bar ").text, "foo  bar ", "two spaces and the trailing one, as typed");
        assert!(matches("my file", &file("my file.txt", 1)));
        assert!(!matches("my file", &file("my-file.txt", 1)));
    }

    #[test]
    fn colons_that_do_not_look_like_a_filter_are_text() {
        for text in ["12:30", "Re:", "C:", "a:b"] {
            let query = parse(text);
            assert!(query.filters.is_empty() && query.problems.is_empty(), "{text} is text");
        }
        assert!(matches("12:30", &file("meeting 12:30.txt", 1)));
    }

    #[test]
    fn an_empty_query_matches_everything() {
        assert!(parse("").is_empty());
        assert!(parse("").matcher(now()).matches(&folder("x")));
    }

    // --- filters ----------------------------------------------------------

    #[test]
    fn ext_matches_the_extension_and_never_a_folder() {
        assert!(matches("ext:rs", &file("main.rs", 1)));
        assert!(matches("ext:.RS", &file("MAIN.RS", 1)), "a dot and capitals are forgiven");
        assert!(!matches("ext:rs", &file("rs", 1)), "a name that is only the extension has none");
        assert!(!matches("ext:rs", &file("cars", 1)), "the dot is part of it");
        assert!(!matches("ext:rs", &folder("x.rs")), "a folder has no extension to filter by");
        assert!(matches("ext:jpg,png", &file("a.png", 1)));
        assert!(matches("ext:jpg|png", &file("a.jpg", 1)));
        assert!(matches("ext:tar.gz", &file("linux.tar.gz", 1)));
        assert!(matches("ext:gz", &file("linux.tar.gz", 1)));
    }

    #[test]
    fn kind_uses_the_listing_s_own_classification() {
        assert!(matches("kind:image", &file("a.png", 1)));
        assert!(matches("kind:images", &file("a.png", 1)), "the plural people type");
        assert!(matches("kind:image|video", &file("a.mp4", 1)));
        assert!(!matches("kind:image", &file("a.txt", 1)));
        assert!(matches("kind:folder", &folder("src")));
        assert!(matches("kind:file", &file("a.txt", 1)));
        assert!(!matches("kind:file", &folder("src")));
        assert!(matches("kind:code", &file("main.rs", 1)));
    }

    #[test]
    fn size_compares_bytes_in_the_units_the_size_column_shows() {
        let ten_mib = 10 << 20;
        assert!(matches("size:>10M", &file("a", ten_mib + 1)));
        assert!(!matches("size:>10M", &file("a", ten_mib)));
        assert!(matches("size:>=10M", &file("a", ten_mib)));
        assert!(matches("size:<1k", &file("a", 1023)));
        assert!(matches("size:<1.5MB", &file("a", 1_500_000)));
        assert!(!matches("size:>0", &folder("a")), "a folder's size is a count, not bytes");
    }

    /// `size:2M` is what the column would print as 2.x MiB — not exactly
    /// 2,097,152 bytes, which no real file ever is.
    #[test]
    fn a_size_with_no_operator_means_what_the_column_rounds_to() {
        assert!(matches("size:2M", &file("a", (2 << 20) + 5000)));
        assert!(!matches("size:2M", &file("a", 3 << 20)));
        assert!(!matches("size:2M", &file("a", (2 << 20) - 1)));
    }

    #[test]
    fn modified_ages_are_measured_from_the_matcher_s_now() {
        assert!(matches("modified:<7d", &aged(file("a", 1), 24)));
        assert!(!matches("modified:<7d", &aged(file("a", 1), 24 * 8)));
        assert!(matches("modified:>30d", &aged(file("a", 1), 24 * 31)));
        assert!(!matches("modified:>30d", &aged(file("a", 1), 24)));
        assert!(matches("modified:7d", &aged(file("a", 1), 1)), "no operator reads as within");
        assert!(matches("modified:<2h", &aged(file("a", 1), 1)));
    }

    #[test]
    fn today_and_yesterday_follow_the_local_calendar() {
        // `now` is 15:00, so ten hours ago is today and twenty is not.
        assert!(matches("modified:today", &aged(file("a", 1), 10)));
        assert!(!matches("modified:today", &aged(file("a", 1), 20)));
        assert!(matches("modified:yesterday", &aged(file("a", 1), 20)));
        assert!(!matches("modified:yesterday", &aged(file("a", 1), 10)));
        assert!(!matches("modified:yesterday", &aged(file("a", 1), 48)));
    }

    /// A file whose time could not be read is not old and not new.
    #[test]
    fn an_unknown_time_matches_no_date_filter_either_way() {
        assert!(!matches("modified:<7d", &file("a", 1)));
        assert!(!matches("modified:>7d", &file("a", 1)));
    }

    #[test]
    fn name_is_a_substring_or_a_glob() {
        assert!(matches("name:port", &file("Report.pdf", 1)));
        assert!(matches("name:*.rs", &file("main.rs", 1)));
        assert!(!matches("name:*.rs", &file("main.rs.bak", 1)), "a glob covers the whole name");
        assert!(matches("name:a?c*", &file("abcdef", 1)));
        assert!(matches("name:\"to do\"", &file("to do list", 1)), "quotes hold a space");
    }

    #[test]
    fn a_leading_minus_turns_a_filter_around() {
        assert!(matches("-kind:folder", &file("a", 1)));
        assert!(!matches("-kind:folder", &folder("a")));
        assert!(!matches("-ext:rs", &file("a.rs", 1)));
    }

    #[test]
    fn every_filter_and_the_text_must_all_hold() {
        let query = "report ext:pdf size:>1k";
        assert!(matches(query, &file("report-2026.pdf", 4096)));
        assert!(!matches(query, &file("report-2026.pdf", 10)));
        assert!(!matches(query, &file("summary.pdf", 4096)));
        assert!(!matches(query, &file("report.txt", 4096)));
    }

    #[test]
    fn is_hidden_and_is_link() {
        assert!(matches("is:hidden", &file(".bashrc", 1)));
        assert!(!matches("is:hidden", &file("bashrc", 1)));
        assert!(parse("is:hidden").wants_hidden());
        assert!(!parse("-is:hidden").wants_hidden(), "asking for no dotfiles is not asking for them");
        let mut link = file("l", 1);
        link.is_symlink = true;
        assert!(matches("is:link", &link));
    }

    // --- what is not understood is said -----------------------------------

    /// The rule from the module doc: an unknown key is reported *and*
    /// searched literally, never dropped and never matching everything.
    #[test]
    fn an_unknown_key_is_reported_and_searched_as_text() {
        let query = parse("colour:red");
        assert_eq!(query.problems, [Problem::UnknownKey { key: "colour".into() }]);
        assert!(query.filters.is_empty());
        assert_eq!(query.text, "colour:red");
        assert!(matches("colour:red", &file("colour:red.txt", 1)));
        assert!(!matches("colour:red", &file("blue.txt", 1)), "not everything");
        assert!(query.problems[0].message().contains("ext:"), "and says what would work");
    }

    /// Git is deferred, so its chip from the mockup is not a filter yet.
    #[test]
    fn git_is_not_a_filter() {
        assert_eq!(parse("git:dirty").problems, [Problem::UnknownKey { key: "git".into() }]);
    }

    #[test]
    fn a_known_key_with_a_value_it_cannot_take_is_reported_and_not_applied() {
        for (text, key) in [("size:huge", "size"), ("kind:spreadsheet", "kind"), ("modified:<3m", "modified"), ("is:big", "is")] {
            let query = parse(text);
            assert!(query.filters.is_empty(), "{text}");
            assert!(matches!(&query.problems[..], [Problem::BadValue { key: k, .. }] if k == key), "{text}");
            assert!(query.matcher(now()).matches(&file("anything", 1)), "{text} narrows nothing");
        }
    }

    // --- chips ------------------------------------------------------------

    #[test]
    fn a_finished_filter_becomes_a_chip_and_leaves_the_field() {
        let (chips, rest) = take_chips("ext:rs ");
        assert_eq!(chips.len(), 1);
        assert_eq!(chips[0].source, "ext:rs");
        assert_eq!(rest, "");
    }

    #[test]
    fn the_token_still_being_typed_stays_in_the_field() {
        let (chips, rest) = take_chips("ext:r");
        assert!(chips.is_empty());
        assert_eq!(rest, "ext:r");
    }

    #[test]
    fn text_around_a_chip_stays_with_one_space_between() {
        let (chips, rest) = take_chips("report ext:pdf notes");
        assert_eq!(chips.iter().map(|c| c.source.as_str()).collect::<Vec<_>>(), ["ext:pdf"]);
        assert_eq!(rest, "report notes");
    }

    #[test]
    fn unknown_keys_and_bad_values_are_not_chips() {
        let (chips, rest) = take_chips("colour:red size:huge ");
        assert!(chips.is_empty());
        assert_eq!(rest, "colour:red size:huge ");
    }

    #[test]
    fn plain_text_is_left_exactly_alone() {
        let (chips, rest) = take_chips("  foo  bar ");
        assert!(chips.is_empty());
        assert_eq!(rest, "  foo  bar ");
    }

    #[test]
    fn a_saved_search_loads_every_filter_as_a_chip() {
        let (chips, rest) = all_chips("ext:rs kind:code main");
        assert_eq!(chips.len(), 2);
        assert_eq!(rest, "main");
        let (chips, rest) = all_chips("modified:today");
        assert_eq!(chips.len(), 1, "the last token is finished too");
        assert_eq!(rest, "");
    }

    #[test]
    fn chips_and_text_write_back_out_as_typed() {
        let mut query = parse("main");
        query.filters = all_chips("ext:rs -kind:folder").0;
        assert_eq!(query.to_text("main "), "ext:rs -kind:folder main");
    }

    #[test]
    fn a_glob_with_many_stars_does_not_blow_up() {
        let name = "a".repeat(200);
        let pattern = "*a".repeat(50) + "b";
        assert!(!glob(pattern.as_bytes(), name.as_bytes()));
    }
}
