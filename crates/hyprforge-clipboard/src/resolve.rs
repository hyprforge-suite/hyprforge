//! Deciding what an accumulated offer is worth keeping — before any of
//! its bytes are read.
//!
//! Two things live here on purpose, in one file: choosing which of an
//! offer's several MIME types to keep ([`select_mime`], a pure function
//! and the piece most likely to be wrong — the wlr crate exists because
//! it is the easiest thing here to get right without a compositor), and
//! the *order* reads happen in ([`resolve_offer`]). That order is the
//! point of the crate: [`Sensitivity::classify`] runs before any content
//! byte is requested, and the only read allowed to happen first is a
//! read of the password hint's own value — classifying it requires
//! nothing less, and `resolve_offer`'s structure makes it hard to get
//! that order backwards.

use crate::types::{Content, Mime, Sensitivity, PASSWORD_HINT};

/// Picks the one MIME type worth keeping out of everything an offer
/// announced, or `None` when nothing usable was offered — an offer of
/// only `text/html`, say, is skipped rather than stored as something
/// blank.
///
/// Order: an image type first (the user asked for images to work), then
/// `text/plain;charset=utf-8`, then plain `text/plain`. HTML is never
/// preferred over plain text — the history is a list of things to
/// paste, and a wall of markup where the user copied a sentence is
/// wrong — and it is never picked at all, since nothing here reads it.
pub fn select_mime(mimes: &[Mime]) -> Option<Mime> {
    if let Some(image) = mimes.iter().find(|m| m.is_image()) {
        return Some(image.clone());
    }
    const TEXT_PREFERENCE: [&str; 2] = ["text/plain;charset=utf-8", "text/plain"];
    TEXT_PREFERENCE.iter().find_map(|wanted| {
        mimes
            .iter()
            .find(|m| m.as_str().eq_ignore_ascii_case(wanted))
            .cloned()
    })
}

/// Turns everything one offer announced into `Content`, or `None` when
/// it should not be kept at all — either because it is secret, or
/// because it offered nothing worth keeping.
///
/// `receive_mime` is the *only* place a content byte ever crosses from
/// the source application into this process. It is taken as a closure
/// so this function carries no Wayland, no pipe and no timeout of its
/// own, and can be exercised with a fake that just records what it was
/// asked to read — which is the test that matters most here. Returning
/// `None` from it means "could not be read" (a timeout, a closed offer,
/// an I/O error), and that is treated the same as "does not have this
/// type" — never as an empty read, and for the password hint, never as
/// "safe to record".
pub fn resolve_offer(
    mimes: &[Mime],
    mut receive_mime: impl FnMut(&Mime) -> Option<Vec<u8>>,
) -> Option<Content> {
    // The one read allowed before classification. `classify` only asks
    // for `hint_value` when `mimes` itself advertises the hint type, so
    // this skips a read for the overwhelming majority of copies that
    // never mention it — and correctly, since a read that fails here
    // collapses to the same `None` `classify` already treats as secret.
    let advertises_hint = mimes.iter().any(|m| m.as_str() == PASSWORD_HINT);
    let hint_value: Option<String> = if advertises_hint {
        receive_mime(&Mime::new(PASSWORD_HINT))
            .map(|bytes| String::from_utf8_lossy(&bytes).trim().to_string())
    } else {
        None
    };
    if Sensitivity::classify(mimes, hint_value.as_deref()).is_secret() {
        return None;
    }

    // Only past this point is a content byte ever requested.
    let chosen = select_mime(mimes)?;
    let bytes = receive_mime(&chosen)?;
    let content = if chosen.is_image() {
        Content::Image {
            bytes,
            mime: chosen,
        }
    } else {
        Content::Text(String::from_utf8_lossy(&bytes).into_owned())
    };
    if content.is_empty() {
        return None;
    }
    Some(content)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::PASSWORD_HINT_SECRET;
    use std::cell::RefCell;

    fn mimes(list: &[&str]) -> Vec<Mime> {
        list.iter().map(|m| Mime::new(*m)).collect()
    }

    #[test]
    fn selection_prefers_an_image_over_any_text() {
        let picked = select_mime(&mimes(&["text/plain", "text/html", "image/png"]));
        assert_eq!(picked, Some(Mime::new("image/png")));
    }

    #[test]
    fn selection_prefers_utf8_text_over_plain_text() {
        let picked = select_mime(&mimes(&["text/plain", "text/plain;charset=utf-8"]));
        assert_eq!(picked, Some(Mime::new("text/plain;charset=utf-8")));
    }

    /// The history is a list of things to paste. Choosing the markup an
    /// application also offered would turn "the user copied a sentence"
    /// into "the user copied a wall of tags".
    #[test]
    fn selection_never_prefers_html_over_plain_text() {
        let picked = select_mime(&mimes(&["text/html", "text/plain"]));
        assert_eq!(picked, Some(Mime::new("text/plain")));
    }

    /// Nothing here reads HTML at all: an offer of only markup has no
    /// type this crate is willing to keep.
    #[test]
    fn an_offer_of_only_html_has_no_usable_type() {
        assert_eq!(select_mime(&mimes(&["text/html"])), None);
    }

    #[test]
    fn an_offer_with_no_types_at_all_has_no_usable_type() {
        assert_eq!(select_mime(&[]), None);
    }

    /// The test that matters most: a secret offer is never read for its
    /// content, only (necessarily) for the hint that condemns it.
    #[test]
    fn a_secret_offer_is_never_read_for_content_only_for_its_hint() {
        let offer = mimes(&["text/plain", PASSWORD_HINT]);
        let read_log = RefCell::new(Vec::new());
        let result = resolve_offer(&offer, |mime| {
            read_log.borrow_mut().push(mime.clone());
            Some(PASSWORD_HINT_SECRET.as_bytes().to_vec())
        });
        assert_eq!(result, None, "a secret offer must never be recorded");
        assert_eq!(
            read_log.into_inner(),
            vec![Mime::new(PASSWORD_HINT)],
            "the only read a secret offer may cause is the hint's own value"
        );
    }

    /// An unreadable hint is treated as secret (see `Sensitivity`'s own
    /// tests), and here that must still mean no content read follows.
    #[test]
    fn a_hint_that_cannot_be_read_still_blocks_every_further_read() {
        let offer = mimes(&["text/plain", PASSWORD_HINT]);
        let read_log = RefCell::new(Vec::new());
        let result = resolve_offer(&offer, |mime| {
            read_log.borrow_mut().push(mime.clone());
            None // every read fails, including the hint's
        });
        assert_eq!(result, None);
        assert_eq!(read_log.into_inner(), vec![Mime::new(PASSWORD_HINT)]);
    }

    /// An ordinary offer with no hint at all reads its chosen content —
    /// and nothing else — exactly once.
    #[test]
    fn an_ordinary_offer_reads_only_its_chosen_type() {
        let offer = mimes(&["text/html", "text/plain"]);
        let read_log = RefCell::new(Vec::new());
        let result = resolve_offer(&offer, |mime| {
            read_log.borrow_mut().push(mime.clone());
            Some(b"hello".to_vec())
        });
        assert_eq!(result, Some(Content::Text("hello".to_string())));
        assert_eq!(read_log.into_inner(), vec![Mime::new("text/plain")]);
    }

    /// An offer with no usable type is skipped outright — never stored
    /// as an empty entry, and never read at all.
    #[test]
    fn an_offer_with_no_usable_type_is_skipped_and_never_read() {
        let offer = mimes(&["text/html", "application/x-something-weird"]);
        let read_log = RefCell::new(Vec::new());
        let result = resolve_offer(&offer, |mime| {
            read_log.borrow_mut().push(mime.clone());
            Some(b"whatever".to_vec())
        });
        assert_eq!(result, None);
        assert!(read_log.into_inner().is_empty());
    }

    /// A chosen type that reads back as empty (or whitespace-only) is
    /// skipped rather than kept as a blank row in the history.
    #[test]
    fn a_blank_read_is_skipped_rather_than_stored() {
        let offer = mimes(&["text/plain"]);
        let result = resolve_offer(&offer, |_| Some(b"   \n\t".to_vec()));
        assert_eq!(result, None);
    }

    /// The hint being present is not itself the signal — only its value
    /// is, per `Sensitivity`. A benign value still records normally.
    #[test]
    fn a_hint_present_with_a_benign_value_still_records() {
        let offer = mimes(&["text/plain", PASSWORD_HINT]);
        let result = resolve_offer(&offer, |mime| {
            if mime.as_str() == PASSWORD_HINT {
                Some(b"not-secret".to_vec())
            } else {
                Some(b"hello".to_vec())
            }
        });
        assert_eq!(result, Some(Content::Text("hello".to_string())));
    }

    #[test]
    fn an_image_offer_is_kept_as_image_content() {
        let offer = mimes(&["text/plain", "image/png"]);
        let result = resolve_offer(&offer, |mime| {
            if mime.is_image() {
                Some(vec![1, 2, 3])
            } else {
                Some(b"text".to_vec())
            }
        });
        assert_eq!(
            result,
            Some(Content::Image {
                bytes: vec![1, 2, 3],
                mime: Mime::new("image/png")
            })
        );
    }
}
