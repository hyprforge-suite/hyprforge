//! The boundary between clipboard-history logic and the compositor.
//!
//! Same shape as `hyprforge-bluetooth::backend` and
//! `hyprforge-displayd::backend::OutputBackend`: the trait and its mock
//! exist before the protocol code is trusted, because the machine
//! running tier 1 has no guaranteed compositor, let alone one speaking
//! `ext-data-control-v1`. Everything above this trait — the store, a
//! popup, a daemon — is testable without either.

use crate::types::Entry;
use tokio::sync::mpsc::UnboundedReceiver;

/// Watches the compositor's clipboard and reports each new copy worth
/// keeping.
pub trait ClipboardWatcher: Send + Sync {
    /// A stream of new entries, one per subscriber.
    ///
    /// Every item already passed [`crate::types::Sensitivity::classify`]
    /// as `Recordable` — nothing secret is ever read to find out, so
    /// nothing secret can ever arrive on this channel. A dropped
    /// receiver is simply a subscriber that stopped listening; it is not
    /// an error and does not affect other subscribers.
    fn subscribe(&self) -> UnboundedReceiver<Entry>;
}

#[cfg(any(test, feature = "mock"))]
pub mod mock {
    use super::*;
    use std::sync::Mutex;
    use tokio::sync::mpsc::{self, UnboundedSender};

    /// Publishes entries a test hands it, with no compositor involved —
    /// for the daemon's and the popup's own tests.
    #[derive(Default)]
    pub struct MockWatcher {
        subscribers: Mutex<Vec<UnboundedSender<Entry>>>,
    }

    impl MockWatcher {
        pub fn new() -> Self {
            Self::default()
        }

        /// Simulates a copy: every current subscriber receives `entry`.
        /// A caller that wants to simulate a secret copy simply does not
        /// call this — the real watcher never emits one either.
        pub fn push(&self, entry: Entry) {
            let mut subscribers = self.subscribers.lock().unwrap_or_else(|e| e.into_inner());
            subscribers.retain(|tx| tx.send(entry.clone()).is_ok());
        }

        /// How many live subscribers there are right now. Lets a test
        /// assert a popup unsubscribed (dropped its receiver) when it
        /// closed, rather than leaking a channel per open.
        pub fn subscriber_count(&self) -> usize {
            self.subscribers
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .len()
        }
    }

    impl ClipboardWatcher for MockWatcher {
        fn subscribe(&self) -> UnboundedReceiver<Entry> {
            let (tx, rx) = mpsc::unbounded_channel();
            self.subscribers
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .push(tx);
            rx
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use crate::types::{Content, EntryId};

        fn entry(text: &str) -> Entry {
            let content = Content::Text(text.to_string());
            Entry {
                id: EntryId::of(&content),
                content,
                copied_at: 0,
                pinned: false,
            }
        }

        #[tokio::test]
        async fn a_pushed_entry_reaches_every_current_subscriber() {
            let watcher = MockWatcher::new();
            let mut a = watcher.subscribe();
            let mut b = watcher.subscribe();
            watcher.push(entry("hello"));
            assert_eq!(
                a.recv().await.unwrap().content,
                Content::Text("hello".to_string())
            );
            assert_eq!(
                b.recv().await.unwrap().content,
                Content::Text("hello".to_string())
            );
        }

        #[tokio::test]
        async fn dropping_a_receiver_removes_it_as_a_subscriber() {
            let watcher = MockWatcher::new();
            let rx = watcher.subscribe();
            assert_eq!(watcher.subscriber_count(), 1);
            drop(rx);
            watcher.push(entry("hello"));
            assert_eq!(watcher.subscriber_count(), 0);
        }
    }
}
