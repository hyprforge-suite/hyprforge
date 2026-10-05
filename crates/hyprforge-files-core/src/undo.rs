//! What can be taken back, and in what order.
//!
//! A record of things that were done, kept as plain paths: the history
//! decides nothing and touches nothing. Carrying an undo out is I/O, so
//! it is the window's job — and every undo checks, before acting, that
//! the world still looks the way the record says, and refuses with a
//! sentence rather than guessing when it does not.
//!
//! What undo means for each:
//!
//! - **Trashed** — put the items back where they came from.
//! - **Renamed** — rename back.
//! - **Renamed several at once** — rename them all back, as one batch:
//!   a bulk rename that swapped two names is undone by swapping them
//!   again, which no sequence of single renames back can do.
//! - **Moved** — move back.
//! - **Copied** — move the copies to the Trash. Never delete them: an
//!   undo that loses a file is worse than no undo.
//! - **Made a folder** — remove it, only if it is still empty.
//! - **Extracted an archive** — move the folder it made to the Trash.
//! - **Made an archive** — move it to the Trash.
//!
//! A permanent delete is not here, because there is nothing to take back
//! from — which is what its confirmation dialog says.
//!
//! Neither is *editing* an archive, and that one is a decision rather
//! than an omission. Every format here rewrites the whole file to change
//! one member, so the only honest undo is a copy of the archive as it
//! was — which for a large one is a multi-gigabyte hidden file sitting
//! beside it for the length of an undo window, on the same filesystem
//! (it cannot go in `/tmp`, which is very often memory), with nothing
//! able to promise it gets cleaned up if the process dies. An undo that
//! quietly costs the size of your archive is not a good trade for
//! renaming something inside it, and one that sometimes has the old copy
//! and sometimes does not — by size, say — is worse than one that never
//! claims to.

use std::collections::VecDeque;
use std::path::PathBuf;

/// One thing that was done.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Undoable {
    /// Items moved to the Trash: each one's stored path, the path it came
    /// from, and its `.trashinfo` record.
    Trashed(Vec<(PathBuf, PathBuf, PathBuf)>),
    Renamed { from: PathBuf, to: PathBuf },
    /// A bulk rename, as `(what it was called, what it is called now)` —
    /// one step, so one Ctrl+Z takes the whole batch back.
    RenamedAll(Vec<(PathBuf, PathBuf)>),
    /// Items moved, as `(where it was, where it is now)`.
    Moved(Vec<(PathBuf, PathBuf)>),
    /// Copies made, where they are.
    Copied(Vec<PathBuf>),
    MadeFolder(PathBuf),
    /// An archive was unpacked into this folder.
    ///
    /// The folder, not the files in it: an extraction makes hundreds of
    /// paths and listing them all would make the record larger than the
    /// listing it came from. It is also why undoing this only works on
    /// a folder the extraction itself made — see the window's own check.
    Extracted(PathBuf),
    /// An archive was made here.
    Compressed(PathBuf),
}

impl Undoable {
    /// What this moved, as `(where it was, where it is now)` — a rename
    /// is a move within a folder. Empty for anything that moved nothing
    /// a star could be on; a trashed file's star stays where it was and
    /// is shown as missing, the way a pin is. See [`crate::starred`].
    pub fn moves(&self) -> Vec<(PathBuf, PathBuf)> {
        match self {
            Undoable::Renamed { from, to } => vec![(from.clone(), to.clone())],
            Undoable::RenamedAll(items) | Undoable::Moved(items) => items.clone(),
            _ => Vec::new(),
        }
    }

    /// What undoing this moves: [`Self::moves`], each the other way.
    pub fn undo_moves(&self) -> Vec<(PathBuf, PathBuf)> {
        self.moves().into_iter().map(|(was, now)| (now, was)).collect()
    }

    /// What the notice offering to undo it says.
    pub fn describe(&self) -> String {
        match self {
            Undoable::Trashed(items) => format!("Moved {} to the Trash", count(items.len())),
            Undoable::Renamed { to, .. } => format!("Renamed to \u{201C}{}\u{201D}", name(to)),
            Undoable::RenamedAll(items) => format!("Renamed {}", count(items.len())),
            Undoable::Moved(items) => format!("Moved {}", count(items.len())),
            Undoable::Copied(items) => format!("Copied {}", count(items.len())),
            Undoable::MadeFolder(path) => format!("Made \u{201C}{}\u{201D}", name(path)),
            Undoable::Extracted(into) => {
                format!("Extracted into \u{201C}{}\u{201D}", name(into))
            }
            Undoable::Compressed(archive) => {
                format!("Made \u{201C}{}\u{201D}", name(archive))
            }
        }
    }

    /// Whether there is anything in it to take back. A job that placed
    /// nothing records nothing.
    pub fn is_empty(&self) -> bool {
        match self {
            Undoable::Trashed(items) => items.is_empty(),
            Undoable::Moved(items) => items.is_empty(),
            Undoable::Copied(items) => items.is_empty(),
            Undoable::RenamedAll(items) => items.is_empty(),
            Undoable::Renamed { .. }
            | Undoable::MadeFolder(_)
            | Undoable::Extracted(_)
            | Undoable::Compressed(_) => false,
        }
    }
}

fn count(n: usize) -> String {
    if n == 1 {
        "1 item".to_string()
    } else {
        format!("{n} items")
    }
}

fn name(path: &std::path::Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

/// The most recent things done, newest last.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UndoHistory {
    entries: VecDeque<Undoable>,
    depth: usize,
}

impl UndoHistory {
    /// A history that remembers the last `depth` things. Short on
    /// purpose: an undo is for the thing you just did, and a record from
    /// an hour ago is more likely to meet a folder that has moved on than
    /// to do what anyone wants.
    pub fn new(depth: usize) -> Self {
        UndoHistory { entries: VecDeque::with_capacity(depth), depth }
    }

    /// Remembers `done`, forgetting the oldest past the depth. An empty
    /// record is not remembered — undoing it would do nothing, and would
    /// use up the Ctrl+Z meant for the thing before it.
    pub fn push(&mut self, done: Undoable) {
        if done.is_empty() || self.depth == 0 {
            return;
        }
        if self.entries.len() == self.depth {
            self.entries.pop_front();
        }
        self.entries.push_back(done);
    }

    /// The most recent thing, taken off the history.
    pub fn pop(&mut self) -> Option<Undoable> {
        self.entries.pop_back()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Everything remembered, **newest first** — the order the history
    /// list shows, and the order Ctrl+Z works through. Only the first can
    /// be undone now: each undo checks the folder still looks the way the
    /// record left it, and taking an older one back first would check
    /// against a state the newer one has since changed.
    pub fn newest_first(&self) -> impl Iterator<Item = &Undoable> {
        self.entries.iter().rev()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn copied(n: usize) -> Undoable {
        Undoable::Copied((0..n).map(|i| PathBuf::from(format!("/c{i}"))).collect())
    }

    /// What stars follow: a rename and a move are moves, undoing one is
    /// the move back, and a copy or a trash moves nothing.
    #[test]
    fn a_rename_is_a_move_and_undoing_it_is_the_move_back() {
        let renamed = Undoable::Renamed { from: "/d/a".into(), to: "/d/b".into() };
        assert_eq!(renamed.moves(), [(PathBuf::from("/d/a"), PathBuf::from("/d/b"))]);
        assert_eq!(renamed.undo_moves(), [(PathBuf::from("/d/b"), PathBuf::from("/d/a"))]);
        assert!(copied(2).moves().is_empty());
        assert!(Undoable::Trashed(vec![("/t/a".into(), "/d/a".into(), "/t/a.info".into())]).moves().is_empty());
    }

    /// The history list and Ctrl+Z agree about what comes next: the list's
    /// first row is what `pop` takes.
    #[test]
    fn the_history_lists_newest_first_and_its_first_row_is_what_undo_takes() {
        let mut history = UndoHistory::new(10);
        history.push(copied(1));
        history.push(Undoable::MadeFolder("/new".into()));
        let listed: Vec<Undoable> = history.newest_first().cloned().collect();
        assert_eq!(listed, [Undoable::MadeFolder("/new".into()), copied(1)]);
        assert_eq!(history.len(), 2);
        assert_eq!(history.pop().as_ref(), listed.first());
    }

    #[test]
    fn the_newest_thing_is_undone_first() {
        let mut history = UndoHistory::new(10);
        history.push(copied(1));
        history.push(Undoable::MadeFolder("/new".into()));
        assert_eq!(history.pop(), Some(Undoable::MadeFolder("/new".into())));
        assert_eq!(history.pop(), Some(copied(1)));
        assert_eq!(history.pop(), None);
    }

    #[test]
    fn the_oldest_is_forgotten_past_the_depth() {
        let mut history = UndoHistory::new(2);
        history.push(copied(1));
        history.push(copied(2));
        history.push(copied(3));
        assert_eq!(history.pop(), Some(copied(3)));
        assert_eq!(history.pop(), Some(copied(2)));
        assert_eq!(history.pop(), None, "the first was forgotten");
    }

    /// A job that placed nothing must not use up the Ctrl+Z meant for
    /// the thing before it.
    #[test]
    fn nothing_done_is_not_remembered() {
        let mut history = UndoHistory::new(5);
        history.push(copied(1));
        history.push(copied(0));
        history.push(Undoable::Trashed(vec![]));
        assert_eq!(history.pop(), Some(copied(1)));
    }

    #[test]
    fn a_depth_of_zero_turns_undo_off() {
        let mut history = UndoHistory::new(0);
        history.push(copied(1));
        assert!(history.is_empty());
    }

    #[test]
    fn each_record_describes_itself_for_the_notice() {
        assert_eq!(
            Undoable::Trashed(vec![("/t/a".into(), "/a".into(), "/i/a".into()); 3]).describe(),
            "Moved 3 items to the Trash"
        );
        assert_eq!(
            Undoable::Renamed { from: "/a.txt".into(), to: "/b.txt".into() }.describe(),
            "Renamed to \u{201C}b.txt\u{201D}"
        );
        assert_eq!(copied(1).describe(), "Copied 1 item");
        assert_eq!(
            Undoable::RenamedAll(vec![("/a".into(), "/b".into()), ("/b".into(), "/a".into())]).describe(),
            "Renamed 2 items"
        );
    }
}
