//! Renaming several things at once: the rules, and what they would do.
//!
//! Pure, like [`crate::naming`] beside it. A [`Request`] is what the
//! browser hands over when F2 is pressed with more than one thing
//! selected — the items in the order the listing shows them, and every
//! name already beside them. [`preview`] turns that and the [`Rules`] the
//! sheet has set into one [`Row`] per item: the old name, the new one,
//! and the [`Problem`] with it if there is one. The sheet draws the rows;
//! [`Preview::renames`] is what Apply hands on, and it refuses while any
//! row has a problem.
//!
//! Recomputed whole on every keystroke, so it has to stay cheap for a
//! folder of thousands: one pass to transform, one to count names per
//! folder, hash lookups after that. And a regular expression someone is
//! halfway through typing is compiled here, on the UI thread, every
//! time — so its compiled size is capped ([`PATTERN_LIMIT`]) and the
//! `regex` crate's linear-time matching means no pattern can hang the
//! window. An invalid one is an [`Preview::error`] in words, never a
//! panic.
//!
//! What this module does *not* decide is the order the renames run in —
//! a swap (`a` to `b`, `b` to `a`) is legal here, because once the batch
//! is done no two things share a name. Getting there without one landing
//! on the other is `hyprforge_fileops::batch`'s job, for the disk and for
//! an archive's table alike.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

/// One thing to rename.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    pub path: PathBuf,
    /// As the listing shows it.
    pub name: String,
    pub is_dir: bool,
}

/// What the browser hands over to start a bulk rename.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    /// In the order the listing shows them, which is the order numbering
    /// counts in — "ordered by the listing's current sort" is just this.
    pub items: Vec<Item>,
    /// Every path the listing holds, selected or not, shown or not. A
    /// new name that lands on one of these that is not itself moving is
    /// a clash. Dotfiles hidden from view are still names in the folder.
    pub neighbours: Vec<PathBuf>,
}

impl Request {
    /// Where the items are, when they all share one folder — what the
    /// sheet's heading names.
    pub fn folder(&self) -> Option<&Path> {
        let first = self.items.first()?.path.parent()?;
        self.items.iter().all(|i| i.path.parent() == Some(first)).then_some(first)
    }
}

/// Which of the four ways to rename is in use. The fields for every mode
/// are kept while another is showing, so switching to look at one and
/// back loses nothing typed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Mode {
    #[default]
    Replace,
    Add,
    Number,
    Case,
}

/// The modes, in the order the sheet offers them, with their labels.
pub const MODES: [(Mode, &str); 4] =
    [(Mode::Replace, "Find & replace"), (Mode::Add, "Add text"), (Mode::Number, "Number"), (Mode::Case, "Change case")];

/// Where added text goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Place {
    #[default]
    Start,
    End,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CaseChange {
    #[default]
    Lower,
    Upper,
    /// Each word's first letter capital, the rest small.
    Title,
    /// The first letter capital, the rest small.
    Sentence,
}

pub const CASES: [(CaseChange, &str); 4] = [
    (CaseChange::Lower, "lower case"),
    (CaseChange::Upper, "UPPER CASE"),
    (CaseChange::Title, "Title Case"),
    (CaseChange::Sentence, "Sentence case"),
];

/// Everything the sheet lets a person set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rules {
    pub mode: Mode,
    pub find: String,
    pub replace_with: String,
    pub match_case: bool,
    /// `find` is a regular expression, and `replace_with` may name its
    /// groups (`$1`, `${year}`). Off, both are taken literally — a `$`
    /// in a replacement is a dollar sign.
    pub regex: bool,
    pub add: String,
    pub add_at: Place,
    /// See [`parse_template`].
    pub template: String,
    /// Text, because that is what the field holds; a value that does
    /// not parse is a [`Preview::error`], not a silent default.
    pub start: String,
    pub step: String,
    pub case: CaseChange,
    /// Whether the rule reaches the extension too. Off by default:
    /// renaming `IMG_0001.jpg` to `Holiday 001` and losing what kind of
    /// file it is is the mistake bulk rename tools are known for.
    pub whole_name: bool,
}

impl Default for Rules {
    fn default() -> Self {
        Rules {
            mode: Mode::default(),
            find: String::new(),
            replace_with: String::new(),
            match_case: false,
            regex: false,
            add: String::new(),
            add_at: Place::default(),
            template: "{name} {n}".to_string(),
            start: "1".to_string(),
            step: "1".to_string(),
            case: CaseChange::default(),
            whole_name: false,
        }
    }
}

/// The largest a compiled pattern may be, in bytes — the `regex` crate's
/// own default is 10 MB, and a pattern for a file name that needs more
/// than a megabyte is a mistake (`a{1000}{1000}`), not a rename.
pub const PATTERN_LIMIT: usize = 1 << 20;

/// The widest a number may be padded to. `{n:999999999}` would otherwise
/// allocate a gigabyte per row on the keystroke that typed it.
pub const MAX_WIDTH: usize = 32;

/// The longest name Linux filesystems hold, in bytes (`NAME_MAX`).
pub const NAME_MAX: usize = 255;

/// What is wrong with one row's new name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Problem {
    Empty,
    Slash,
    Nul,
    /// `.` or `..`.
    Reserved,
    TooLong,
    /// It would start with a dot, and so vanish from every listing that
    /// hides dotfiles — this one by default. Almost always a stray `.`
    /// in an "add text" box rather than an intent; and someone who
    /// really means it can rename the one file by hand.
    Hidden,
    /// Something already in the folder, and not being renamed away, has
    /// this name.
    Taken,
    /// Another row would get the same name.
    Duplicate,
    /// The old name is not valid text, so a new one built from it would
    /// not be the same bytes with a change — it would be a different
    /// name with replacement characters in it.
    NotText,
}

impl Problem {
    /// The words under the row.
    pub fn describe(&self) -> &'static str {
        match self {
            Problem::Empty => "No name left",
            Problem::Slash => "A name can't contain \u{201C}/\u{201D}",
            Problem::Nul => "A name can't contain a null character",
            Problem::Reserved => "\u{201C}.\u{201D} and \u{201C}..\u{201D} are reserved",
            Problem::TooLong => "Longer than a name can be (255 bytes)",
            Problem::Hidden => "Would start with \u{201C}.\u{201D} and be hidden",
            Problem::Taken => "Something here already has this name",
            Problem::Duplicate => "Another item would get this name too",
            Problem::NotText => "This name isn't readable text, so it can't be changed here",
        }
    }
}

/// One item's line in the preview.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    pub from: PathBuf,
    pub old: String,
    pub new: String,
    pub problem: Option<Problem>,
}

impl Row {
    pub fn changed(&self) -> bool {
        self.old != self.new
    }
}

/// What the rules would do to the request.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Preview {
    pub rows: Vec<Row>,
    /// The rules themselves are wrong — a pattern that does not compile,
    /// a template with an unknown field, a start that is not a number.
    /// Every row is then shown unchanged, since there is no rule to
    /// apply.
    pub error: Option<String>,
}

impl Preview {
    /// How many rows would change.
    pub fn changes(&self) -> usize {
        self.rows.iter().filter(|r| r.changed()).count()
    }

    /// How many rows have a problem.
    pub fn problems(&self) -> usize {
        self.rows.iter().filter(|r| r.problem.is_some()).count()
    }

    /// The renames to carry out, as `(from, to)`, unchanged rows left
    /// out — or the sentence saying why not.
    pub fn renames(&self) -> Result<Vec<(PathBuf, PathBuf)>, String> {
        if let Some(error) = &self.error {
            return Err(error.clone());
        }
        match self.problems() {
            0 => {}
            1 => return Err("One name has a problem.".to_string()),
            n => return Err(format!("{n} names have problems.")),
        }
        let renames: Vec<_> =
            self.rows.iter().filter(|r| r.changed()).map(|r| (r.from.clone(), r.from.with_file_name(&r.new))).collect();
        if renames.is_empty() {
            return Err("Nothing would change.".to_string());
        }
        Ok(renames)
    }
}

/// What `rules` would do to every item in `request`.
pub fn preview(request: &Request, rules: &Rules) -> Preview {
    let transform = match compile(rules) {
        Ok(t) => t,
        Err(error) => {
            let rows = request
                .items
                .iter()
                .map(|i| Row { from: i.path.clone(), old: i.name.clone(), new: i.name.clone(), problem: None })
                .collect();
            return Preview { rows, error: Some(error) };
        }
    };

    let mut rows: Vec<Row> = request
        .items
        .iter()
        .enumerate()
        .map(|(index, item)| {
            let new = transform.apply(item, index, rules.whole_name);
            Row { from: item.path.clone(), old: item.name.clone(), new, problem: None }
        })
        .collect();

    // What each folder will still hold under its own name afterwards:
    // everything not selected, and every selected row that does not
    // change. Keyed by folder, because a search's results can come from
    // several.
    let selected: HashSet<&Path> = request.items.iter().map(|i| i.path.as_path()).collect();
    let mut stays: HashMap<Option<&Path>, HashSet<String>> = HashMap::new();
    for path in &request.neighbours {
        if selected.contains(path.as_path()) {
            continue;
        }
        if let Some(name) = path.file_name() {
            stays.entry(path.parent()).or_default().insert(name.to_string_lossy().into_owned());
        }
    }
    for row in rows.iter().filter(|r| !r.changed()) {
        stays.entry(row.from.parent()).or_default().insert(row.old.clone());
    }
    let mut arriving: HashMap<(Option<&Path>, &str), usize> = HashMap::new();
    for row in rows.iter().filter(|r| r.changed()) {
        *arriving.entry((row.from.parent(), row.new.as_str())).or_default() += 1;
    }

    let problems: Vec<Option<Problem>> = rows
        .iter()
        .zip(&request.items)
        .map(|(row, item)| {
            if !row.changed() {
                return None;
            }
            if item.path.file_name().and_then(|n| n.to_str()).is_none() {
                return Some(Problem::NotText);
            }
            if let Some(problem) = shape_problem(&row.old, &row.new) {
                return Some(problem);
            }
            let folder = row.from.parent();
            if stays.get(&folder).is_some_and(|names| names.contains(&row.new)) {
                return Some(Problem::Taken);
            }
            if arriving.get(&(folder, row.new.as_str())).copied().unwrap_or(0) > 1 {
                return Some(Problem::Duplicate);
            }
            None
        })
        .collect();
    for (row, problem) in rows.iter_mut().zip(problems) {
        row.problem = problem;
    }
    Preview { rows, error: None }
}

/// What is wrong with `new` on its own, before any other name is
/// considered.
fn shape_problem(old: &str, new: &str) -> Option<Problem> {
    if new.is_empty() {
        Some(Problem::Empty)
    } else if new.contains('/') {
        Some(Problem::Slash)
    } else if new.contains('\0') {
        Some(Problem::Nul)
    } else if new == "." || new == ".." {
        Some(Problem::Reserved)
    } else if new.len() > NAME_MAX {
        Some(Problem::TooLong)
    } else if new.starts_with('.') && !old.starts_with('.') {
        Some(Problem::Hidden)
    } else {
        None
    }
}

/// `name` as the part a rule changes and the part it leaves alone: the
/// name before its last extension, and the extension with its dot. A
/// folder, a dotfile with nothing after the dot and a name with no dot
/// are all subject — the same split the single rename's selection uses
/// ([`crate::naming::stem_len`]), so "the extension" means one thing in
/// this app.
pub fn split(name: &str, is_dir: bool, whole_name: bool) -> (&str, &str) {
    if whole_name || is_dir {
        return (name, "");
    }
    match name.rsplit_once('.') {
        Some((stem, _)) if !stem.is_empty() => name.split_at(stem.len()),
        _ => (name, ""),
    }
}

/// A rule, compiled once per preview rather than once per row.
enum Transform {
    Same,
    Replace { pattern: regex::Regex, with: String, literal: bool },
    Add { text: String, at: Place },
    Number { template: Vec<Piece>, start: u64, step: u64 },
    Case(CaseChange),
}

impl Transform {
    fn apply(&self, item: &Item, index: usize, whole_name: bool) -> String {
        let (subject, kept) = split(&item.name, item.is_dir, whole_name);
        let changed = match self {
            Transform::Same => return item.name.clone(),
            Transform::Replace { pattern, with, literal: true } => {
                pattern.replace_all(subject, regex::NoExpand(with)).into_owned()
            }
            Transform::Replace { pattern, with, literal: false } => pattern.replace_all(subject, with.as_str()).into_owned(),
            Transform::Add { text, at: Place::Start } => format!("{text}{subject}"),
            Transform::Add { text, at: Place::End } => format!("{subject}{text}"),
            Transform::Number { template, start, step } => {
                // Saturating: a start near u64::MAX is legal input and
                // must not panic in a debug build on the row that
                // overflows. The duplicates it produces are flagged.
                let n = start.saturating_add(step.saturating_mul(index as u64));
                expand(template, n, subject)
            }
            Transform::Case(change) => change_case(subject, *change),
        };
        format!("{changed}{kept}")
    }
}

fn compile(rules: &Rules) -> Result<Transform, String> {
    match rules.mode {
        Mode::Replace => {
            if rules.find.is_empty() {
                return Ok(Transform::Same);
            }
            let source = if rules.regex { rules.find.clone() } else { regex::escape(&rules.find) };
            let pattern = regex::RegexBuilder::new(&source)
                .case_insensitive(!rules.match_case)
                .size_limit(PATTERN_LIMIT)
                .dfa_size_limit(PATTERN_LIMIT)
                .build()
                .map_err(pattern_error)?;
            Ok(Transform::Replace { pattern, with: rules.replace_with.clone(), literal: !rules.regex })
        }
        Mode::Add => Ok(if rules.add.is_empty() {
            Transform::Same
        } else {
            Transform::Add { text: rules.add.clone(), at: rules.add_at }
        }),
        Mode::Number => {
            let template = parse_template(&rules.template)?;
            let start = rules
                .start
                .trim()
                .parse::<u64>()
                .map_err(|_| "Start at has to be a whole number, 0 or more.".to_string())?;
            let step = match rules.step.trim().parse::<u64>() {
                Ok(0) | Err(_) => return Err("Step has to be a whole number, 1 or more.".to_string()),
                Ok(step) => step,
            };
            Ok(Transform::Number { template, start, step })
        }
        Mode::Case => Ok(Transform::Case(rules.case)),
    }
}

/// A `regex` error as one sentence. The crate's own message draws a caret
/// under the pattern across several lines, which only lines up in a
/// fixed-width font and a terminal; the last line is the reason.
fn pattern_error(error: regex::Error) -> String {
    match error {
        regex::Error::CompiledTooBig(_) => "That pattern is too large.".to_string(),
        other => {
            let message = other.to_string();
            let reason = message
                .lines()
                .rev()
                .find_map(|l| l.trim().strip_prefix("error: "))
                .unwrap_or("it can't be read")
                .to_string();
            format!("That isn't a valid pattern: {reason}.")
        }
    }
}

/// One piece of a numbering template.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Piece {
    Text(String),
    /// The number, zero-padded to `width` digits (0 for no padding).
    Number { width: usize },
    /// The old name, without its extension unless the whole name is
    /// being renamed.
    Name,
}

/// Reads a numbering template: text, with `{n}` for the number, `{n:3}`
/// or `{n:03}` for the number padded with zeros to three digits, and
/// `{name}` for the old name. `{{` and `}}` are literal braces.
///
/// Padding is always with zeros: a file name padded with spaces sorts
/// the same and reads worse, and nobody asks for it.
pub fn parse_template(template: &str) -> Result<Vec<Piece>, String> {
    let mut pieces = Vec::new();
    let mut text = String::new();
    let mut chars = template.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '{' if chars.peek() == Some(&'{') => {
                chars.next();
                text.push('{');
            }
            '}' if chars.peek() == Some(&'}') => {
                chars.next();
                text.push('}');
            }
            '}' => return Err("A \u{201C}}\u{201D} has no \u{201C}{\u{201D} before it — write \u{201C}}}\u{201D} for a brace.".to_string()),
            '{' => {
                let mut field = String::new();
                loop {
                    match chars.next() {
                        Some('}') => break,
                        Some(c) => field.push(c),
                        None => return Err("A \u{201C}{\u{201D} is never closed.".to_string()),
                    }
                }
                if !text.is_empty() {
                    pieces.push(Piece::Text(std::mem::take(&mut text)));
                }
                pieces.push(parse_field(&field)?);
            }
            c => text.push(c),
        }
    }
    if !text.is_empty() {
        pieces.push(Piece::Text(text));
    }
    Ok(pieces)
}

fn parse_field(field: &str) -> Result<Piece, String> {
    match field.split_once(':') {
        None if field == "n" => Ok(Piece::Number { width: 0 }),
        None if field == "name" => Ok(Piece::Name),
        Some(("n", width)) => match width.parse::<usize>() {
            Ok(width) if width <= MAX_WIDTH => Ok(Piece::Number { width }),
            Ok(_) => Err(format!("A number can be padded to {MAX_WIDTH} digits at most.")),
            Err(_) => Err(format!("\u{201C}{{n:{width}}}\u{201D} needs a width, like {{n:3}}.")),
        },
        _ => Err(format!("\u{201C}{{{field}}}\u{201D} isn't something a name can hold — use {{n}} or {{name}}.")),
    }
}

fn expand(template: &[Piece], n: u64, name: &str) -> String {
    let mut out = String::new();
    for piece in template {
        match piece {
            Piece::Text(text) => out.push_str(text),
            Piece::Number { width } => out.push_str(&format!("{n:0width$}")),
            Piece::Name => out.push_str(name),
        }
    }
    out
}

/// `text` in the case asked for. A "word" starts after anything that is
/// not a letter or digit — space, `_`, `-`, `.` — so `my_holiday-pics`
/// becomes `My_Holiday-Pics` in title case.
pub fn change_case(text: &str, change: CaseChange) -> String {
    match change {
        CaseChange::Lower => text.to_lowercase(),
        CaseChange::Upper => text.to_uppercase(),
        CaseChange::Title => {
            let mut out = String::with_capacity(text.len());
            let mut word_start = true;
            for c in text.chars() {
                if word_start {
                    out.extend(c.to_uppercase());
                } else {
                    out.extend(c.to_lowercase());
                }
                word_start = !c.is_alphanumeric();
            }
            out
        }
        CaseChange::Sentence => {
            let mut out = String::with_capacity(text.len());
            let mut seen_letter = false;
            for c in text.chars() {
                if !seen_letter && c.is_alphabetic() {
                    out.extend(c.to_uppercase());
                    seen_letter = true;
                } else {
                    out.extend(c.to_lowercase());
                }
            }
            out
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(name: &str) -> Item {
        Item { path: PathBuf::from("/d").join(name), name: name.to_string(), is_dir: false }
    }

    fn folder(name: &str) -> Item {
        Item { path: PathBuf::from("/d").join(name), name: name.to_string(), is_dir: true }
    }

    /// A request for `names`, in a folder that also holds `others`.
    fn request(names: &[&str], others: &[&str]) -> Request {
        let items: Vec<Item> = names.iter().map(|n| item(n)).collect();
        let neighbours =
            items.iter().map(|i| i.path.clone()).chain(others.iter().map(|n| PathBuf::from("/d").join(n))).collect();
        Request { items, neighbours }
    }

    fn new_names(preview: &Preview) -> Vec<&str> {
        preview.rows.iter().map(|r| r.new.as_str()).collect()
    }

    fn replace(find: &str, with: &str) -> Rules {
        Rules { mode: Mode::Replace, find: find.into(), replace_with: with.into(), ..Rules::default() }
    }

    fn number(template: &str) -> Rules {
        Rules { mode: Mode::Number, template: template.into(), ..Rules::default() }
    }

    // --- the four modes -------------------------------------------------

    #[test]
    fn find_and_replace_ignores_case_unless_asked_not_to() {
        let req = request(&["IMG_1.jpg", "img_2.jpg"], &[]);
        assert_eq!(new_names(&preview(&req, &replace("img_", "Photo "))), ["Photo 1.jpg", "Photo 2.jpg"]);
        let exact = Rules { match_case: true, ..replace("img_", "Photo ") };
        assert_eq!(new_names(&preview(&req, &exact)), ["IMG_1.jpg", "Photo 2.jpg"]);
    }

    /// Off, a pattern is text: a `.` is a dot, and a `$1` in the
    /// replacement is a dollar sign and a one.
    #[test]
    fn plain_find_and_replace_takes_both_fields_literally() {
        let req = request(&["a.b.c.txt", "abc.txt"], &[]);
        assert_eq!(new_names(&preview(&req, &replace(".", "$1"))), ["a$1b$1c.txt", "abc.txt"]);
    }

    #[test]
    fn a_regular_expression_can_reuse_what_it_matched() {
        let req = request(&["2024-05-01 trip.jpg"], &[]);
        let rules = Rules { regex: true, ..replace(r"(\d{4})-(\d{2})-(\d{2})", "$3.$2.$1") };
        assert_eq!(new_names(&preview(&req, &rules)), ["01.05.2024 trip.jpg"]);
    }

    /// Typing `(` on the way to `(\d+)` is an error in words and every
    /// row unchanged — never a panic, never a half-applied rule.
    #[test]
    fn an_invalid_pattern_is_an_error_in_words_and_changes_nothing() {
        let req = request(&["a.txt"], &[]);
        let p = preview(&req, &Rules { regex: true, ..replace("(", "x") });
        let error = p.error.as_deref().expect("an error");
        assert!(error.starts_with("That isn't a valid pattern"), "{error}");
        assert!(!error.contains('\n'), "one line, not the crate's caret drawing: {error}");
        assert_eq!(p.changes(), 0);
        assert!(p.renames().is_err());
    }

    /// A pattern that compiles to something enormous is refused, rather
    /// than being built on the UI thread on the keystroke that typed it.
    #[test]
    fn a_pattern_too_large_to_compile_is_refused() {
        let req = request(&["a.txt"], &[]);
        let p = preview(&req, &Rules { regex: true, ..replace(r"\w{1000}{1000}", "") });
        assert_eq!(p.error.as_deref(), Some("That pattern is too large."));
    }

    #[test]
    fn an_empty_find_changes_nothing() {
        let req = request(&["a.txt"], &[]);
        let p = preview(&req, &Rules { regex: true, ..replace("", "x") });
        assert_eq!(p.error, None);
        assert_eq!(p.changes(), 0);
    }

    #[test]
    fn added_text_goes_before_the_name_or_before_the_extension() {
        let req = request(&["a.txt", "Makefile"], &[]);
        let start = Rules { mode: Mode::Add, add: "old ".into(), ..Rules::default() };
        assert_eq!(new_names(&preview(&req, &start)), ["old a.txt", "old Makefile"]);
        let end = Rules { add: " (copy)".into(), add_at: Place::End, ..start };
        assert_eq!(new_names(&preview(&req, &end)), ["a (copy).txt", "Makefile (copy)"]);
    }

    #[test]
    fn with_the_whole_name_added_text_goes_after_the_extension() {
        let req = request(&["a.txt"], &[]);
        let rules = Rules { mode: Mode::Add, add: ".bak".into(), add_at: Place::End, whole_name: true, ..Rules::default() };
        assert_eq!(new_names(&preview(&req, &rules)), ["a.txt.bak"]);
    }

    /// The order of the request is the listing's sort, and the numbers
    /// follow it — not the order the items were clicked in.
    #[test]
    fn numbering_counts_in_the_order_the_listing_shows() {
        let req = request(&["c.jpg", "a.jpg", "b.jpg"], &[]);
        assert_eq!(new_names(&preview(&req, &number("Holiday {n:03}"))), ["Holiday 001.jpg", "Holiday 002.jpg", "Holiday 003.jpg"]);
    }

    #[test]
    fn numbering_starts_and_steps_where_it_is_told() {
        let req = request(&["a", "b", "c"], &[]);
        let rules = Rules { start: "10".into(), step: "5".into(), ..number("{n}") };
        assert_eq!(new_names(&preview(&req, &rules)), ["10", "15", "20"]);
    }

    #[test]
    fn a_template_can_keep_the_old_name_and_literal_braces() {
        let req = request(&["trip.jpg"], &[]);
        assert_eq!(new_names(&preview(&req, &number("{n:2} - {name} {{draft}}"))), ["01 - trip {draft}.jpg"]);
    }

    #[test]
    fn a_template_that_cannot_be_read_says_why() {
        let req = request(&["a"], &[]);
        for (bad, says) in [
            ("{x}", "isn't something"),
            ("{n", "never closed"),
            ("a}", "no \u{201C}{\u{201D}"),
            ("{n:abc}", "needs a width"),
            ("{n:9999}", "32 digits"),
        ] {
            let error = preview(&req, &number(bad)).error.unwrap_or_default();
            assert!(error.contains(says), "{bad}: {error}");
        }
    }

    #[test]
    fn a_start_or_step_that_is_not_a_number_is_said_not_assumed() {
        let req = request(&["a"], &[]);
        assert!(preview(&req, &Rules { start: "one".into(), ..number("{n}") }).error.unwrap().contains("Start"));
        assert!(preview(&req, &Rules { step: "0".into(), ..number("{n}") }).error.unwrap().contains("Step"));
    }

    /// A start at the very top of the range must not overflow — and
    /// panic, in a debug build — on the second row.
    #[test]
    fn numbering_at_the_end_of_the_range_does_not_panic() {
        let req = request(&["a", "b"], &[]);
        let p = preview(&req, &Rules { start: u64::MAX.to_string(), ..number("{n}") });
        assert_eq!(p.rows[1].problem, Some(Problem::Duplicate));
    }

    #[test]
    fn case_changes_leave_the_extension_alone() {
        let req = request(&["my_holiday-PICS.JPG"], &[]);
        let case = |c| new_names(&preview(&req, &Rules { mode: Mode::Case, case: c, ..Rules::default() }))[0].to_string();
        assert_eq!(case(CaseChange::Lower), "my_holiday-pics.JPG");
        assert_eq!(case(CaseChange::Upper), "MY_HOLIDAY-PICS.JPG");
        assert_eq!(case(CaseChange::Title), "My_Holiday-Pics.JPG");
        assert_eq!(case(CaseChange::Sentence), "My_holiday-pics.JPG");
    }

    #[test]
    fn with_the_whole_name_the_extension_changes_too() {
        let req = request(&["A.JPG"], &[]);
        let rules = Rules { mode: Mode::Case, case: CaseChange::Lower, whole_name: true, ..Rules::default() };
        assert_eq!(new_names(&preview(&req, &rules)), ["a.jpg"]);
    }

    /// A folder has no extension: `photos.2024` is all name.
    #[test]
    fn a_folder_is_renamed_whole() {
        let req = Request { items: vec![folder("photos.2024")], neighbours: vec![] };
        let rules = Rules { mode: Mode::Add, add: "-old".into(), add_at: Place::End, ..Rules::default() };
        assert_eq!(new_names(&preview(&req, &rules)), ["photos.2024-old"]);
    }

    #[test]
    fn the_extension_is_the_last_dot_and_a_dotfile_has_none() {
        assert_eq!(split("archive.tar.gz", false, false), ("archive.tar", ".gz"));
        assert_eq!(split(".bashrc", false, false), (".bashrc", ""));
        assert_eq!(split("Makefile", false, false), ("Makefile", ""));
        assert_eq!(split("a.txt", false, true), ("a.txt", ""));
    }

    // --- problems -------------------------------------------------------

    #[test]
    fn two_items_given_one_name_are_both_flagged() {
        let req = request(&["a.txt", "b.txt"], &[]);
        let p = preview(&req, &number("same"));
        assert!(p.rows.iter().all(|r| r.problem == Some(Problem::Duplicate)));
        assert_eq!(p.renames(), Err("2 names have problems.".to_string()));
    }

    /// A dotfile hidden from view is still a name in the folder.
    #[test]
    fn a_name_already_in_the_folder_is_flagged() {
        let req = request(&["a.txt", "b.txt"], &[".x.txt", "c.txt"]);
        let p = preview(&req, &replace("b", "c"));
        assert_eq!(p.rows[0].problem, None, "unchanged rows are not problems");
        assert_eq!(p.rows[1].problem, Some(Problem::Taken));
    }

    /// `a` to `b` while `b` goes to `a` is fine: afterwards no two
    /// things share a name. How to get there is the planner's question.
    #[test]
    fn a_swap_is_not_a_clash() {
        let swap = Request { items: vec![item("ab"), item("ba")], neighbours: vec![] };
        let p = preview(&swap, &Rules { regex: true, ..replace("^(.)(.)$", "$2$1") });
        assert_eq!(new_names(&p), ["ba", "ab"]);
        assert_eq!(p.problems(), 0);
        assert_eq!(p.renames().unwrap().len(), 2);
    }

    /// A selected item that does not change keeps its name, so another
    /// row moving onto that name is a clash — even though both are in
    /// the selection.
    #[test]
    fn landing_on_a_selected_item_that_stays_is_a_clash() {
        let req = request(&["a", "b"], &[]);
        let p = preview(&req, &replace("b", "a"));
        assert_eq!(p.rows[0].problem, None);
        assert_eq!(p.rows[1].problem, Some(Problem::Taken));
    }

    #[test]
    fn a_name_a_filesystem_cannot_hold_is_flagged() {
        let req = request(&["a"], &[]);
        let cases = [
            (replace("a", ""), Problem::Empty),
            (replace("a", "x/y"), Problem::Slash),
            (replace("a", "x\0"), Problem::Nul),
            (replace("a", ".."), Problem::Reserved),
            (replace("a", &"x".repeat(256)), Problem::TooLong),
        ];
        for (rules, problem) in cases {
            assert_eq!(preview(&req, &rules).rows[0].problem, Some(problem));
        }
    }

    /// 255 is bytes, not characters: 128 two-byte letters is too long.
    #[test]
    fn the_length_limit_is_in_bytes() {
        let req = request(&["a"], &[]);
        assert_eq!(preview(&req, &replace("a", &"é".repeat(127))).rows[0].problem, None);
        assert_eq!(preview(&req, &replace("a", &"é".repeat(128))).rows[0].problem, Some(Problem::TooLong));
    }

    #[test]
    fn a_name_that_would_become_hidden_is_flagged_and_one_already_hidden_is_not() {
        let req = request(&["notes", ".bashrc"], &[]);
        let p = preview(&req, &Rules { mode: Mode::Add, add: ".".into(), ..Rules::default() });
        assert_eq!(p.rows[0].problem, Some(Problem::Hidden));
        assert_eq!(p.rows[1].problem, None, "{:?}", p.rows[1]);
    }

    /// The same new name in two different folders — a search's results —
    /// is two files, not a clash.
    #[test]
    fn the_same_name_in_two_folders_is_not_a_clash() {
        let items = vec![
            Item { path: "/one/a.txt".into(), name: "a.txt".into(), is_dir: false },
            Item { path: "/two/a.txt".into(), name: "a.txt".into(), is_dir: false },
        ];
        let req = Request { neighbours: items.iter().map(|i| i.path.clone()).collect(), items };
        let p = preview(&req, &replace("a", "b"));
        assert_eq!(p.problems(), 0);
        assert_eq!(req.folder(), None, "no single folder to name");
    }

    #[test]
    fn unchanged_rows_are_left_out_of_what_is_applied() {
        let req = request(&["a.txt", "b.txt"], &[]);
        let renames = preview(&req, &replace("a", "c")).renames().unwrap();
        assert_eq!(renames, vec![(PathBuf::from("/d/a.txt"), PathBuf::from("/d/c.txt"))]);
    }

    #[test]
    fn nothing_to_change_is_not_something_to_apply() {
        let req = request(&["a.txt"], &[]);
        assert_eq!(preview(&req, &replace("zzz", "y")).renames(), Err("Nothing would change.".to_string()));
    }

    /// Recomputed on every keystroke: a folder of ten thousand must stay
    /// well inside a frame. Generous, because a test machine is busy —
    /// what it guards against is a quadratic pass, which at this size is
    /// seconds, not milliseconds.
    #[test]
    fn a_preview_of_ten_thousand_is_quick() {
        let names: Vec<String> = (0..10_000).map(|i| format!("IMG_{i:05}.jpg")).collect();
        let refs: Vec<&str> = names.iter().map(String::as_str).collect();
        let req = request(&refs, &["other.txt"]);
        let started = std::time::Instant::now();
        let p = preview(&req, &number("Holiday {n:05}"));
        assert_eq!(p.problems(), 0);
        assert!(started.elapsed() < std::time::Duration::from_millis(500), "{:?}", started.elapsed());
    }
}
