//! Copied files on the Wayland clipboard, where other applications can
//! paste them — and where theirs can be pasted from.
//!
//! Writes offer what Nautilus, Dolphin and a terminal each look for (see
//! `hyprforge_files_core::clipboard::clip_offers`). Reads ask for
//! GNOME's copied-files type, then the plain URI list, once, at the
//! moment Paste is pressed — never on a timer, and never on the thread
//! that paints: [`FileClipboard::get`] here can wait on the compositor
//! and on whichever application owns the selection.
//!
//! An in-process copy is kept alongside. It is what `get` answers when
//! the system clipboard cannot be read at all — no compositor, or one
//! with only `zwlr-data-control-v1`, which `hyprforge_clipboard` can
//! write to but not read from once — so copy and paste within this app
//! work everywhere, and across applications wherever the compositor
//! allows. Every component runs alone: none of this needs the clipboard
//! history daemon installed, let alone running.

use hyprforge_clipboard::{Mime, Offers, WaylandWriter};
use hyprforge_files_core::clipboard::{clip_from, clip_offers, FileClip, FileClipboard, MemoryClipboard, PASTE_TYPES};
use std::time::Duration;

/// How long a paste waits for the clipboard's owner to answer. Longer
/// than a person notices and shorter than they give up.
const READ_TIMEOUT: Duration = Duration::from_millis(1500);

#[derive(Default)]
pub struct SystemClipboard {
    here: MemoryClipboard,
    writer: WaylandWriter,
}

impl SystemClipboard {
    pub fn new() -> Self {
        Self::default()
    }

    /// What the system clipboard holds, if it can be read: `Ok(None)`
    /// means it holds something that is not files.
    fn read_system(&self) -> Result<Option<FileClip>, String> {
        let wanted = PASTE_TYPES.iter().map(|m| Mime::new(*m)).collect();
        match hyprforge_clipboard::read_selection(wanted, READ_TIMEOUT) {
            Ok(Some((mime, bytes))) => Ok(clip_from(mime.as_str(), &bytes)),
            Ok(None) => Ok(None),
            Err(e) => Err(e.to_string()),
        }
    }
}

impl FileClipboard for SystemClipboard {
    fn set(&self, clip: FileClip) -> Result<(), String> {
        let offers = Offers::new(
            clip_offers(&clip)
                .into_iter()
                .map(|(mime, text)| (Mime::new(mime), text.into_bytes()))
                .collect(),
        );
        self.here.set(clip)?;
        // A failure here is logged, not shown: copy and paste inside this
        // app still work from the in-process copy, and the one thing that
        // is lost — pasting into another application — is not something
        // the status bar could fix.
        if let Err(e) = self.writer.set_offers(offers) {
            tracing::warn!(error = %e, "copied files could not be put on the system clipboard");
        }
        Ok(())
    }

    fn get(&self) -> Option<FileClip> {
        match self.read_system() {
            // The system clipboard is the truth when it can be read: a
            // copy of some text in another application since, means there
            // are no files to paste, even though this app remembers some.
            Ok(found) => found,
            Err(e) => {
                tracing::info!(error = %e, "the system clipboard could not be read; using this app's own copy");
                self.here.get()
            }
        }
    }

    fn clear(&self) {
        let ours = self.here.get();
        self.here.clear();
        // Only empty the system clipboard if it still holds what this app
        // put there — otherwise it now belongs to something copied
        // elsewhere, which a finished paste here has no business erasing.
        if ours.is_some() && self.read_system().ok().flatten() == ours {
            if let Err(e) = self.writer.set_offers(Offers::default()) {
                tracing::warn!(error = %e, "the system clipboard could not be emptied after a move");
            }
        }
    }

    /// Always `true`: whether another application has since put files on
    /// the clipboard cannot be known without asking it, and this is asked
    /// every time a menu opens. Paste finds out.
    fn may_hold_files(&self) -> bool {
        true
    }
}
