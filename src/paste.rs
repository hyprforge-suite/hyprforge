//! Synthesising a paste (Ctrl+V) after the clipboard has been set.
//!
//! Setting the clipboard is only half of "act like Windows": the popup's
//! job is to land the selected entry in whatever application had focus,
//! and only a synthesized keypress can do that. This is a distinct
//! capability from [`crate::write::ClipboardWriter`] on purpose — a
//! compositor that has no `zwp_virtual_keyboard_manager_v1` can still
//! have a perfectly good `ext-data-control-v1`, and setting the
//! clipboard must succeed regardless of whether the keypress can follow
//! it. See [`crate::wayland::WaylandPaster`] for the real implementation
//! and why it can never fail to *construct*, only to *act*.

/// Whether a synthesized paste actually happened.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PasteOutcome {
    /// Ctrl+V was sent to the compositor.
    Sent,
    /// No virtual keyboard was available (or sending failed), so nothing
    /// was pressed. The clipboard is still set; the caller (the popup)
    /// is expected to tell the user to press Ctrl+V themselves.
    Unavailable,
}

/// Presses and releases Ctrl+V in whatever surface currently has
/// keyboard focus.
///
/// Never returns an error: a compositor that does not support this is
/// not a failure of the paste popup, only of this one optional step, so
/// the trait itself has no way to fail — only to report
/// [`PasteOutcome::Unavailable`].
pub trait PasteSynthesizer: Send + Sync {
    fn paste(&self) -> PasteOutcome;
}

#[cfg(any(test, feature = "mock"))]
pub mod mock {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Reports a fixed outcome and counts calls — for the popup's own
    /// tests, standing in for a compositor with or without the protocol.
    pub struct MockPaster {
        outcome: PasteOutcome,
        calls: AtomicUsize,
    }

    impl MockPaster {
        pub fn available() -> Self {
            MockPaster {
                outcome: PasteOutcome::Sent,
                calls: AtomicUsize::new(0),
            }
        }

        /// Stands in for a compositor with no virtual-keyboard protocol.
        pub fn unavailable() -> Self {
            MockPaster {
                outcome: PasteOutcome::Unavailable,
                calls: AtomicUsize::new(0),
            }
        }

        pub fn call_count(&self) -> usize {
            self.calls.load(Ordering::SeqCst)
        }
    }

    impl PasteSynthesizer for MockPaster {
        fn paste(&self) -> PasteOutcome {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.outcome
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn an_unavailable_mock_reports_unavailable_and_still_counts_the_call() {
            let paster = MockPaster::unavailable();
            assert_eq!(paster.paste(), PasteOutcome::Unavailable);
            assert_eq!(paster.call_count(), 1);
        }

        #[test]
        fn an_available_mock_reports_sent() {
            let paster = MockPaster::available();
            assert_eq!(paster.paste(), PasteOutcome::Sent);
        }
    }
}
