//! Back and forward, over the folders this window has shown.
//!
//! The header's ‹ and › in mockup `2a` are the file manager's, in the
//! same place and meaning the same thing: where you were, not the
//! previous picture. Paging through pictures is ← and →, and the overlay
//! arrows on the photo; a folder visited from the sidebar or the library
//! is what Back takes you out of.

use std::path::PathBuf;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct History {
    back: Vec<PathBuf>,
    forward: Vec<PathBuf>,
}

impl History {
    /// Leaving `from` for somewhere new. Going somewhere new ends the
    /// forward trail, as it does in every browser.
    pub fn leave(&mut self, from: PathBuf) {
        if self.back.last() != Some(&from) {
            self.back.push(from);
        }
        self.forward.clear();
    }

    /// Where Back goes from `current`, recording `current` for Forward.
    pub fn back(&mut self, current: PathBuf) -> Option<PathBuf> {
        let to = self.back.pop()?;
        self.forward.push(current);
        Some(to)
    }

    pub fn forward(&mut self, current: PathBuf) -> Option<PathBuf> {
        let to = self.forward.pop()?;
        self.back.push(current);
        Some(to)
    }

    pub fn can_go_back(&self) -> bool {
        !self.back.is_empty()
    }

    pub fn can_go_forward(&self) -> bool {
        !self.forward.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(s: &str) -> PathBuf {
        PathBuf::from(s)
    }

    #[test]
    fn back_returns_to_the_folder_you_left_and_forward_undoes_it() {
        let mut h = History::default();
        h.leave(p("/a"));
        assert_eq!(h.back(p("/b")), Some(p("/a")));
        assert!(h.can_go_forward());
        assert_eq!(h.forward(p("/a")), Some(p("/b")));
        assert!(!h.can_go_forward());
    }

    #[test]
    fn going_somewhere_new_ends_the_forward_trail() {
        let mut h = History::default();
        h.leave(p("/a"));
        h.back(p("/b"));
        h.leave(p("/a"));
        assert!(!h.can_go_forward());
    }

    #[test]
    fn a_fresh_window_has_nowhere_to_go_back_to() {
        let mut h = History::default();
        assert!(!h.can_go_back());
        assert_eq!(h.back(p("/a")), None);
    }
}
