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
//! Content sniffing still has the cases the name cannot answer — a file
//! with no extension at all — and is deliberately not implemented here
//! rather than half-implemented: see [`type_of`]'s own doc for what
//! happens instead.

use std::collections::BTreeMap;
use std::path::Path;

/// One pattern from `globs2`.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Glob {
    /// The database's own weight. Higher wins.
    weight: u32,
    mime: String,
    pattern: String,
    /// `cs` in the flags field: this pattern only matches with the
    /// letters exactly as written.
    case_sensitive: bool,
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
            globs.push(Glob {
                weight,
                mime: mime.to_string(),
                pattern: pattern.to_string(),
                case_sensitive: fields.next().is_some_and(|flags| {
                    flags.split(':').any(|flag| flag == "cs")
                }),
            });
        }
        Globs { globs }
    }

    /// Every `globs2` under `dirs` (each a `XDG_DATA_DIRS` entry),
    /// merged. Earlier directories are more specific and are consulted
    /// first when two rules are otherwise equal.
    pub fn load_from(dirs: &[std::path::PathBuf]) -> Globs {
        let mut globs = Vec::new();
        for dir in dirs {
            let path = dir.join("mime").join("globs2");
            if let Ok(text) = std::fs::read_to_string(&path) {
                globs.extend(Globs::parse(&text).globs);
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
    pub fn type_of(&self, path: &Path) -> Option<&str> {
        let name = path.file_name()?.to_str()?;
        let lowered = name.to_lowercase();
        self.globs
            .iter()
            .filter(|glob| {
                if glob.case_sensitive {
                    matches(&glob.pattern, name)
                } else {
                    matches(&glob.pattern.to_lowercase(), &lowered)
                }
            })
            // Weight first, then the longer pattern, which is what makes
            // `*.tar.gz` beat `*.gz` at the same weight.
            .max_by_key(|glob| (glob.weight, glob.pattern.len()))
            .map(|glob| glob.mime.as_str())
    }

    /// Whether anything at all was loaded. An empty database is not an
    /// error — a machine may genuinely have no `shared-mime-info` — but
    /// a caller that shows a chooser wants to say so rather than
    /// reporting that every file is of unknown type.
    pub fn is_empty(&self) -> bool {
        self.globs.is_empty()
    }

    /// Every type the database knows a name rule for, with one example
    /// pattern each — for a "what can this machine open" listing.
    pub fn types(&self) -> BTreeMap<&str, &str> {
        let mut types = BTreeMap::new();
        for glob in &self.globs {
            types.entry(glob.mime.as_str()).or_insert(glob.pattern.as_str());
        }
        types
    }
}

/// `fnmatch` over the subset `globs2` actually uses: `*`, `?`, and
/// `[abc]` / `[0-9]` character classes.
///
/// Written out rather than pulled in as a dependency because the subset
/// is this small and the crate is a leaf on purpose. Recursion is on the
/// pattern, so a pathological pattern costs pattern length, not file
/// length.
fn matches(pattern: &str, name: &str) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    let n: Vec<char> = name.chars().collect();
    matches_at(&p, &n)
}

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

    #[test]
    fn matching_ignores_case_unless_the_rule_asks_otherwise() {
        assert_eq!(db().type_of(&PathBuf::from("SHOUTING.STL")), Some("model/stl"));
        assert_eq!(db().type_of(&PathBuf::from("Makefile")), Some("text/x-makefile"));
        assert_eq!(db().type_of(&PathBuf::from("makefile")), None, "cs means exactly that");
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
    fn a_type_with_its_globs_removed_carries_no_pattern() {
        assert!(db().types().keys().all(|t| *t != "application/x-modrinth-modpack+zip"));
    }

    #[test]
    fn a_machine_with_no_database_is_empty_rather_than_wrong() {
        let globs = Globs::load_from(&[PathBuf::from("/nonexistent-xyz")]);
        assert!(globs.is_empty());
        assert_eq!(globs.type_of(&PathBuf::from("a.png")), None);
    }
}
