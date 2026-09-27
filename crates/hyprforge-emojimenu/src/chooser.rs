//! Putting a picked emoji where the rest of the desktop can see it.
//!
//! Almost the same seam `hyprforge-clipmenu::chooser::Chooser` defines,
//! and for the same reason: [`dispatch_action`](crate::popup_app::dispatch_action)
//! is tested end to end against [`mock::MockChooser`], with no
//! compositor and no daemon involved. The one difference from
//! `hyprforge-clipmenu`'s version is what [`Chooser::set_clipboard`]
//! takes — a plain `&str`, not an already-recorded
//! `hyprforge_clipboard::Entry` — because a picked emoji was never
//! copied through `hyprforge-clipd`'s watcher and has no history id to
//! name. [`Wired::set_clipboard`] is the one place that difference
//! actually shows up on the wire: it calls
//! `hyprforge_clipboard::ipc::set_clipboard_text` (which this crate's
//! own work added — see that function's doc for why the daemon needed a
//! second `set-clipboard*` command) rather than `set_clipboard(id)`.
//! Everything downstream of that — the two-call, teardown-then-paste
//! ordering `finish_paste` embodies — is identical to `hyprforge-clipmenu`'s
//! own `Wired`, because the ordering hazard it guards against
//! (`hyprforge-popup::finish_after_teardown`) is exactly the same one.
//!
//! `hyprforge-clipmenu` is a binary target of `hyprforge-clipboard`, not
//! part of its library, so its `chooser`/`target` modules are private to
//! that binary and cannot be imported here — this is a deliberate, small, hand-copied adaptation
//! of that crate's own logic, not a dependency on it.

use hyprforge_clipboard::ipc::ClientError;
use hyprforge_clipboard::{ClipboardWriter, Content, Shortcut};
use std::sync::Mutex;
use std::time::Duration;

/// See `hyprforge-clipmenu::chooser`'s identical constant for the full
/// reasoning — this crate's `finish_paste` waits on the exact same
/// per-process source-lifetime hazard `hyprforge_clipboard::write`'s
/// module doc describes.
const SELECTION_WAIT: Duration = Duration::from_secs(2);

/// Whatever it takes to act on a picked emoji: put it on the clipboard,
/// then paste it into whatever had focus before the popup opened. See
/// `hyprforge-clipmenu::chooser::Chooser`'s own doc for why these are two
/// separate calls with the popup's own teardown strictly between them —
/// the same ordering hazard, the same fix.
pub trait Chooser {
    /// Puts `text` on the clipboard. `Err` is a message fit to print to
    /// stderr — by the time a choice has been made there is no UI left
    /// open to show it in, since choosing is what ends the popup.
    fn set_clipboard(&self, text: &str) -> Result<(), String>;

    /// Synthesizes `shortcut` and waits (bounded) for the clipboard
    /// source `set_clipboard` created to be read or superseded. Call
    /// only after `set_clipboard` returned `Ok`, and only once this
    /// popup's own surface is gone — see the trait's own doc.
    fn finish_paste(&self, shortcut: Shortcut);
}

/// **The seam**: the real implementation.
///
/// Tries `hyprforge-clipd` first — asking it to own the selection is
/// what makes a picked emoji pasteable more than once after this
/// short-lived popup has already exited, the same reason
/// `hyprforge-clipmenu::chooser::Wired` asks it for an already-recorded
/// entry. Falls back to owning the selection itself, from this process,
/// when the daemon cannot be reached — CLAUDE.md's "every component runs
/// alone": a picker with no clipboard daemon installed still works, it
/// only loses the "can be pasted again after this process exits" half.
pub struct Wired {
    writer: hyprforge_clipboard::WaylandWriter,
    paster: hyprforge_clipboard::WaylandPaster,
    /// See `hyprforge-clipmenu::chooser::Wired::guard`'s identical doc:
    /// held here so `finish_paste` (a separate call, on the same
    /// `&self`) can wait on it. There is never real contention — this
    /// popup only ever chooses once before exiting.
    guard: Mutex<Option<<hyprforge_clipboard::WaylandWriter as ClipboardWriter>::Guard>>,
}

impl Wired {
    pub fn connect() -> anyhow::Result<Self> {
        Ok(Wired {
            writer: hyprforge_clipboard::WaylandWriter::new(),
            paster: hyprforge_clipboard::WaylandPaster::connect()?,
            guard: Mutex::new(None),
        })
    }
}

impl Chooser for Wired {
    fn set_clipboard(&self, text: &str) -> Result<(), String> {
        match hyprforge_clipboard::ipc::set_clipboard_text(text) {
            Ok(()) => return Ok(()),
            // Nobody to ask — not an error, and not a reason to refuse
            // to work; see `hyprforge-clipmenu::chooser::Wired`'s
            // identical branch for the same reasoning.
            Err(ClientError::Unreachable(_) | ClientError::NoRuntimeDir) => {}
            // The daemon answered and said no — a real answer, passed
            // through rather than papered over by a fallback that
            // behaves differently.
            Err(e) => return Err(e.to_string()),
        }

        let guard = self
            .writer
            .set_selection(Content::Text(text.to_string()))
            .map_err(|e| e.to_string())?;
        *self.guard.lock().unwrap_or_else(|e| e.into_inner()) = Some(guard);
        Ok(())
    }

    fn finish_paste(&self, shortcut: Shortcut) {
        use hyprforge_clipboard::{PasteOutcome, PasteSynthesizer, SelectionGuard, SelectionOutcome};

        if self.paster.paste(shortcut) == PasteOutcome::Unavailable {
            let keys = match shortcut {
                Shortcut::CtrlV => "Ctrl+V",
                Shortcut::CtrlShiftV => "Ctrl+Shift+V",
            };
            eprintln!(
                "copied to the clipboard — this compositor has no virtual-keyboard \
                 protocol, so press {keys} yourself to paste it"
            );
        }

        let Some(guard) = self.guard.lock().unwrap_or_else(|e| e.into_inner()).take() else {
            // `finish_paste` without a preceding successful
            // `set_clipboard` (the daemon owned the selection instead,
            // so there is no local guard to wait on) — nothing to do.
            return;
        };

        match guard.wait(SELECTION_WAIT) {
            SelectionOutcome::Served | SelectionOutcome::Superseded => {}
            SelectionOutcome::TimedOut => {
                eprintln!(
                    "copied to the clipboard, but nothing pasted it within {SELECTION_WAIT:?} — \
                     if the paste didn't happen, copy it again before pasting by hand"
                );
            }
        }
    }
}

#[cfg(test)]
pub mod mock {
    use super::*;
    use std::cell::RefCell;

    /// Records exactly what it was asked to put on the clipboard, so a
    /// test can assert on it without a clipboard or a compositor.
    pub struct MockChooser {
        pub calls: RefCell<Vec<String>>,
        pub result: Result<(), String>,
        pub log: RefCell<Vec<&'static str>>,
    }

    impl MockChooser {
        pub fn succeeding() -> Self {
            MockChooser { calls: RefCell::new(Vec::new()), result: Ok(()), log: RefCell::new(Vec::new()) }
        }

        pub fn failing(message: &str) -> Self {
            MockChooser {
                calls: RefCell::new(Vec::new()),
                result: Err(message.to_string()),
                log: RefCell::new(Vec::new()),
            }
        }
    }

    impl Chooser for MockChooser {
        fn set_clipboard(&self, text: &str) -> Result<(), String> {
            self.calls.borrow_mut().push(text.to_string());
            self.log.borrow_mut().push("set_clipboard");
            self.result.clone()
        }

        fn finish_paste(&self, _shortcut: Shortcut) {
            self.log.borrow_mut().push("finish_paste");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::mock::MockChooser;
    use super::*;

    #[test]
    fn choosing_an_emoji_hands_exactly_that_text_to_the_chooser() {
        let chooser = MockChooser::succeeding();
        chooser.set_clipboard("👋").unwrap();
        assert_eq!(chooser.calls.borrow().as_slice(), &["👋".to_string()]);
    }

    #[test]
    fn a_failing_chooser_reports_its_message() {
        let chooser = MockChooser::failing("no seat");
        let err = chooser.set_clipboard("👋").unwrap_err();
        assert_eq!(err, "no seat");
    }

    /// The same ordering the clipboard popup pins:
    /// `set_clipboard` before `finish_paste`, never the reverse, and
    /// never welded into one call — see
    /// `crate::popup_app`'s own regression test for the version of this
    /// that catches the two being merged back together.
    #[test]
    fn finishing_a_choice_runs_set_clipboard_before_finish_paste() {
        let chooser = MockChooser::succeeding();
        chooser.set_clipboard("👋").unwrap();
        chooser.finish_paste(Shortcut::CtrlV);
        assert_eq!(chooser.log.borrow().as_slice(), &["set_clipboard", "finish_paste"]);
    }
}
