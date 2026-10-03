//! The fuzzy path bar: `~/pr/hy/src` to `~/projects/hyprsuite/src`.
//!
//! Each segment of what was typed is matched against the names in the
//! folder the previous segment landed on, so a few letters per level are
//! enough to say where you mean — the design's "the single best idea in
//! the document", and the thing neither Finder nor Explorer has.
//!
//! Pure over [`FsBackend`], like everything else under the browser: the
//! host runs [`resolve`] off the UI thread through the backend it already
//! holds, which is also what lets an archive or the Trash be typed as a
//! literal path — `RoutingBackend` answers for those the same way it does
//! for a listing.
//!
//! Two rules shape it, and both are about not surprising someone who
//! typed a real path:
//!
//! - **A path that exists as typed is always the first answer.** Fuzzy
//!   matching is for getting somewhere with fewer keys, never for
//!   second-guessing somebody who spelled it out.
//! - **It is bounded.** Every level reads a few folders, never all of
//!   them — [`Limits`] — because this runs on every keystroke and a
//!   home directory has `target/` folders with ten thousand entries in
//!   them.

use crate::backend::FsBackend;
use crate::types::Entry;
use std::cmp::Ordering;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// What was typed, taken apart: the folder matching starts from, and one
/// pattern per level below it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Query {
    /// Where the first segment is matched — home for `~/…`, the root for
    /// `/…`, and the folder in view for anything else.
    pub base: PathBuf,
    /// One pattern per level. An empty final segment is a trailing `/`:
    /// "show me what is in here", the way a shell completes after a
    /// slash.
    pub segments: Vec<String>,
}

impl Query {
    /// The path this query names if every segment is taken literally.
    pub fn literal(&self) -> PathBuf {
        let mut path = self.base.clone();
        for segment in self.segments.iter().filter(|s| !s.is_empty()) {
            path.push(segment);
        }
        path
    }
}

/// Takes typed text apart into a [`Query`].
///
/// `.` and `..` are resolved here, lexically, rather than matched: they
/// are not names anyone means fuzzily, and `..` matched as a pattern
/// would find every folder with two dots in its name.
pub fn parse(text: &str, current_dir: &Path, home: Option<&Path>) -> Query {
    let text = text.trim();
    let (mut base, rest) = if text == "~" {
        (home.map(Path::to_path_buf).unwrap_or_else(|| PathBuf::from("/")), "")
    } else if let Some(rest) = text.strip_prefix("~/") {
        (home.map(Path::to_path_buf).unwrap_or_else(|| PathBuf::from("/")), rest)
    } else if let Some(rest) = text.strip_prefix('/') {
        (PathBuf::from("/"), rest)
    } else {
        (current_dir.to_path_buf(), text)
    };
    let mut segments: Vec<String> = Vec::new();
    for part in rest.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                if segments.pop().is_none() {
                    base.pop();
                }
            }
            other => segments.push(other.to_string()),
        }
    }
    // A trailing slash asks for that folder's contents — `~/projects/`
    // lists projects, and `~/` lists home — the way a shell completes
    // after one.
    if rest.ends_with('/') || (rest.is_empty() && text.ends_with('/')) {
        segments.push(String::new());
    }
    Query { base, segments }
}

/// How well one name matches one segment, best first. The derived order
/// is the ranking.
///
/// Modelled on the emoji picker's tiers (`hyprforge-emoji`'s `search`):
/// a flat "does it contain these letters" puts `hyprsuite` and `why-hyper`
/// on equal footing for `hy`, which is exactly the "feels random" a
/// matcher should not have.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Tier {
    /// The whole name, ignoring case.
    Exact,
    /// The start of the name.
    Prefix,
    /// The start of a later word in it — `cfg` in `hypr-cfg`,
    /// `store` in `appStore`.
    WordPrefix,
    /// Somewhere inside it.
    Substring,
    /// Its letters in order, starting where the name starts — `hfg` in
    /// `hyprforge`.
    Scattered,
}

impl Tier {
    fn cost(self) -> u32 {
        self as u32
    }
}

/// Scores `name` against one `segment`, or `None` if it does not match.
///
/// An empty segment matches everything, as a prefix — it is what a
/// trailing slash asks for.
///
/// Compared as characters lowercased one for one, rather than as
/// `to_lowercase` strings: a character whose lowercase is two
/// characters would otherwise shift every position after it, and the
/// word starts below are positions.
pub fn score(name: &str, segment: &str) -> Option<Tier> {
    let fold = |c: char| c.to_lowercase().next().unwrap_or(c);
    let name_l: Vec<char> = name.chars().map(fold).collect();
    let seg_l: Vec<char> = segment.chars().map(fold).collect();
    if seg_l.is_empty() {
        return Some(Tier::Prefix);
    }
    if name_l == seg_l {
        return Some(Tier::Exact);
    }
    if name_l.starts_with(&seg_l) {
        return Some(Tier::Prefix);
    }
    if word_starts(name).any(|i| i > 0 && name_l[i..].starts_with(&seg_l)) {
        return Some(Tier::WordPrefix);
    }
    if name_l.windows(seg_l.len()).any(|w| w == seg_l.as_slice()) {
        return Some(Tier::Substring);
    }
    // Scattered letters must start where the name starts. Without that,
    // two letters match nearly every name in a big folder — `hy` would
    // find `why` and `rhythm` — and the beam fills with noise.
    if name_l.first() != seg_l.first() {
        return None;
    }
    let mut wanted = seg_l.iter().skip(1).peekable();
    for c in &name_l[1..] {
        if wanted.peek() == Some(&c) {
            wanted.next();
        }
    }
    wanted.peek().is_none().then_some(Tier::Scattered)
}

/// Character positions where a word begins in `name`: the start, after
/// a separator, and at a lower-to-upper case change (`appStore`).
fn word_starts(name: &str) -> impl Iterator<Item = usize> + '_ {
    let mut previous: Option<char> = None;
    name.chars().enumerate().filter_map(move |(i, c)| {
        let starts = match previous {
            None => true,
            Some(p) => (!p.is_alphanumeric() && c.is_alphanumeric()) || (p.is_lowercase() && c.is_uppercase()),
        };
        previous = Some(c);
        starts.then_some(i)
    })
}

/// How much one resolve may cost.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// How many folders are kept at each level to look inside next.
    pub beam: usize,
    /// How many folders may be read in all, across every level.
    pub max_reads: usize,
    /// How many answers come back.
    pub max_results: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Limits { beam: 6, max_reads: 48, max_results: 8 }
    }
}

/// One place the text could mean.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    pub path: PathBuf,
    pub is_dir: bool,
    /// Exists exactly as typed — see the module doc.
    pub exact: bool,
}

/// Where `query` could take you, best first.
///
/// `known` is the places this window has a reason to think you mean:
/// where you have been, the sidebar's places and pins. Being one, or on
/// the way to one, wins a tie — never more than a tie, so a closer match
/// is never buried under a familiar one.
///
/// Folders only, except at the last level, where a file is an answer
/// too: it means "its folder, with it selected". And never *into* an
/// archive while matching, because listing one means decompressing it;
/// typed out literally, an archive's insides are still found.
pub fn resolve(
    backend: &dyn FsBackend,
    query: &Query,
    known: &HashSet<PathBuf>,
    show_hidden: bool,
    limits: Limits,
) -> Vec<Candidate> {
    let mut out: Vec<Candidate> = Vec::new();
    let literal = query.literal();
    if let Ok(entry) = backend.stat(&literal) {
        out.push(Candidate { path: literal, is_dir: entry.is_dir, exact: true });
    }

    let is_known = |path: &Path| known.iter().any(|k| k.starts_with(path));
    let rank = |a: &Scored, b: &Scored| -> Ordering {
        a.cost
            .cmp(&b.cost)
            .then_with(|| is_known(&b.path).cmp(&is_known(&a.path)))
            // A folder before a file at the same score: this is a bar for
            // going places. Measured: `/u/s/ic` landed on `/usr/sbin/icat`
            // rather than `/usr/share/icons`, by two characters of length.
            .then_with(|| b.is_dir.cmp(&a.is_dir))
            .then_with(|| a.path.as_os_str().len().cmp(&b.path.as_os_str().len()))
            .then_with(|| {
                crate::sort::natural_compare(&a.path.to_string_lossy(), &b.path.to_string_lossy())
            })
    };

    let mut frontier = vec![Scored { path: query.base.clone(), cost: 0, is_dir: true }];
    let mut reads = 0;
    let last = query.segments.len().saturating_sub(1);
    for (level, segment) in query.segments.iter().enumerate() {
        let at_last = level == last;
        let wants_hidden = show_hidden || segment.starts_with('.');
        let mut next: Vec<Scored> = Vec::new();
        for parent in &frontier {
            if reads >= limits.max_reads {
                break;
            }
            reads += 1;
            // One unreadable folder loses its own branch, never the
            // answer: a permission error three levels down is not a
            // reason to say nothing matches.
            let Ok(entries) = backend.read_dir(&parent.path) else { continue };
            next.extend(entries.into_iter().filter_map(|entry: Entry| {
                if entry.hidden && !wants_hidden {
                    return None;
                }
                if !at_last && !entry.is_dir {
                    return None;
                }
                let tier = score(&entry.name, segment)?;
                Some(Scored { path: entry.path, cost: parent.cost + tier.cost(), is_dir: entry.is_dir })
            }));
        }
        next.sort_by(rank);
        next.truncate(if at_last { limits.max_results } else { limits.beam });
        frontier = next;
        if frontier.is_empty() {
            break;
        }
    }

    if !query.segments.is_empty() {
        for scored in frontier {
            if out.len() >= limits.max_results {
                break;
            }
            if out.iter().any(|c| c.path == scored.path) {
                continue;
            }
            out.push(Candidate { path: scored.path, is_dir: scored.is_dir, exact: false });
        }
    }
    out
}

/// One resolve, as the browser hands it to the host — everything
/// [`resolve`] needs except the backend, which is the host's.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    /// The text this answers, exactly as typed, so the answer can be
    /// matched back to it — see `Message::PathResolved`.
    pub text: String,
    pub query: Query,
    pub known: HashSet<PathBuf>,
    pub show_hidden: bool,
}

impl Request {
    /// Runs it. Blocking: the host calls this off the UI thread.
    pub fn run(&self, backend: &dyn FsBackend) -> Vec<Candidate> {
        resolve(backend, &self.query, &self.known, self.show_hidden, Limits::default())
    }
}

/// A path partway through matching, and what matching it has cost.
struct Scored {
    path: PathBuf,
    cost: u32,
    is_dir: bool,
}

/// `path` as the path bar writes it — home as `~` — and as the field
/// is filled when it opens, so editing starts from what was on screen.
pub fn display(path: &Path, home: Option<&Path>) -> String {
    crate::format::tilde_path(path, home)
}

/// The text for completing to `candidate`: its path, and a slash if it
/// is a folder so the next letters go one level down.
pub fn completion(candidate: &Candidate, home: Option<&Path>) -> String {
    let mut text = display(&candidate.path, home);
    if candidate.is_dir && !text.ends_with('/') {
        text.push('/');
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::mock::MockBackend;
    use crate::types::FilesError;
    use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};

    fn home() -> PathBuf {
        PathBuf::from("/home/alex")
    }

    /// A home with a projects tree in it, and a few names chosen to be
    /// near-misses for the patterns the tests type.
    fn tree() -> MockBackend {
        let mock = MockBackend::new();
        let h = home();
        mock.set_home(&h);
        let projects = h.join("projects");
        let prints = h.join("prints");
        mock.seed(
            &h,
            vec![
                MockBackend::dir(&h, "projects"),
                MockBackend::dir(&h, "prints"),
                MockBackend::dir(&h, "Documents"),
                MockBackend::dir(&h, ".config"),
                MockBackend::file(&h, "notes.txt", 1),
            ],
        );
        mock.seed(
            &projects,
            vec![
                MockBackend::dir(&projects, "hyprsuite"),
                MockBackend::dir(&projects, "why-hyper"),
                MockBackend::dir(&projects, "website"),
            ],
        );
        mock.seed(&prints, vec![MockBackend::dir(&prints, "hydra")]);
        mock.seed(h.join("Documents"), vec![]);
        mock.seed(h.join(".config"), vec![MockBackend::dir(&h.join(".config"), "hypr")]);
        mock.seed(projects.join("hyprsuite"), vec![MockBackend::dir(&projects.join("hyprsuite"), "src")]);
        mock.seed(projects.join("why-hyper"), vec![MockBackend::dir(&projects.join("why-hyper"), "src")]);
        mock.seed(prints.join("hydra"), vec![]);
        mock.seed(projects.join("website"), vec![]);
        mock.seed(projects.join("hyprsuite").join("src"), vec![]);
        mock.seed(projects.join("why-hyper").join("src"), vec![]);
        mock
    }

    fn run(mock: &dyn FsBackend, text: &str) -> Vec<PathBuf> {
        let query = parse(text, &home(), Some(&home()));
        resolve(mock, &query, &HashSet::new(), false, Limits::default())
            .into_iter()
            .map(|c| c.path)
            .collect()
    }

    #[test]
    fn each_segment_narrows_from_the_folder_the_last_one_matched() {
        let found = run(&tree(), "~/pr/hy/src");
        assert_eq!(found.first(), Some(&home().join("projects/hyprsuite/src")));
    }

    #[test]
    fn a_path_that_exists_as_typed_is_always_first() {
        // Typed in full, a real path is the answer outright — flagged as
        // the literal one, and not listed a second time as a match.
        let query = parse("~/prints", &home(), Some(&home()));
        let found = resolve(&tree(), &query, &HashSet::new(), false, Limits::default());
        assert_eq!(found[0].path, home().join("prints"));
        assert!(found[0].exact);
        assert_eq!(found.iter().filter(|c| c.path == home().join("prints")).count(), 1, "listed once");
    }

    #[test]
    fn a_prefix_outranks_a_substring_and_a_substring_outranks_a_scattered_match() {
        assert_eq!(score("hyprsuite", "hy"), Some(Tier::Prefix));
        assert_eq!(score("why-hyper", "hy"), Some(Tier::WordPrefix));
        assert_eq!(score("why", "hy"), Some(Tier::Substring));
        assert_eq!(score("whyt", "hyt"), Some(Tier::Substring));
        assert_eq!(score("hyprforge", "hfg"), Some(Tier::Scattered));
        assert!(Tier::Prefix < Tier::WordPrefix);
        assert!(Tier::WordPrefix < Tier::Substring);
        assert!(Tier::Substring < Tier::Scattered);
        // And it decides the order of the answers, not just the score.
        let found = run(&tree(), "~/projects/hy");
        assert_eq!(found[0], home().join("projects/hyprsuite"));
        assert_eq!(found[1], home().join("projects/why-hyper"));
    }

    #[test]
    fn a_word_inside_a_camel_case_name_is_a_word() {
        assert_eq!(score("appStore", "store"), Some(Tier::WordPrefix));
    }

    #[test]
    fn a_scattered_match_must_start_where_the_name_starts() {
        assert_eq!(score("rhythm", "ym"), None, "letters in order, but not from the start");
        assert_eq!(score("hyprforge", "hpf"), Some(Tier::Scattered));
    }

    #[test]
    fn a_folder_you_have_been_to_wins_a_tie() {
        let mock = tree();
        let query = parse("~/p", &home(), Some(&home()));
        // `prints` and `projects` are both prefix matches; unaided, the
        // shorter wins.
        let plain = resolve(&mock, &query, &HashSet::new(), false, Limits::default());
        assert_eq!(plain[0].path, home().join("prints"));
        let known = HashSet::from([home().join("projects/hyprsuite")]);
        let helped = resolve(&mock, &query, &known, false, Limits::default());
        assert_eq!(helped[0].path, home().join("projects"), "on the way to somewhere visited");
    }

    #[test]
    fn being_known_never_beats_a_closer_match() {
        let mock = tree();
        let known = HashSet::from([home().join("projects/why-hyper")]);
        let query = parse("~/projects/hy", &home(), Some(&home()));
        let found = resolve(&mock, &query, &known, false, Limits::default());
        assert_eq!(found[0].path, home().join("projects/hyprsuite"));
    }

    #[test]
    fn dotfolders_are_found_only_when_asked_for() {
        let mock = tree();
        assert!(!run(&mock, "~/co").contains(&home().join(".config")));
        assert!(run(&mock, "~/.co").contains(&home().join(".config")));
        let query = parse("~/co", &home(), Some(&home()));
        let shown = resolve(&mock, &query, &HashSet::new(), true, Limits::default());
        assert!(shown.iter().any(|c| c.path == home().join(".config")), "with hidden files on");
    }

    #[test]
    fn an_unreadable_folder_drops_its_branch_not_the_answer() {
        let mock = tree();
        mock.make_unreadable(home().join("prints"));
        let found = run(&mock, "~/pr/hy");
        assert_eq!(found.first(), Some(&home().join("projects/hyprsuite")));
    }

    /// A backend that counts its reads, over the mock.
    struct Counting {
        inner: MockBackend,
        reads: AtomicUsize,
    }

    impl FsBackend for Counting {
        fn read_dir(&self, path: &Path) -> Result<Vec<Entry>, FilesError> {
            self.reads.fetch_add(1, AtomicOrdering::Relaxed);
            self.inner.read_dir(path)
        }
        fn stat(&self, path: &Path) -> Result<Entry, FilesError> {
            self.inner.stat(path)
        }
        fn count_children(&self, path: &Path) -> Result<usize, FilesError> {
            self.inner.count_children(path)
        }
        fn home_dir(&self) -> PathBuf {
            self.inner.home_dir()
        }
        fn canonicalize(&self, path: &Path) -> Result<PathBuf, FilesError> {
            self.inner.canonicalize(path)
        }
    }

    #[test]
    fn resolving_never_reads_more_than_its_budget() {
        // A wide tree: forty folders that all match `d`, each holding
        // forty more. Unbounded, `d/d/d` reads 1 + 40 + 1600 folders.
        let mock = MockBackend::new();
        let root = PathBuf::from("/w");
        let mut level1 = Vec::new();
        for i in 0..40 {
            let a = root.join(format!("d{i}"));
            level1.push(MockBackend::dir(&root, &format!("d{i}")));
            mock.seed(&a, (0..40).map(|j| MockBackend::dir(&a, &format!("d{j}"))).collect());
            for j in 0..40 {
                mock.seed(a.join(format!("d{j}")), vec![MockBackend::dir(&a.join(format!("d{j}")), "d")]);
            }
        }
        mock.seed(&root, level1);
        let counting = Counting { inner: mock, reads: AtomicUsize::new(0) };
        let limits = Limits { beam: 6, max_reads: 10, max_results: 8 };
        let query = parse("d/d/d", &root, None);
        let found = resolve(&counting, &query, &HashSet::new(), false, limits);
        assert!(counting.reads.load(AtomicOrdering::Relaxed) <= 10);
        assert!(!found.is_empty(), "a bounded answer is still an answer");
    }

    #[test]
    fn intermediate_segments_never_descend_into_an_archive() {
        let mock = MockBackend::new();
        let h = home();
        mock.set_home(&h);
        mock.seed(&h, vec![MockBackend::file(&h, "backup.zip", 1)]);
        // Seeded as though the routing backend would list it — which it
        // would, by decompressing it.
        mock.seed(h.join("backup.zip"), vec![MockBackend::dir(&h.join("backup.zip"), "src")]);
        assert!(run(&mock, "~/bak/src").is_empty());
        // Typed out, it is found: that is the literal path, not a match.
        assert_eq!(run(&mock, "~/backup.zip/src").first(), Some(&h.join("backup.zip/src")));
    }

    #[test]
    fn a_folder_wins_a_tie_with_a_file() {
        let mock = MockBackend::new();
        let u = PathBuf::from("/u");
        mock.seed(&u, vec![MockBackend::file(&u, "ic", 1), MockBackend::dir(&u, "icons")]);
        mock.seed(u.join("icons"), vec![]);
        mock.seed("/", vec![MockBackend::dir(Path::new("/"), "u")]);
        // `i` is a prefix of both names, so the score ties.
        let query = parse("/u/i", Path::new("/"), None);
        let found = resolve(&mock, &query, &HashSet::new(), false, Limits::default());
        assert_eq!(found[0].path, u.join("icons"), "even though the file's path is shorter");
    }

    #[test]
    fn the_last_segment_can_be_a_file() {
        let found = run(&tree(), "~/not");
        assert_eq!(found.first(), Some(&home().join("notes.txt")));
    }

    #[test]
    fn a_trailing_slash_lists_what_is_inside() {
        let found = run(&tree(), "~/projects/");
        assert_eq!(found[0], home().join("projects"), "the folder itself, as typed, first");
        assert!(found.contains(&home().join("projects/hyprsuite")));
        assert!(found.contains(&home().join("projects/website")));
    }

    #[test]
    fn dots_are_steps_not_patterns() {
        let q = parse("../x/./y", Path::new("/a/b"), None);
        assert_eq!(q, Query { base: PathBuf::from("/a"), segments: vec!["x".into(), "y".into()] });
        let q = parse("~/one/../two", Path::new("/"), Some(&home()));
        assert_eq!(q, Query { base: home(), segments: vec!["two".into()] });
    }

    #[test]
    fn a_relative_query_starts_where_you_are() {
        let found = run(&tree(), "proj/hy");
        assert_eq!(found.first(), Some(&home().join("projects/hyprsuite")));
    }

    #[test]
    fn completing_a_folder_leaves_the_cursor_one_level_down() {
        let c = Candidate { path: home().join("projects"), is_dir: true, exact: false };
        assert_eq!(completion(&c, Some(&home())), "~/projects/");
        let f = Candidate { path: home().join("notes.txt"), is_dir: false, exact: false };
        assert_eq!(completion(&f, Some(&home())), "~/notes.txt");
        let root = Candidate { path: PathBuf::from("/"), is_dir: true, exact: true };
        assert_eq!(completion(&root, Some(&home())), "/");
    }
}
