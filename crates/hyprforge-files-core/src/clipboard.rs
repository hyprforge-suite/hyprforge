//! Copied and cut files, and what pasting them means.
//!
//! Two halves, kept apart.
//!
//! **Where the copied files are kept** is [`FileClipboard`], a trait —
//! the same "seam before the client" shape as `backend::FsBackend`. The
//! in-app [`MemoryClipboard`] is what exists today; a Wayland one that
//! puts files on the *system* clipboard, so a copy in Files pastes into
//! another file manager and back, plugs in behind the same trait. Nothing
//! above the trait changes when it does.
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
