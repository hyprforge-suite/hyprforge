//! Putting content back on the clipboard — the write side of the boundary
//! [`crate::backend`] draws for reading it.
//!
//! Same shape as `backend.rs`: a trait plus a mock so a popup can be
//! tested without a compositor, and the real implementation
//! ([`crate::wayland::WaylandWriter`]) lives behind it. The two pure
//! functions here ([`mimes_to_offer`] and [`bytes_for`]) are what decide
//! *what* to advertise and *which* bytes answer which offer — the part
//! most worth testing without any Wayland involved at all — and both the
//! real writer and the mock call them, so a test on either is a test of
//! the same decision.
//!
//! # Never the primary selection
//!
//! `ext-data-control-v1` and `zwlr-data-control-v1` can each also set the
//! X11-style "whatever is currently highlighted" selection
//! (`set_primary_selection`). Nothing in this module or
//! `crate::wayland::write_ext`/`write_wlr` ever calls it — only
//! `set_selection`, the ordinary clipboard. Setting the primary selection
//! from a clipboard manager would mean every popup selection also
//! silently overwrites whatever text the user has merely highlighted
//! elsewhere, which is not what "paste this" was asking for.

use crate::types::{Content, Mime};

/// Puts `content` on the clipboard.
///
/// Implementations own a data source that must **stay alive** for as
/// long as this remains the current selection — see
/// `crate::wayland::write_ext`'s module doc for why a source dropped
/// right after this call returns would leave the clipboard empty. This
/// method itself only has to make the compositor accept the new
/// selection; keeping the source alive afterward is the implementation's
/// job, not the caller's.
pub trait ClipboardWriter: Send + Sync {
    fn set_selection(&self, content: Content) -> anyhow::Result<()>;
}

/// Which MIME types to advertise for a piece of content.
///
/// Text offers both `text/plain;charset=utf-8` (what most modern
/// applications look for first) and plain `text/plain` (what the rest
/// still look for) — this crate's own `resolve::select_mime` prefers the
/// former over the latter for exactly the same reason a paste target
/// might. An image offers only the one MIME type it was stored with:
/// re-encoding it as anything else is not something this crate does
/// anywhere, on the read side either.
pub fn mimes_to_offer(content: &Content) -> Vec<Mime> {
    match content {
        Content::Text(_) => vec![
            Mime::new("text/plain;charset=utf-8"),
            Mime::new("text/plain"),
        ],
        Content::Image { mime, .. } => vec![mime.clone()],
    }
}

/// The bytes to answer a `send` request for `requested`, or `None` when
/// `requested` is not one of [`mimes_to_offer`]'s own list for this
/// content — which should not happen against a well-behaved compositor,
/// since nothing else was ever advertised, but a source must still
/// answer *something* rather than write nothing at all to a paste
/// target's pipe.
pub fn bytes_for(content: &Content, requested: &Mime) -> Option<Vec<u8>> {
    match content {
        Content::Text(text) => mimes_to_offer(content)
            .contains(requested)
            .then(|| text.clone().into_bytes()),
        Content::Image { bytes, mime } => (requested == mime).then(|| bytes.clone()),
    }
}

#[cfg(any(test, feature = "mock"))]
pub mod mock {
    use super::*;
    use std::sync::Mutex;

    /// Records every call rather than talking to a compositor — for the
    /// popup's own tests. Never actually touches the machine's real
    /// clipboard.
    #[derive(Default)]
    pub struct MockWriter {
        calls: Mutex<Vec<Content>>,
    }

    impl MockWriter {
        pub fn new() -> Self {
            Self::default()
        }

        /// Every `set_selection` call so far, in order.
        pub fn calls(&self) -> Vec<Content> {
            self.calls.lock().unwrap_or_else(|e| e.into_inner()).clone()
        }

        /// The MIME types the *last* call would have offered — what a
        /// test asserts to check the right types are advertised, without
        /// re-deriving `mimes_to_offer` itself.
        pub fn last_offered(&self) -> Option<Vec<Mime>> {
            self.calls
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .last()
                .map(mimes_to_offer)
        }
    }

    impl ClipboardWriter for MockWriter {
        fn set_selection(&self, content: Content) -> anyhow::Result<()> {
            self.calls
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .push(content);
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The property Part 1 of the task exists to pin: text offers both
    /// plain-text MIME types.
    #[test]
    fn setting_text_offers_both_plain_text_mime_types() {
        let offered = mimes_to_offer(&Content::Text("hello".to_string()));
        assert_eq!(
            offered,
            vec![
                Mime::new("text/plain;charset=utf-8"),
                Mime::new("text/plain"),
            ]
        );
    }

    /// An image offers exactly the type it was stored with — never
    /// re-typed as something else.
    #[test]
    fn setting_an_image_offers_only_the_type_it_was_stored_with() {
        let offered = mimes_to_offer(&Content::Image {
            bytes: vec![1, 2, 3],
            mime: Mime::new("image/png"),
        });
        assert_eq!(offered, vec![Mime::new("image/png")]);
    }

    #[test]
    fn bytes_for_answers_either_text_mime_with_the_same_bytes() {
        let content = Content::Text("hello".to_string());
        assert_eq!(
            bytes_for(&content, &Mime::new("text/plain;charset=utf-8")),
            Some(b"hello".to_vec())
        );
        assert_eq!(
            bytes_for(&content, &Mime::new("text/plain")),
            Some(b"hello".to_vec())
        );
        assert_eq!(bytes_for(&content, &Mime::new("text/html")), None);
    }

    #[test]
    fn bytes_for_an_image_only_answers_its_own_mime() {
        let content = Content::Image {
            bytes: vec![9, 9, 9],
            mime: Mime::new("image/png"),
        };
        assert_eq!(
            bytes_for(&content, &Mime::new("image/png")),
            Some(vec![9, 9, 9])
        );
        assert_eq!(bytes_for(&content, &Mime::new("image/jpeg")), None);
    }

    /// The mock exists so the popup can assert what would have been
    /// offered without a compositor — this pins that it actually tracks
    /// calls and reports the right offer list per the shared function
    /// above, rather than a hand-rolled duplicate of the decision.
    #[test]
    fn the_mock_reports_what_the_last_call_would_have_offered() {
        let writer = mock::MockWriter::new();
        writer
            .set_selection(Content::Text("first".to_string()))
            .unwrap();
        writer
            .set_selection(Content::Image {
                bytes: vec![1],
                mime: Mime::new("image/png"),
            })
            .unwrap();
        assert_eq!(writer.calls().len(), 2);
        assert_eq!(writer.last_offered(), Some(vec![Mime::new("image/png")]));
    }
}
