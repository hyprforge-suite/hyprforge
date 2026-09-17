//! Copied and cut files, and what pasting them means.
//!
//! Two halves, kept apart.
//!
//! **Where the copied files are kept** is [`FileClipboard`], a trait —
//! the same "seam before the client" shape as `backend::FsBackend`.
//! [`MemoryClipboard`] keeps them in this process; the app's own
//! `SystemClipboard` puts them on the Wayland clipboard, so a copy in
//! Files pastes into another file manager and back. That one lives in
//! the app crate, because this one — shared with the open/save dialog —
//! stays free of Wayland. The formats both sides of that exchange use are
//! here, and pure: [`clip_offers`] and [`clip_from`].
//!
//! **What a paste will do** is [`plan`], a pure function from what was
//! copied and where it is going to a list of copy/move steps plus
//! anything refused — tested without touching a disk. The host carries
//! the steps out through `hyprforge_fileops::ops`.

use hyprforge_fileops::OpKind;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// Whether pasting should copy the files or move them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClipVerb {
    Copy,
    Cut,
}

/// Files waiting to be pasted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileClip {
    pub paths: Vec<PathBuf>,
    pub verb: ClipVerb,
}

/// Where copied files are kept until they are pasted.
pub trait FileClipboard: Send + Sync {
    /// Replaces whatever was there. `Err` is a sentence for the status
    /// bar — a clipboard that could not be written must say so, or the
    /// next paste pastes something older without anyone knowing.
    fn set(&self, clip: FileClip) -> Result<(), String>;

    /// The files waiting to be pasted, if the clipboard holds files.
    fn get(&self) -> Option<FileClip>;

    /// Empties it — after a cut has been pasted, since the files are no
    /// longer where the clipboard says they are.
    fn clear(&self);

    /// Whether Paste should be offered, answered without waiting on
    /// anything.
    ///
    /// Separate from [`Self::get`] because `get` may have to ask another
    /// process, and this is asked every time a menu opens. A clipboard
    /// that cannot know cheaply answers `true`, and the paste itself
    /// finds out — "nothing to paste" after pressing Paste is a lesser
    /// failure than a Paste that stays greyed out while files are
    /// sitting on the clipboard.
    fn may_hold_files(&self) -> bool {
        self.get().is_some()
    }
}

/// A clipboard that lives in this process.
///
/// Every tab shares one, so a copy in one tab pastes in another. It does
/// not reach other applications; see the module doc for what will.
#[derive(Debug, Default)]
pub struct MemoryClipboard {
    held: Mutex<Option<FileClip>>,
}

impl MemoryClipboard {
    pub fn new() -> Self {
        Self::default()
    }
}

impl FileClipboard for MemoryClipboard {
    fn set(&self, clip: FileClip) -> Result<(), String> {
        *self.held.lock().unwrap_or_else(|e| e.into_inner()) = Some(clip);
        Ok(())
    }

    fn get(&self) -> Option<FileClip> {
        self.held.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    fn clear(&self) {
        *self.held.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }
}

// ---------------------------------------------------------------------
// The formats other file managers read and write
// ---------------------------------------------------------------------

/// GNOME's (and Nautilus's, Thunar's, Nemo's) copied-files type: a verb
/// line, then one URI per line.
pub const GNOME_COPIED_FILES: &str = "x-special/gnome-copied-files";

/// The standard list of URIs (RFC 2483). Every file manager reads it;
/// on its own it cannot say "cut", so it always means copy.
pub const URI_LIST: &str = "text/uri-list";

/// KDE's (Dolphin's) way of saying a `text/uri-list` was cut: this type
/// offered alongside it, with the content `1`.
pub const KDE_CUT_SELECTION: &str = "application/x-kde-cutselection";

/// What a copy of files offers, most specific first, as
/// `(type, content)`: GNOME's type, the URI list, KDE's cut marker when
/// it is a cut, and the paths as plain text — so pasting into a
/// terminal or an editor gives paths rather than nothing.
pub fn clip_offers(clip: &FileClip) -> Vec<(&'static str, String)> {
    let uris: Vec<String> = clip.paths.iter().map(|p| file_uri(p)).collect();
    let verb = match clip.verb {
        ClipVerb::Copy => "copy",
        ClipVerb::Cut => "cut",
    };
    let mut offers = vec![
        // No trailing newline: Nautilus writes none, and some readers
        // take a trailing empty line as an empty URI.
        (GNOME_COPIED_FILES, format!("{verb}\n{}", uris.join("\n"))),
        // RFC 2483: CRLF after every line, the last included.
        (URI_LIST, uris.iter().map(|u| format!("{u}\r\n")).collect()),
    ];
    if clip.verb == ClipVerb::Cut {
        offers.push((KDE_CUT_SELECTION, "1".to_string()));
    }
    let text: Vec<String> = clip.paths.iter().map(|p| p.to_string_lossy().into_owned()).collect();
    offers.push(("text/plain;charset=utf-8", text.join("\n")));
    offers.push(("text/plain", text.join("\n")));
    offers
}

/// The types worth asking for when pasting, best first.
pub const PASTE_TYPES: [&str; 2] = [GNOME_COPIED_FILES, URI_LIST];

/// Reads what another application put on the clipboard, given the type
/// it came as. `None` when it holds no local files.
pub fn clip_from(mime: &str, content: &[u8]) -> Option<FileClip> {
    let text = String::from_utf8_lossy(content);
    match mime {
        GNOME_COPIED_FILES => {
            let mut lines = text.lines();
            let verb = match lines.next()?.trim() {
                "cut" => ClipVerb::Cut,
                "copy" => ClipVerb::Copy,
                _ => return None,
            };
            clip_of(verb, lines)
        }
        URI_LIST => clip_of(ClipVerb::Copy, text.lines()),
        _ => None,
    }
}

fn clip_of<'a>(verb: ClipVerb, lines: impl Iterator<Item = &'a str>) -> Option<FileClip> {
    let paths: Vec<PathBuf> = lines
        .map(str::trim)
        // RFC 2483 comment lines.
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .filter_map(path_of_uri)
        .collect();
    (!paths.is_empty()).then_some(FileClip { paths, verb })
}

/// `file:///home/a%20b` as `/home/a b`.
///
/// Only local files: a `file://` URI with no host or with `localhost`.
/// Anything else — `sftp://`, `smb://`, `file://otherhost/…` — is a place
/// this app cannot copy from, and is left out rather than misread as a
/// local path.
pub fn path_of_uri(uri: &str) -> Option<PathBuf> {
    let rest = uri.strip_prefix("file://")?;
    let rest = rest.strip_prefix("localhost").unwrap_or(rest);
    if !rest.starts_with('/') {
        return None;
    }
    hyprforge_fileops::percent::decode_path(rest).ok()
}

/// `/home/a b` as `file:///home/a%20b`.
pub fn file_uri(path: &Path) -> String {
    format!("file://{}", hyprforge_fileops::percent::encode_path(path))
}

/// One file or folder to put somewhere.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PasteStep {
    pub source: PathBuf,
    /// The full path it lands at — `hyprforge_fileops::ops::Operation`
    /// takes a destination path, not a destination folder.
    pub dest: PathBuf,
    pub kind: OpKind,
    /// A copy into the folder it came from. Its destination is itself,
    /// so it always collides, and the only sensible answer is a second
    /// copy beside the first — asking "replace this file with itself?"
    /// would be absurd. The host answers that collision with Keep Both
    /// without asking.
    pub duplicate: bool,
}

/// What a paste will do.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PastePlan {
    pub steps: Vec<PasteStep>,
    /// Sources that will not be touched, each with the sentence saying
    /// why.
    pub refused: Vec<(PathBuf, String)>,
}

/// Plans pasting `clip` into the folder `into`.
///
/// - A cut pasted back into the folder it came from does nothing: the
///   files are already there. Not reported — nothing went wrong.
/// - A copy pasted back into its own folder makes a duplicate beside the
///   original. See [`PasteStep::duplicate`].
/// - A folder pasted into itself, or into anything inside itself, is
///   refused. Copying a tree into itself never finishes; moving one into
///   itself is not possible at all.
/// - A path with no name — `/` — is refused.
pub fn plan(clip: &FileClip, into: &Path) -> PastePlan {
    let mut result = PastePlan::default();
    for source in &clip.paths {
        let Some(name) = source.file_name() else {
            result.refused.push((source.clone(), format!("{} has no name to paste under.", source.display())));
            continue;
        };
        if into == source || into.starts_with(source) {
            result.refused.push((
                source.clone(),
                format!("{} can't be pasted inside itself.", name.to_string_lossy()),
            ));
            continue;
        }
        let same_folder = source.parent() == Some(into);
        let kind = match clip.verb {
            ClipVerb::Copy => OpKind::Copy,
            ClipVerb::Cut => OpKind::Move,
        };
        if same_folder && kind == OpKind::Move {
            continue;
        }
        result.steps.push(PasteStep {
            source: source.clone(),
            dest: into.join(name),
            kind,
            duplicate: same_folder,
        });
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clip(verb: ClipVerb, paths: &[&str]) -> FileClip {
        FileClip { paths: paths.iter().map(PathBuf::from).collect(), verb }
    }

    #[test]
    fn a_copy_lands_in_the_target_under_the_same_name() {
        let p = plan(&clip(ClipVerb::Copy, &["/a/notes.txt"]), Path::new("/b"));
        assert_eq!(
            p.steps,
            [PasteStep {
                source: "/a/notes.txt".into(),
                dest: "/b/notes.txt".into(),
                kind: OpKind::Copy,
                duplicate: false,
            }]
        );
        assert!(p.refused.is_empty());
    }

    #[test]
    fn a_cut_becomes_a_move() {
        let p = plan(&clip(ClipVerb::Cut, &["/a/notes.txt"]), Path::new("/b"));
        assert_eq!(p.steps[0].kind, OpKind::Move);
    }

    #[test]
    fn a_copy_into_its_own_folder_is_a_duplicate() {
        let p = plan(&clip(ClipVerb::Copy, &["/a/notes.txt"]), Path::new("/a"));
        assert!(p.steps[0].duplicate);
    }

    /// Nothing to do, and nothing wrong — so nothing is reported.
    #[test]
    fn a_cut_into_its_own_folder_does_nothing_quietly() {
        let p = plan(&clip(ClipVerb::Cut, &["/a/notes.txt"]), Path::new("/a"));
        assert_eq!(p, PastePlan::default());
    }

    /// A tree copied into itself never finishes.
    #[test]
    fn a_folder_pasted_inside_itself_is_refused() {
        for into in ["/a/sub", "/a/sub/deeper"] {
            let p = plan(&clip(ClipVerb::Copy, &["/a/sub"]), Path::new(into));
            assert!(p.steps.is_empty(), "{into}");
            assert!(p.refused[0].1.contains("inside itself"), "{into}");
        }
    }

    /// `/a/subway` is not inside `/a/sub` — `starts_with` compares whole
    /// components, which is what makes this safe.
    #[test]
    fn a_folder_with_a_similar_name_is_not_inside() {
        let p = plan(&clip(ClipVerb::Copy, &["/a/sub"]), Path::new("/a/subway"));
        assert_eq!(p.steps.len(), 1);
    }

    #[test]
    fn one_refusal_does_not_stop_the_rest() {
        let p = plan(&clip(ClipVerb::Copy, &["/a/sub", "/a/file.txt"]), Path::new("/a/sub"));
        assert_eq!(p.steps.len(), 1);
        assert_eq!(p.refused.len(), 1);
    }

    #[test]
    fn the_root_has_no_name_to_paste_under() {
        let p = plan(&clip(ClipVerb::Copy, &["/"]), Path::new("/tmp"));
        assert!(p.steps.is_empty());
        assert_eq!(p.refused.len(), 1);
    }

    // --- formats -------------------------------------------------------------

    /// What Nautilus writes, byte for byte: the verb, then URIs, newline
    /// separated, no trailing newline.
    #[test]
    fn a_copy_offers_nautilus_its_own_format() {
        let offers = clip_offers(&clip(ClipVerb::Copy, &["/home/a b/x.txt", "/tmp/y"]));
        let gnome = offers.iter().find(|(m, _)| *m == GNOME_COPIED_FILES).unwrap();
        assert_eq!(gnome.1, "copy\nfile:///home/a%20b/x.txt\nfile:///tmp/y");
        assert!(!offers.iter().any(|(m, _)| *m == KDE_CUT_SELECTION), "no cut marker on a copy");
    }

    #[test]
    fn a_uri_list_ends_every_line_with_crlf() {
        let offers = clip_offers(&clip(ClipVerb::Copy, &["/a", "/b"]));
        let list = offers.iter().find(|(m, _)| *m == URI_LIST).unwrap();
        assert_eq!(list.1, "file:///a\r\nfile:///b\r\n");
    }

    #[test]
    fn a_cut_says_so_to_gnome_and_to_kde() {
        let offers = clip_offers(&clip(ClipVerb::Cut, &["/a"]));
        assert!(offers.iter().any(|(m, c)| *m == GNOME_COPIED_FILES && c.starts_with("cut\n")));
        assert!(offers.iter().any(|(m, c)| *m == KDE_CUT_SELECTION && c == "1"));
    }

    /// Pasting into a terminal gives the paths.
    #[test]
    fn plain_text_is_offered_as_the_paths() {
        let offers = clip_offers(&clip(ClipVerb::Copy, &["/a b", "/c"]));
        assert!(offers.iter().any(|(m, c)| *m == "text/plain" && c == "/a b\n/c"));
    }

    #[test]
    fn what_we_offer_reads_back_as_what_was_copied() {
        for verb in [ClipVerb::Copy, ClipVerb::Cut] {
            let original = clip(verb, &["/home/a b/ünïcödé.txt", "/tmp/100%.txt"]);
            let offers = clip_offers(&original);
            let gnome = offers.iter().find(|(m, _)| *m == GNOME_COPIED_FILES).unwrap();
            assert_eq!(clip_from(GNOME_COPIED_FILES, gnome.1.as_bytes()), Some(original.clone()));
            let list = offers.iter().find(|(m, _)| *m == URI_LIST).unwrap();
            assert_eq!(
                clip_from(URI_LIST, list.1.as_bytes()),
                Some(FileClip { verb: ClipVerb::Copy, ..original }),
                "a plain URI list cannot say cut"
            );
        }
    }

    /// Nautilus terminates with nothing, some tools with a newline, and
    /// RFC 2483 allows comments — all read the same.
    #[test]
    fn other_applications_spellings_are_read() {
        let from_nautilus = b"cut\nfile:///x";
        assert_eq!(clip_from(GNOME_COPIED_FILES, from_nautilus), Some(clip(ClipVerb::Cut, &["/x"])));
        let trailing = b"copy\nfile:///x\n";
        assert_eq!(clip_from(GNOME_COPIED_FILES, trailing), Some(clip(ClipVerb::Copy, &["/x"])));
        let with_comment = b"# from somewhere\r\nfile://localhost/x\r\n";
        assert_eq!(clip_from(URI_LIST, with_comment), Some(clip(ClipVerb::Copy, &["/x"])));
    }

    /// A remote file is not a local path with a funny prefix.
    #[test]
    fn remote_uris_are_left_out() {
        assert_eq!(path_of_uri("sftp://host/x"), None);
        assert_eq!(path_of_uri("file://otherhost/x"), None);
        assert_eq!(clip_from(URI_LIST, b"https://example.com/\r\n"), None, "nothing local to paste");
        let mixed = b"https://example.com/\r\nfile:///keep\r\n";
        assert_eq!(clip_from(URI_LIST, mixed), Some(clip(ClipVerb::Copy, &["/keep"])));
    }

    #[test]
    fn a_verb_we_do_not_know_is_not_guessed_at() {
        assert_eq!(clip_from(GNOME_COPIED_FILES, b"link\nfile:///x"), None);
    }

    #[test]
    fn the_memory_clipboard_holds_one_clip_and_clears() {
        let board = MemoryClipboard::new();
        assert_eq!(board.get(), None);
        board.set(clip(ClipVerb::Copy, &["/a"])).unwrap();
        board.set(clip(ClipVerb::Cut, &["/b"])).unwrap();
        assert_eq!(board.get(), Some(clip(ClipVerb::Cut, &["/b"])), "the newest replaces the older");
        board.clear();
        assert_eq!(board.get(), None);
    }
}
