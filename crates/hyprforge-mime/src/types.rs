//! What the database knows about a *type*, as opposed to about a file:
//! its other names, what it is a kind of, and what to call it in front
//! of a person.
//!
//! Three more plain files next to `globs2`:
//!
//! - `aliases` — `alias canonical`, one per line. `application/acrobat`
//!   is `application/pdf` under an older name, and a lookup that does
//!   not resolve it will miss a perfectly good association.
//! - `subclasses` — `child parent`. A `.3mf` is a zip and a
//!   `application/yaml` is text; an application registered for the
//!   parent can open the child.
//! - `<media>/<subtype>.xml` — the type's `<comment>`, translated. "STL
//!   3D model" rather than `model/stl`.
//!
//! # The two subclass rules that are not in the file
//!
//! The specification states them in prose and every implementation has
//! to add them by hand: every `text/*` is a subclass of `text/plain`,
//! and everything whatsoever is a subclass of
//! `application/octet-stream`. Leave them out and "can this application
//! open this file" answers no for every text editor and every file.

use std::collections::BTreeMap;
use std::path::Path;

/// The type graph: aliases, subclasses and descriptions.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Types {
    /// Alias to the name the rest of the database uses.
    aliases: BTreeMap<String, String>,
    /// Child to its direct parents.
    parents: BTreeMap<String, Vec<String>>,
    /// Type to what a person should be shown.
    descriptions: BTreeMap<String, String>,
}

impl Types {
    /// Reads `aliases` and `subclasses` from every directory given.
    ///
    /// Descriptions are *not* read here: they are one XML file per type,
    /// some thousands of them, and nothing needs them until something
    /// asks for one. [`Types::describe_from`] reads the one file.
    pub fn load_from(dirs: &[std::path::PathBuf]) -> Types {
        let mut types = Types::default();
        for dir in dirs {
            let mime = dir.join("mime");
            if let Ok(text) = std::fs::read_to_string(mime.join("aliases")) {
                types.add_aliases(&text);
            }
            if let Ok(text) = std::fs::read_to_string(mime.join("subclasses")) {
                types.add_subclasses(&text);
            }
        }
        types
    }

    /// `alias canonical`, one per line. The first file to name an alias
    /// keeps it, so a user's own database entry beats the system's.
    pub fn add_aliases(&mut self, text: &str) {
        for (from, to) in pairs(text) {
            self.aliases.entry(from).or_insert(to);
        }
    }

    /// `child parent`, one per line. A type can have several parents,
    /// and each is kept.
    pub fn add_subclasses(&mut self, text: &str) {
        for (child, parent) in pairs(text) {
            let parents = self.parents.entry(child).or_default();
            if !parents.contains(&parent) {
                parents.push(parent);
            }
        }
    }

    /// The name the rest of the database uses for this type.
    ///
    /// Aliases do not chain in practice, but a database that claimed
    /// they did would otherwise hang here, so this follows at most a
    /// few links and then stops.
    pub fn canonical<'a>(&'a self, mime: &'a str) -> &'a str {
        let mut name = mime;
        for _ in 0..4 {
            match self.aliases.get(name) {
                Some(next) if next != name => name = next,
                _ => break,
            }
        }
        name
    }

    /// Whether `mime` is `parent`, or a kind of it.
    ///
    /// Includes the two rules the file does not carry — see the module
    /// doc — so `text/x-shellscript` is a `text/plain` and everything is
    /// an `application/octet-stream`.
    pub fn is_subclass_of(&self, mime: &str, parent: &str) -> bool {
        let mime = self.canonical(mime);
        let parent = self.canonical(parent);
        if mime == parent || parent == "application/octet-stream" {
            return true;
        }
        if parent == "text/plain" && mime.starts_with("text/") {
            return true;
        }
        // Depth-first, with a seen set: the database is generated, but
        // a hand-written package in a user's own directory can make a
        // cycle, and a cycle must not hang a file manager.
        let mut seen: Vec<&str> = vec![mime];
        let mut stack: Vec<&str> = self.parents.get(mime).into_iter().flatten().map(String::as_str).collect();
        while let Some(next) = stack.pop() {
            let next = self.canonical(next);
            if next == parent {
                return true;
            }
            if seen.contains(&next) {
                continue;
            }
            seen.push(next);
            stack.extend(self.parents.get(next).into_iter().flatten().map(String::as_str));
        }
        false
    }

    /// Every type this one is a kind of, nearest first — for finding an
    /// application when nothing is registered for the exact type. A 3MF
    /// with no 3MF viewer can still be opened by an archive manager.
    pub fn ancestors(&self, mime: &str) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        let mut stack: Vec<String> = vec![self.canonical(mime).to_string()];
        while let Some(next) = stack.pop() {
            for parent in self.parents.get(&next).into_iter().flatten() {
                let parent = self.canonical(parent).to_string();
                if !out.contains(&parent) {
                    out.push(parent.clone());
                    stack.push(parent);
                }
            }
        }
        if mime.starts_with("text/") && !out.iter().any(|p| p == "text/plain") && mime != "text/plain" {
            out.push("text/plain".to_string());
        }
        if mime != "application/octet-stream" {
            out.push("application/octet-stream".to_string());
        }
        out
    }

    /// Reads one type's description out of `<media>/<subtype>.xml`,
    /// remembering it.
    ///
    /// `language` is an `xml:lang` to prefer — `"de"` finds
    /// `<comment xml:lang="de">`; the untagged comment is the fallback,
    /// which is what a request for English gets, since the database
    /// leaves English untagged.
    pub fn describe_from(&mut self, dirs: &[std::path::PathBuf], mime: &str, language: Option<&str>) -> Option<&str> {
        let canonical = self.canonical(mime).to_string();
        if !self.descriptions.contains_key(&canonical) {
            if let Some(comment) = description_of(dirs, &canonical, language) {
                self.descriptions.insert(canonical.clone(), comment);
            }
        }
        self.descriptions.get(&canonical).map(String::as_str)
    }

    /// Whether anything was loaded.
    pub fn is_empty(&self) -> bool {
        self.aliases.is_empty() && self.parents.is_empty()
    }
}

/// One type's description, read and not remembered.
///
/// The uncached half of [`Types::describe_from`], public because the
/// caller that needs a screenful of descriptions at once has nowhere to
/// put a cache: it holds the database behind an `Arc` and cannot borrow
/// it mutably. Reading a handful of small files off the UI thread is
/// the cheaper of the two problems.
///
/// `mime` must already be canonical — this reads a file named after it
/// and resolves no aliases, which is the difference between this and
/// the method.
pub fn description_of(
    dirs: &[std::path::PathBuf],
    mime: &str,
    language: Option<&str>,
) -> Option<String> {
    let (media, subtype) = mime.split_once('/')?;
    for dir in dirs {
        let path = dir.join("mime").join(media).join(format!("{subtype}.xml"));
        if let Ok(text) = std::fs::read_to_string(&path) {
            if let Some(comment) = comment_in(&text, language) {
                return Some(comment);
            }
        }
    }
    None
}

/// `a b` per line, ignoring blanks and comments.
fn pairs(text: &str) -> impl Iterator<Item = (String, String)> + '_ {
    text.lines().filter_map(|line| {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            return None;
        }
        let (a, b) = line.split_once(char::is_whitespace)?;
        Some((a.trim().to_string(), b.trim().to_string()))
    })
}

/// The `<comment>` for a language, or the untagged one.
///
/// Hand-written rather than an XML parser: these files are generated by
/// `update-mime-database` in a fixed shape, and the one element wanted
/// is on its own line. A dependency on a full parser to read one tag out
/// of a generated file would be the larger risk — but this is why the
/// function is conservative, taking only what is between the tags it
/// recognises and nothing else.
fn comment_in(xml: &str, language: Option<&str>) -> Option<String> {
    let mut untagged = None;
    for line in xml.lines() {
        let line = line.trim();
        let Some(rest) = line.strip_prefix("<comment") else { continue };
        let Some(end) = rest.find("</comment>") else { continue };
        let Some(open) = rest.find('>') else { continue };
        if open > end {
            continue;
        }
        let text = unescape(&rest[open + 1..end]);
        match (rest[..open].split_once("xml:lang=\""), language) {
            // `<comment xml:lang="de">`, and German was asked for.
            (Some((_, tail)), Some(wanted)) if tail.starts_with(&format!("{wanted}\"")) => {
                return Some(text);
            }
            (Some(_), _) => continue,
            // The untagged one. Kept rather than returned, so a
            // requested language later in the file still wins.
            (None, _) => untagged = untagged.or(Some(text)),
        }
    }
    untagged
}

/// The five XML entities `update-mime-database` writes.
fn unescape(text: &str) -> String {
    text.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

/// Every `mime` directory to read, for a caller that has its own list
/// of XDG data directories.
pub fn mime_dirs(data_dirs: &[std::path::PathBuf]) -> Vec<std::path::PathBuf> {
    data_dirs.iter().map(|dir| dir.join("mime")).collect()
}

/// Whether `path` is something other than an ordinary file, and what
/// the database calls that.
///
/// The specification's own first step, and the one every content
/// sniffer forgets: a socket is not "an empty file", and a directory is
/// `inode/directory` rather than whatever its name suggests — a folder
/// called `notes.txt` is still a folder.
///
/// `follow` is whether a symlink is reported as what it points at.
pub fn inode_type(path: &Path, follow: bool) -> Option<&'static str> {
    use std::os::unix::fs::FileTypeExt;
    let meta = match follow {
        true => std::fs::metadata(path).ok()?,
        false => std::fs::symlink_metadata(path).ok()?,
    };
    let kind = meta.file_type();
    if kind.is_file() {
        return None;
    }
    Some(if kind.is_dir() {
        // A directory on a different device from its parent is where
        // something is mounted, and the database has a name for that:
        // `x-content/*` handlers and "eject" actions hang off it.
        match mounted_here(path, &meta) {
            true => "inode/mount-point",
            false => "inode/directory",
        }
    } else if kind.is_symlink() {
        // Only reachable with `follow` false, or a broken link.
        "inode/symlink"
    } else if kind.is_fifo() {
        "inode/fifo"
    } else if kind.is_socket() {
        "inode/socket"
    } else if kind.is_block_device() {
        "inode/blockdevice"
    } else if kind.is_char_device() {
        "inode/chardevice"
    } else {
        "inode/unknown"
    })
}

/// Whether this directory is where a filesystem is mounted — its
/// device differs from its parent's.
///
/// The root directory is its own parent, so it compares equal and is
/// reported as an ordinary directory rather than a mount point. That is
/// what `File::MimeInfo` does too, and arguing with it would mean every
/// implementation on the machine disagreeing about `/`.
fn mounted_here(path: &Path, meta: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;
    let Ok(absolute) = std::fs::canonicalize(path) else { return false };
    let Some(parent) = absolute.parent() else { return false };
    match std::fs::metadata(parent) {
        Ok(parent) => parent.dev() != meta.dev(),
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn graph() -> Types {
        let mut types = Types::default();
        types.add_aliases("application/acrobat application/pdf\napplication/x-pdf application/pdf\n");
        types.add_subclasses(
            "model/3mf application/zip\n\
             application/zip application/octet-stream\n\
             text/x-shellscript text/plain\n\
             application/yaml text/plain\n",
        );
        types
    }

    #[test]
    fn an_alias_resolves_to_the_name_the_database_uses() {
        assert_eq!(graph().canonical("application/acrobat"), "application/pdf");
        assert_eq!(graph().canonical("application/pdf"), "application/pdf");
        assert_eq!(graph().canonical("model/stl"), "model/stl", "not an alias, unchanged");
    }

    #[test]
    fn a_child_is_a_kind_of_its_parent_however_far_up() {
        let types = graph();
        assert!(types.is_subclass_of("model/3mf", "application/zip"));
        assert!(types.is_subclass_of("model/3mf", "application/octet-stream"));
        assert!(!types.is_subclass_of("application/zip", "model/3mf"), "not the other way round");
    }

    /// The two rules the file does not carry — see the module doc.
    #[test]
    fn every_text_is_plain_text_and_everything_is_bytes() {
        let types = graph();
        assert!(types.is_subclass_of("text/markdown", "text/plain"), "not in the file at all");
        assert!(types.is_subclass_of("image/png", "application/octet-stream"));
        assert!(types.is_subclass_of("model/stl", "model/stl"));
        assert!(!types.is_subclass_of("image/png", "text/plain"));
    }

    #[test]
    fn subclassing_sees_through_an_alias() {
        let mut types = graph();
        types.add_subclasses("application/pdf application/octet-stream\n");
        assert!(types.is_subclass_of("application/acrobat", "application/octet-stream"));
    }

    /// A hand-written package in a user's own directory can make a
    /// cycle, and a cycle must not hang the window asking the question.
    #[test]
    fn a_cycle_in_the_database_does_not_hang() {
        let mut types = Types::default();
        types.add_subclasses("a/one a/two\na/two a/one\n");
        assert!(!types.is_subclass_of("a/one", "image/png"));
        assert!(types.is_subclass_of("a/one", "a/two"));
    }

    #[test]
    fn the_ancestors_are_listed_for_finding_an_application() {
        let types = graph();
        let ancestors = types.ancestors("model/3mf");
        assert_eq!(ancestors.first().map(String::as_str), Some("application/zip"));
        assert!(ancestors.contains(&"application/octet-stream".to_string()));

        let text = types.ancestors("text/markdown");
        assert!(text.contains(&"text/plain".to_string()));
    }

    #[test]
    fn a_description_is_read_from_the_types_own_file() {
        assert_eq!(
            comment_in(
                "<mime-type type=\"model/stl\">\n  <comment>STL 3D model</comment>\n  \
                 <comment xml:lang=\"de\">STL-3D-Modell</comment>\n</mime-type>\n",
                None
            )
            .as_deref(),
            Some("STL 3D model")
        );
    }

    #[test]
    fn a_translation_is_preferred_when_one_is_asked_for() {
        let xml = "<comment>STL 3D model</comment>\n<comment xml:lang=\"de\">STL-3D-Modell</comment>\n";
        assert_eq!(comment_in(xml, Some("de")).as_deref(), Some("STL-3D-Modell"));
        assert_eq!(comment_in(xml, Some("fr")).as_deref(), Some("STL 3D model"), "falls back");
    }

    #[test]
    fn an_escaped_description_reads_as_the_characters_it_stands_for() {
        assert_eq!(
            comment_in("<comment>Bob &amp; Sons&apos; &lt;tag&gt;</comment>", None).as_deref(),
            Some("Bob & Sons' <tag>")
        );
    }

    #[test]
    fn a_directory_is_a_directory_whatever_it_is_called() {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("notes.txt");
        std::fs::create_dir(&folder).unwrap();
        assert_eq!(inode_type(&folder, true), Some("inode/directory"));

        let file = dir.path().join("real.txt");
        std::fs::write(&file, b"x").unwrap();
        assert_eq!(inode_type(&file, true), None, "an ordinary file is for the other rules");
        assert_eq!(inode_type(&dir.path().join("nothing-here"), true), None);
    }

    /// `/` is its own parent, so the comparison says "same device" and
    /// it reads as an ordinary directory — which is what every other
    /// implementation on the machine says about it.
    #[test]
    fn the_root_directory_is_not_reported_as_a_mount_point() {
        assert_eq!(inode_type(Path::new("/"), true), Some("inode/directory"));
    }

    #[test]
    fn a_symlink_is_followed_or_reported_as_one() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("target.txt");
        std::fs::write(&target, b"x").unwrap();
        let link = dir.path().join("link.txt");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        assert_eq!(inode_type(&link, true), None, "followed: an ordinary file");
        assert_eq!(inode_type(&link, false), Some("inode/symlink"));
    }
}
