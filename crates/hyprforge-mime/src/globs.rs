//! What type a file is, decided by its **name**.
//!
//! The shared MIME database ships `globs2`, a plain list of
//! `weight:type:pattern` lines that every desktop already agrees on.
//! This reads it; it does not invent rules of its own.
//!
//! # Why by name, and not by content
//!
//! Content sniffing gets the interesting cases wrong in a way that is
//! invisible until someone double-clicks. `file --mime-type` — which is
//! what `xdg-open` falls back to on a desktop it does not recognise,
//! Hyprland included — reads bytes and never the name, so a `.stl` is
//! `application/octet-stream`, a `.3mf` is `application/zip` (it is one)
//! and a `.blend` is `application/zstd` (it is that too). Those are all
//! true statements about the bytes and useless statements about the
//! file: nothing has a default application for them, and a lookup that
//! finds nothing sends the file to a web browser.
//!
//! The name knows. `part.3mf` is a 3MF model that happens to be stored
//! as a zip, and that is exactly what `globs2` says.
//!
//! Content sniffing answers the cases a name cannot — a file with no
//! extension at all — and lives in [`crate::magic`]. Which of the two
//! wins, and when, is [`crate::lookup`]: a name beats a weak content
//! guess, and a strong one beats a name.

use std::path::Path;

/// One pattern from `globs2`.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Glob {
    /// The database's own weight. Higher wins.
    weight: u32,
    mime: String,
    pattern: String,
    /// The pattern's characters, and its length in them.
    ///
    /// Kept beside the pattern because matching walks characters, and
    /// collecting them per rule per lookup is the whole cost of a
    /// lookup: 1541 rules on this machine, ~40,000 on a full one, twice
    /// each. Parsing happens once at load; matching happens on every
    /// file anyone opens.
    chars: Vec<char>,
    /// `cs` in the flags field: this pattern only matches with the
    /// letters exactly as written.
    case_sensitive: bool,
}

impl Glob {
    fn new(weight: u32, mime: &str, pattern: &str, case_sensitive: bool) -> Glob {
        Glob {
            weight,
            mime: mime.to_string(),
            chars: pattern.chars().collect(),
            pattern: pattern.to_string(),
            case_sensitive,
        }
    }
}

/// The filename rules, loaded from every `globs2` on the system.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Globs {
    globs: Vec<Glob>,
}

impl Globs {
    /// Parses one `globs2` file's contents.
    ///
    /// Lines that cannot be read are skipped rather than failing the
    /// file: this is generated data, and one unfamiliar line in it must
    /// not cost every rule after it. `__NOGLOBS__` — the database's way
    /// of saying a type has had its inherited patterns removed — carries
    /// no pattern and is skipped with the rest.
    pub fn parse(text: &str) -> Globs {
        let mut globs = Vec::new();
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            // weight:type:pattern[:flags]
            let mut fields = line.splitn(4, ':');
            let (Some(weight), Some(mime), Some(pattern)) =
                (fields.next(), fields.next(), fields.next())
            else {
                continue;
            };
            let Ok(weight) = weight.parse::<u32>() else { continue };
            if pattern.is_empty() || pattern == "__NOGLOBS__" {
                continue;
            }
            let case_sensitive =
                fields.next().is_some_and(|flags| flags.split(':').any(|flag| flag == "cs"));
            globs.push(Glob::new(weight, mime, pattern, case_sensitive));
        }
        Globs { globs }
    }

    /// The older `globs` format: `type:pattern`, with no weight and no
    /// flags. Still shipped beside `globs2`, and the only one some
    /// hand-made databases have.
    ///
    /// Everything in it is weight 50, which is what the format means
    /// and what `update-mime-database` writes into `globs2` for a rule
    /// that did not ask for anything else.
    pub fn parse_legacy(text: &str) -> Globs {
        let mut globs = Vec::new();
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let Some((mime, pattern)) = line.split_once(':') else { continue };
            if pattern.is_empty() {
                continue;
            }
            globs.push(Glob::new(50, mime, pattern, false));
        }
        Globs { globs }
    }

    /// Every `globs2` under `dirs` (each a `XDG_DATA_DIRS` entry),
    /// merged. Earlier directories are more specific and are consulted
    /// first when two rules are otherwise equal.
    ///
    /// A directory with no `globs2` falls back to its `globs` — the
    /// older file, which is what a hand-made database is most likely to
    /// have. Never both from the same directory: they say the same
    /// thing, and reading both would double every rule in it.
    pub fn load_from(dirs: &[std::path::PathBuf]) -> Globs {
        let mut globs = Vec::new();
        for dir in dirs {
            let mime = dir.join("mime");
            if let Ok(text) = std::fs::read_to_string(mime.join("globs2")) {
                globs.extend(Globs::parse(&text).globs);
            } else if let Ok(text) = std::fs::read_to_string(mime.join("globs")) {
                globs.extend(Globs::parse_legacy(&text).globs);
            }
        }
        Globs { globs }
    }

    /// The type of a file with this name, or `None` when no rule
    /// matches.
    ///
    /// `None` rather than `application/octet-stream`: "no rule knows
    /// this name" and "this is a stream of bytes" are different answers,
    /// and a caller offering to open the file needs to tell them apart —
    /// the first deserves "nothing here knows what this is", the second
    /// would be a claim the database never made. This is the same
    /// distinction as a missing config file versus an unparseable one.
    ///
    /// When several rules match, the database's own precedence applies:
    /// the highest weight wins, then the longest pattern — so
    /// `archive.tar.gz` is a compressed tar rather than a gzip file,
    /// because `*.tar.gz` is longer than `*.gz`. A case-sensitive rule
    /// only matches the spelling it gives.
    ///
    /// **Case is tried exactly first, then folded.** `a.c` and `a.C`
    /// are different files: the database has `*.c` for C and `*.C` for
    /// C++, and a lookup that lowercases everything up front makes one
    /// of them unreachable. So the name is matched as written, and only
    /// if nothing matches is it tried again in lower case — which is
    /// what makes `script.PL` still a perl script.
    ///
    /// **Beyond that the specification says the result is undefined**,
    /// and implementations do differ. Two tie-breaks, in this order,
    /// each chosen against what the rest of this machine answers:
    ///
    /// 1. A registered type beats an `x-` one. `*.obj` is claimed by
    ///    `model/obj`, `application/x-coff` and `application/x-tgif` at
    ///    the same weight; `x-` means unregistered, and a Wavefront
    ///    model is the better reading of a `.obj` than a COFF object
    ///    file. `gio` answers `model/obj` here too.
    /// 2. Then the first rule wins, which is what the reference
    ///    implementation does (it skips a second rule for an extension
    ///    it has already seen). `*.json` is claimed by
    ///    `application/json` and `application/schema+json`; the first
    ///    is the answer everything else on the machine gives.
    pub fn type_of(&self, path: &Path) -> Option<&str> {
        let name = path.file_name()?.to_str()?;
        // Collected once for the whole sweep rather than once per rule
        // — see [`Glob::chars`].
        let chars: Vec<char> = name.chars().collect();
        self.best_match(&chars).or_else(|| {
            let lowered: Vec<char> = name.to_lowercase().chars().collect();
            (lowered != chars).then(|| self.best_match(&lowered)).flatten()
        })
    }

    /// The best rule for a name, taken exactly as given.
    fn best_match(&self, name: &[char]) -> Option<&str> {
        self.globs
            .iter()
            .filter(|glob| matches_at(&glob.chars, name))
            // Folded rather than `max_by_key`, which keeps the *last* of
            // several equal maxima — the opposite of what is wanted.
            .fold(None, |best: Option<&Glob>, glob| match best {
                Some(best) if rank(best) >= rank(glob) => Some(best),
                _ => Some(glob),
            })
            .map(|glob| glob.mime.as_str())
    }

    /// Every type whose pattern matches this name, best first.
    ///
    /// For `mimetype --all`, and for a caller that would rather see an
    /// ambiguity than have it resolved: `photo.jpg` matches one rule,
    /// but `archive.tar.gz` matches both `*.tar.gz` and `*.gz` and a
    /// person may want to know that.
    pub fn all_matches(&self, path: &Path) -> Vec<&str> {
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else { return Vec::new() };
        let exact: Vec<char> = name.chars().collect();
        let lowered: Vec<char> = name.to_lowercase().chars().collect();
        let mut matched: Vec<&Glob> = self
            .globs
            .iter()
            .filter(|glob| {
                matches_at(&glob.chars, &exact)
                    || (!glob.case_sensitive && matches_at(&glob.chars, &lowered))
            })
            .collect();
        matched.sort_by_key(|glob| std::cmp::Reverse((glob.weight, glob.pattern.len())));
        let mut out: Vec<&str> = Vec::new();
        for glob in matched {
            if !out.contains(&glob.mime.as_str()) {
                out.push(&glob.mime);
            }
        }
        out
    }

    /// Every type some rule can name a file after, in load order and
    /// with repeats — one type usually has several patterns.
    ///
    /// This is as close to "the types this machine knows about" as the
    /// database gets. It is deliberately the *glob* file and not the
    /// union of everything mentioned anywhere: a type no filename can
    /// produce is not one somebody will go looking for by name, and
    /// `subclasses` alone names hundreds of them.
    pub fn mimes(&self) -> impl Iterator<Item = &str> {
        self.globs.iter().map(|glob| glob.mime.as_str())
    }

    /// Whether anything at all was loaded. An empty database is not an
    /// error — a machine may genuinely have no `shared-mime-info` — but
    /// a caller that shows a chooser wants to say so rather than
    /// reporting that every file is of unknown type.
    pub fn is_empty(&self) -> bool {
        self.globs.is_empty()
    }

}

/// How good a rule is: the database's weight, then the length of the
/// pattern, then whether the type is a registered one rather than an
/// `x-` name. See [`Globs::type_of`] for where each comes from.
fn rank(glob: &Glob) -> (u32, usize, bool) {
    let registered = !glob.mime.split('/').nth(1).is_some_and(|sub| sub.starts_with("x-"));
    (glob.weight, glob.pattern.len(), registered)
}

/// `fnmatch` over the subset `globs2` actually uses: `*`, `?`, and
/// `[abc]` / `[0-9]` character classes.
///
/// Written out rather than pulled in as a dependency because the subset
/// is this small and the crate is a leaf on purpose. Recursion is on the
/// pattern, so a pathological pattern costs pattern length, not file
/// length. Both sides arrive as characters already — see [`Glob::chars`].
fn matches_at(pattern: &[char], name: &[char]) -> bool {
    match pattern.first() {
        None => name.is_empty(),
        Some('*') => {
            // Every split of the remaining name, shortest first.
            (0..=name.len()).any(|skip| matches_at(&pattern[1..], &name[skip..]))
        }
        Some('?') => !name.is_empty() && matches_at(&pattern[1..], &name[1..]),
        Some('[') => {
            let Some(close) = pattern.iter().position(|c| *c == ']') else {
                // An unclosed class is a literal bracket, which is what
                // a shell does with it too.
                return name.first() == Some(&'[') && matches_at(&pattern[1..], &name[1..]);
            };
            let Some(c) = name.first() else { return false };
            let (negated, set) = match pattern.get(1) {
                Some('!') => (true, &pattern[2..close]),
                _ => (false, &pattern[1..close]),
            };
            if in_class(set, *c) == negated {
                return false;
            }
            matches_at(&pattern[close + 1..], &name[1..])
        }
        Some(literal) => name.first() == Some(literal) && matches_at(&pattern[1..], &name[1..]),
    }
}

/// Whether `c` is in a character class body, ranges included.
fn in_class(set: &[char], c: char) -> bool {
    let mut i = 0;
    while i < set.len() {
        if set.get(i + 1) == Some(&'-') {
            if let Some(end) = set.get(i + 2) {
                if set[i] <= c && c <= *end {
                    return true;
                }
                i += 3;
                continue;
            }
        }
        if set[i] == c {
            return true;
        }
        i += 1;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn db() -> Globs {
        Globs::parse(
            "# generated, do not edit\n\
             50:model/stl:*.stl\n\
             50:model/3mf:*.3mf\n\
             50:application/zip:*.zip\n\
             50:image/png:*.png\n\
             40:image/apng:*.png\n\
             50:application/gzip:*.gz\n\
             50:application/x-compressed-tar:*.tar.gz\n\
             50:text/x-makefile:Makefile:cs\n\
             60:application/x-sharedlib:*.so.[0-9]*\n\
             0:application/x-modrinth-modpack+zip:__NOGLOBS__\n",
        )
    }

    /// The case this whole module exists for: a 3MF really is a zip, and
    /// the name is what says it is a model.
    #[test]
    fn a_model_stored_as_a_zip_is_still_a_model() {
        assert_eq!(db().type_of(&PathBuf::from("/d/part.3mf")), Some("model/3mf"));
        assert_eq!(db().type_of(&PathBuf::from("/d/thing.stl")), Some("model/stl"));
        assert_eq!(db().type_of(&PathBuf::from("/d/archive.zip")), Some("application/zip"));
    }

    /// Two rules, same weight, same pattern. The first one in the
    /// database wins — `*.json` really is claimed twice, and picking
    /// the second one made every `.json` on the machine a JSON
    /// *schema*.
    #[test]
    fn the_first_of_two_equal_rules_wins() {
        let globs = Globs::parse("50:application/json:*.json\n50:application/schema+json:*.json\n");
        assert_eq!(globs.type_of(&PathBuf::from("package.json")), Some("application/json"));

        // And the other way round, to prove it is the order and not the
        // name that decides.
        let reversed = Globs::parse("50:application/schema+json:*.json\n50:application/json:*.json\n");
        assert_eq!(reversed.type_of(&PathBuf::from("package.json")), Some("application/schema+json"));
    }

    #[test]
    fn the_heavier_rule_wins_and_then_the_longer_one() {
        assert_eq!(db().type_of(&PathBuf::from("a.png")), Some("image/png"), "50 beats 40");
        assert_eq!(
            db().type_of(&PathBuf::from("backup.tar.gz")),
            Some("application/x-compressed-tar"),
            "*.tar.gz is longer than *.gz"
        );
    }

    /// A name nothing knows is `None`, never octet-stream — see
    /// [`Globs::type_of`].
    #[test]
    fn an_unknown_name_is_unknown_rather_than_a_stream_of_bytes() {
        assert_eq!(db().type_of(&PathBuf::from("/d/mystery.qqq")), None);
        assert_eq!(db().type_of(&PathBuf::from("/d/no-extension")), None);
    }

    /// Both rules are reported, best first, rather than one being
    /// chosen for the caller.
    #[test]
    fn every_matching_rule_can_be_listed() {
        assert_eq!(
            db().all_matches(&PathBuf::from("backup.tar.gz")),
            ["application/x-compressed-tar", "application/gzip"]
        );
        assert_eq!(db().all_matches(&PathBuf::from("a.png")), ["image/png", "image/apng"]);
        assert!(db().all_matches(&PathBuf::from("mystery.qqq")).is_empty());
    }

    #[test]
    fn matching_ignores_case_unless_the_rule_asks_otherwise() {
        assert_eq!(db().type_of(&PathBuf::from("SHOUTING.STL")), Some("model/stl"));
        assert_eq!(db().type_of(&PathBuf::from("Makefile")), Some("text/x-makefile"));
        assert_eq!(db().type_of(&PathBuf::from("makefile")), None, "cs means exactly that");
    }

    /// `a.c` and `a.C` are different files, and the database says so:
    /// one is C, the other C++. Lowercasing before matching makes the
    /// second unreachable.
    #[test]
    fn an_exact_case_match_is_preferred_to_a_folded_one() {
        let globs = Globs::parse("50:text/x-c++src:*.C\n50:text/x-csrc:*.c\n");
        assert_eq!(globs.type_of(&PathBuf::from("main.c")), Some("text/x-csrc"));
        assert_eq!(globs.type_of(&PathBuf::from("main.C")), Some("text/x-c++src"));
        // Nothing matches `README.TXT` exactly, so it is folded and
        // found — the rule that keeps `script.PL` a perl script.
        let upper = Globs::parse("50:text/plain:*.txt\n");
        assert_eq!(upper.type_of(&PathBuf::from("README.TXT")), Some("text/plain"));
    }

    /// Where the specification gives up — same weight, same pattern —
    /// a registered type beats an `x-` one.
    #[test]
    fn a_registered_type_beats_an_unregistered_one_on_a_tie() {
        let globs = Globs::parse(
            "50:application/x-coff:*.obj\n50:application/x-tgif:*.obj\n50:model/obj:*.obj\n",
        );
        assert_eq!(globs.type_of(&PathBuf::from("bracket.obj")), Some("model/obj"));
    }

    #[test]
    fn a_character_class_matches_a_range() {
        assert_eq!(db().type_of(&PathBuf::from("libc.so.6")), Some("application/x-sharedlib"));
        assert_eq!(db().type_of(&PathBuf::from("libc.so.x")), None);
    }

    /// Generated data with something unfamiliar in it must cost that
    /// line only — the same rule the config parsers follow.
    #[test]
    fn an_unreadable_line_costs_that_line_alone() {
        let globs = Globs::parse("nonsense\n50:text/plain:*.txt\nalso:nonsense\n\n");
        assert_eq!(globs.type_of(&PathBuf::from("a.txt")), Some("text/plain"));
        assert!(!globs.is_empty());
    }

    #[test]
    fn the_older_globs_format_is_read_when_that_is_all_there_is() {
        let globs = Globs::parse_legacy(
            "# a test file\napplication/x-perl:*.pl\napplication/x-compressed-tar:*.tar.gz\n\
             application/x-gzip:*.gz\ntext/x-makefile:[Mm]akefile\n",
        );
        assert_eq!(globs.type_of(&PathBuf::from("script.pl")), Some("application/x-perl"));
        assert_eq!(
            globs.type_of(&PathBuf::from("script.tar.gz")),
            Some("application/x-compressed-tar"),
            "the longer pattern still wins at equal weight"
        );
        assert_eq!(globs.type_of(&PathBuf::from("Makefile")), Some("text/x-makefile"));
    }

    #[test]
    fn a_machine_with_no_database_is_empty_rather_than_wrong() {
        let globs = Globs::load_from(&[PathBuf::from("/nonexistent-xyz")]);
        assert!(globs.is_empty());
        assert_eq!(globs.type_of(&PathBuf::from("a.png")), None);
    }
}
