//! Putting a chosen entry where the rest of the desktop can see it.
//!
//! This crate does not implement the clipboard write itself. That lives
//! in `hyprforge-clipboard` (`ClipboardWriter` for "set the selection",
//! `PasteSynthesizer` for "synthesise Ctrl+V") — built by another agent
//! in parallel with this one, so it did not exist yet when this module
//! was started. Rather than guess at that API up front, this crate
//! defined its own small seam, [`Chooser`], and drove every test in this
//! crate through [`mock::MockChooser`]. `hyprforge-clipboard`'s write
//! side landed before this was finished, so [`Wired`] below is the one
//! place it plugs in — see its doc comment.
use hyprforge_clipboard::Entry;

/// Whatever it takes to act on a chosen entry: put it on the clipboard
/// and paste it into whatever had focus before the popup opened.
pub trait Chooser {
    /// `Err` is a message fit to print to stderr — this crate has no UI
    /// left to show it in by the time the choice has been made, since
    /// choosing is what ends the popup.
    fn choose(&self, entry: &Entry) -> Result<(), String>;
}

/// **The seam**: the real implementation, over `hyprforge-clipboard`'s
/// write-side traits.
///
/// `writer` and `paster` each open their own short-lived Wayland
/// connection on demand (see `WaylandWriter::new` and
/// `WaylandPaster::connect`) rather than sharing the layer-shell
/// connection this popup draws with — the same reasoning
/// `hyprforge-tray::TrayIcon::register` gives for opening its own
/// connection per registration, and it means a paste failure can never
/// take the popup's own surface down with it.
///
/// Failing to *set* the clipboard is the only thing that reports as
/// `Err` here — that is the step a person cannot work around. A
/// compositor with no virtual-keyboard protocol still gets the content
/// onto the clipboard; [`PasteSynthesizer::paste`] reporting
/// [`PasteOutcome::Unavailable`] is not a failure of *choosing*, only of
/// the one optional step after it, so it is reported to stderr and
/// still returns `Ok`.
pub struct Wired {
    writer: hyprforge_clipboard::WaylandWriter,
    paster: hyprforge_clipboard::WaylandPaster,
}

impl Wired {
    /// Connects the paste-synthesis side now, once, rather than per
    /// choice — a popup only ever calls `choose` once before exiting
    /// (see `main.rs`'s module doc), but constructing early means a
    /// compositor with no virtual-keyboard protocol is discovered
    /// before the user has picked anything, in case that is ever worth
    /// surfacing differently.
    pub fn connect() -> anyhow::Result<Self> {
        Ok(Wired {
            writer: hyprforge_clipboard::WaylandWriter::new(),
            paster: hyprforge_clipboard::WaylandPaster::connect()?,
        })
    }
}

impl Chooser for Wired {
    fn choose(&self, entry: &Entry) -> Result<(), String> {
        use hyprforge_clipboard::{ClipboardWriter, PasteOutcome, PasteSynthesizer};

        self.writer
            .set_selection(entry.content.clone())
            .map_err(|e| e.to_string())?;

        if self.paster.paste() == PasteOutcome::Unavailable {
            eprintln!(
                "copied to the clipboard — this compositor has no virtual-keyboard \
                 protocol, so press Ctrl+V yourself to paste it"
            );
        }
        Ok(())
    }
}

#[cfg(test)]
pub mod mock {
    use super::*;
    use hyprforge_clipboard::EntryId;
    use std::cell::RefCell;

    /// Records exactly what it was asked to choose, so a test can assert
    /// on it without a clipboard or a compositor.
    pub struct MockChooser {
        pub calls: RefCell<Vec<EntryId>>,
        pub result: Result<(), String>,
    }

    impl MockChooser {
        pub fn succeeding() -> Self {
            MockChooser { calls: RefCell::new(Vec::new()), result: Ok(()) }
        }

        pub fn failing(message: &str) -> Self {
            MockChooser { calls: RefCell::new(Vec::new()), result: Err(message.to_string()) }
        }
    }

    impl Chooser for MockChooser {
        fn choose(&self, entry: &Entry) -> Result<(), String> {
            self.calls.borrow_mut().push(entry.id.clone());
            self.result.clone()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::mock::MockChooser;
    use super::*;
    use hyprforge_clipboard::{Content, EntryId};

    fn entry(text: &str) -> Entry {
        let content = Content::Text(text.to_string());
        Entry { id: EntryId::of(&content), content, copied_at: 0, pinned: false }
    }

    /// The whole point of the mock: choosing hands the mock *exactly*
    /// the entry that was chosen, not some other one and not a copy that
    /// differs in id.
    #[test]
    fn choosing_an_entry_hands_exactly_that_entry_to_the_chooser() {
        let chooser = MockChooser::succeeding();
        let picked = entry("pick me");
        chooser.choose(&picked).unwrap();
        assert_eq!(chooser.calls.borrow().as_slice(), std::slice::from_ref(&picked.id));
    }

    #[test]
    fn a_failing_chooser_reports_its_message() {
        let chooser = MockChooser::failing("no seat");
        let err = chooser.choose(&entry("x")).unwrap_err();
        assert_eq!(err, "no seat");
    }
}
