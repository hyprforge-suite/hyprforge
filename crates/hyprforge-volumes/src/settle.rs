//! Waiting for a burst of "something changed" to finish.
//!
//! Plugging in one USB stick makes UDisks2 announce a new drive, a new
//! block device for the disk, one per partition, and then a property
//! change on each as udev's probing fills in labels and filesystem types
//! — dozens of signals over most of a second. Re-reading the whole list
//! on each would be the video player's mistake that CLAUDE.md records:
//! work per signal, falling behind a sender that signals faster than the
//! work. So a signal only means *look again*, and [`settle`] turns a
//! burst of them into one look, after the burst goes quiet.

use std::time::Duration;
use tokio::sync::mpsc;
use tokio::time::{timeout, Instant};

/// How long the signals must stop for before a burst counts as over.
pub const QUIET: Duration = Duration::from_millis(300);

/// The most a burst may delay a look, however long it goes on — a
/// drive that never stops announcing must not freeze the sidebar.
pub const CEILING: Duration = Duration::from_secs(2);

/// Waits for a signal, then for the burst it starts to go `quiet`, or
/// for `ceiling` to pass since the first one. `false` when the sender
/// has gone and no more will ever come.
pub async fn settle(rx: &mut mpsc::Receiver<()>, quiet: Duration, ceiling: Duration) -> bool {
    if rx.recv().await.is_none() {
        return false;
    }
    let started = Instant::now();
    loop {
        let left = ceiling.saturating_sub(started.elapsed());
        if left.is_zero() {
            return true;
        }
        match timeout(quiet.min(left), rx.recv()).await {
            // Quiet for long enough: the burst is over.
            Err(_) => return true,
            // The sender went away mid-burst: what arrived is still
            // worth one look.
            Ok(None) => return true,
            Ok(Some(())) => continue,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(start_paused = true)]
    async fn a_burst_of_signals_is_one_look_after_it_goes_quiet() {
        let (tx, mut rx) = mpsc::channel(1);
        let sender = tokio::spawn(async move {
            for _ in 0..20 {
                let _ = tx.try_send(());
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            // Held open past the burst, so the end is quiet, not closed.
            tokio::time::sleep(Duration::from_secs(10)).await;
            drop(tx);
        });
        let start = Instant::now();
        assert!(settle(&mut rx, QUIET, CEILING).await);
        let waited = start.elapsed();
        // Twenty signals 50ms apart end at about 950ms; one look comes
        // a quiet period later.
        assert!(waited >= Duration::from_millis(950) + QUIET - Duration::from_millis(60), "{waited:?}");
        assert!(waited < Duration::from_millis(1400), "{waited:?}");
        sender.abort();
    }

    #[tokio::test(start_paused = true)]
    async fn a_burst_that_never_ends_still_gets_looked_at() {
        let (tx, mut rx) = mpsc::channel(1);
        let sender = tokio::spawn(async move {
            loop {
                let _ = tx.try_send(());
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        });
        let start = Instant::now();
        assert!(settle(&mut rx, QUIET, CEILING).await);
        assert!(start.elapsed() <= CEILING + Duration::from_millis(1), "{:?}", start.elapsed());
        sender.abort();
    }

    #[tokio::test]
    async fn a_closed_channel_means_no_more_looks() {
        let (tx, mut rx) = mpsc::channel::<()>(1);
        drop(tx);
        assert!(!settle(&mut rx, QUIET, CEILING).await);
    }
}
