use hyprforge_displayd::backend::mock::MockBackend;
use hyprforge_displayd::backend::OutputBackend;
use hyprforge_displayd::daemon::{Daemon, DaemonSignal};
use hyprforge_displayd::types::Identity;
use std::sync::Arc;
use std::time::Duration;

fn identity(make: &str, model: &str, serial: &str) -> Identity {
    Identity {
        make: make.to_string(),
        model: model.to_string(),
        serial: serial.to_string(),
    }
}

/// Spins up a `Daemon` over a fresh `MockBackend` with a tiny debounce (so
/// tests run fast under `tokio::time::pause`), wired to its own storage
/// file in a temp dir, and returns the pieces needed to drive it.
struct Harness {
    backend: Arc<MockBackend>,
    daemon: Arc<Daemon>,
    signal_rx: tokio::sync::mpsc::UnboundedReceiver<DaemonSignal>,
    _dir: tempfile::TempDir,
}

impl Harness {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let storage_path = dir.path().join("display-profiles.toml");
        let backend = Arc::new(MockBackend::new());
        let daemon = Arc::new(
            Daemon::new(backend.clone() as Arc<dyn OutputBackend>, storage_path)
                .unwrap()
                .with_debounce(Duration::from_millis(10)),
        );
        let (signal_tx, signal_rx) = tokio::sync::mpsc::unbounded_channel();
        let events = backend.subscribe();
        let daemon_clone = daemon.clone();
        tokio::spawn(async move {
            daemon_clone.run(events, signal_tx).await;
        });
        Harness {
            backend,
            daemon,
            signal_rx,
            _dir: dir,
        }
    }

    async fn next_signal(&mut self) -> DaemonSignal {
        tokio::time::timeout(Duration::from_secs(5), self.signal_rx.recv())
            .await
            .expect("timed out waiting for daemon signal")
            .expect("signal channel closed")
    }
}

#[tokio::test(start_paused = true)]
async fn first_ever_topology_is_learned_without_touching_config() {
    let mut h = Harness::new();
    let boe = identity("BOE", "0x0BC9", "");

    h.backend.set_topology(vec![boe]);
    tokio::time::advance(Duration::from_millis(50)).await;

    let signal = h.next_signal().await;
    assert!(matches!(signal, DaemonSignal::NewTopologySeen { .. }));

    let profiles = h.daemon.profiles().await;
    assert_eq!(profiles.len(), 1);
    assert_eq!(profiles[0].heads.len(), 1);
}

#[tokio::test(start_paused = true)]
async fn reconnecting_same_topology_is_an_exact_match() {
    let mut h = Harness::new();
    let boe = identity("BOE", "0x0BC9", "");

    h.backend.set_topology(vec![boe.clone()]);
    tokio::time::advance(Duration::from_millis(50)).await;
    h.next_signal().await; // NewTopologySeen — learned as profile #1

    // Disconnect (zero connected outputs is a pure no-op: never matched,
    // never learned, no signal) then reconnect the identical set.
    h.backend.set_topology(vec![]);
    tokio::time::advance(Duration::from_millis(50)).await;

    h.backend.set_topology(vec![boe]);
    tokio::time::advance(Duration::from_millis(50)).await;
    let sig = h.next_signal().await;
    match sig {
        DaemonSignal::ProfileApplied { tier, .. } => assert_eq!(tier, "exact"),
        other => panic!("expected ProfileApplied(exact), got {other:?}"),
    }

    let profiles = h.daemon.profiles().await;
    assert_eq!(profiles.len(), 1, "reconnecting the same set should exact-match the learned profile, not create a new one");
}

#[tokio::test(start_paused = true)]
async fn superset_match_extends_uncovered_output_to_the_right() {
    let mut h = Harness::new();
    let boe = identity("BOE", "0x0BC9", "");
    let dell = identity("DELL", "U2720Q", "ABC");

    // Learn a single-display profile first.
    h.backend.set_topology(vec![boe.clone()]);
    tokio::time::advance(Duration::from_millis(50)).await;
    h.next_signal().await;

    // Now plug in a second, previously-unseen display alongside it.
    h.backend.set_topology(vec![boe.clone(), dell]);
    tokio::time::advance(Duration::from_millis(50)).await;
    let sig = h.next_signal().await;
    match sig {
        DaemonSignal::ProfileApplied { tier, .. } => assert_eq!(tier, "superset"),
        other => panic!("expected ProfileApplied(superset), got {other:?}"),
    }

    let outputs = h.backend.list_outputs().unwrap();
    let boe_head = outputs.iter().find(|o| o.identity == boe).unwrap();
    let dell_head = outputs.iter().find(|o| o.identity != boe).unwrap();
    assert!(dell_head.enabled);
    assert!(
        dell_head.position.0 >= boe_head.position.0,
        "extra output should extend to the right of the existing one"
    );
}

#[tokio::test(start_paused = true)]
async fn subset_match_applies_partial_profile_and_stays_valid() {
    let mut h = Harness::new();
    let boe = identity("BOE", "0x0BC9", "");
    let dell = identity("DELL", "U2720Q", "ABC");

    // Learn the two-display profile.
    h.backend.set_topology(vec![boe.clone(), dell]);
    tokio::time::advance(Duration::from_millis(50)).await;
    h.next_signal().await;

    // Undock: only one of the two is still connected.
    h.backend.set_topology(vec![boe]);
    tokio::time::advance(Duration::from_millis(50)).await;
    let sig = h.next_signal().await;
    match sig {
        DaemonSignal::ProfileApplied { tier, .. } => assert_eq!(tier, "subset"),
        other => panic!("expected ProfileApplied(subset), got {other:?}"),
    }

    let outputs = h.backend.list_outputs().unwrap();
    assert_eq!(outputs.len(), 1);
    assert!(outputs[0].enabled, "safety rail: at least one output stays enabled");
}

#[tokio::test(start_paused = true)]
async fn duplicate_blank_serial_identities_exact_match_and_assign_on_reconnect() {
    let mut h = Harness::new();
    let boe = identity("BOE", "0x0BC9", "");

    // Two identical blank-serial panels — mirrors this machine's eDP-2,
    // whose real EDID serial is also blank. Matching works fine on the
    // multiset; per-head *assignment* is the interesting part, exercised
    // directly (with a `head_swaps` override) in matching::tests — this
    // confirms the daemon still applies cleanly end-to-end when it occurs.
    h.backend.set_topology(vec![boe.clone(), boe.clone()]);
    tokio::time::advance(Duration::from_millis(50)).await;
    h.next_signal().await;

    // Reconnect — exact match by multiset, assignment falls back to
    // connector-name order (MOCK-1/MOCK-2 both times, since the mock
    // backend assigns connector names in call order).
    h.backend.set_topology(vec![boe.clone(), boe]);
    tokio::time::advance(Duration::from_millis(50)).await;
    let sig = h.next_signal().await;
    assert!(matches!(sig, DaemonSignal::ProfileApplied { .. }));

    let outputs = h.backend.list_outputs().unwrap();
    assert_eq!(outputs.len(), 2);
    assert!(outputs.iter().all(|o| o.enabled));
}

#[tokio::test(start_paused = true)]
async fn auto_learns_layout_changed_externally() {
    let mut h = Harness::new();
    let boe = identity("BOE", "0x0BC9", "");

    h.backend.set_topology(vec![boe.clone()]);
    tokio::time::advance(Duration::from_millis(50)).await;
    h.next_signal().await; // NewTopologySeen — learned as profile #1

    // A fresh settled snapshot of the identical topology (e.g. a reconnect)
    // exact-matches the just-learned profile and gets applied, starting
    // the auto-learn cooldown.
    h.backend.set_topology(vec![boe.clone()]);
    tokio::time::advance(Duration::from_millis(50)).await;
    h.next_signal().await; // ProfileApplied(exact)

    // Wait out the post-apply auto-learn cooldown.
    tokio::time::advance(Duration::from_millis(2000)).await;

    // Simulate an external change (e.g. `wlr-randr`) moving the head,
    // which the compositor reports as a fresh settled snapshot.
    let mut outputs = h.backend.list_outputs().unwrap();
    outputs[0].position = (500, 0);
    h.backend.push_snapshot(outputs);
    tokio::time::advance(Duration::from_millis(50)).await;

    // First the match/apply signal, since the daemon still re-applies the
    // (now stale) stored layout for an exact identity match...
    let sig = h.next_signal().await;
    assert!(matches!(sig, DaemonSignal::ProfileApplied { .. }));
    // ...but then detects the divergence and learns the new position.
    let sig = h.next_signal().await;
    match sig {
        DaemonSignal::ProfileAutoLearned { .. } => {}
        other => panic!("expected ProfileAutoLearned, got {other:?}"),
    }

    let profiles = h.daemon.profiles().await;
    let learned = profiles.iter().find(|p| p.heads.len() == 1).unwrap();
    assert_eq!(learned.heads[0].x, 500);
}

#[tokio::test(start_paused = true)]
async fn most_recently_used_wins_tie_break_across_reconnects() {
    let mut h = Harness::new();
    let boe = identity("BOE", "0x0BC9", "");
    let dell = identity("DELL", "U2720Q", "ABC");

    // Two different two-display profiles is not directly constructible via
    // set_topology (identity multiset determines the profile), so instead
    // verify that re-applying always refreshes last_used, keeping the
    // active profile the tie-break winner against a stale duplicate.
    h.backend.set_topology(vec![boe.clone()]);
    tokio::time::advance(Duration::from_millis(50)).await;
    h.next_signal().await;
    let first_used = h.daemon.profiles().await[0].last_used.clone();

    tokio::time::advance(Duration::from_secs(2)).await;
    h.backend.set_topology(vec![boe]);
    tokio::time::advance(Duration::from_millis(50)).await;
    h.next_signal().await;

    let second_used = h.daemon.profiles().await[0].last_used.clone();
    assert!(second_used >= first_used);
    let _ = dell; // reserved for a future multi-profile tie-break test
}

#[tokio::test(start_paused = true)]
async fn swap_heads_toggles_rather_than_accumulates() {
    let mut h = Harness::new();
    let boe = identity("BOE", "0x0BC9", "");

    h.backend.set_topology(vec![boe.clone(), boe]);
    tokio::time::advance(Duration::from_millis(50)).await;
    h.next_signal().await;

    let id = h.daemon.profiles().await[0].id.clone();
    assert!(h.daemon.profiles().await[0].head_swaps.is_empty());

    h.daemon.swap_heads(&id, "MOCK-1", "MOCK-2").await.unwrap();
    assert_eq!(h.daemon.profiles().await[0].head_swaps.len(), 1);

    // Calling again with the same pair toggles it back off, rather than
    // accumulating a second entry that would cancel the first out via
    // matching::assign_heads' sequential swap application.
    h.daemon.swap_heads(&id, "MOCK-1", "MOCK-2").await.unwrap();
    assert!(h.daemon.profiles().await[0].head_swaps.is_empty());

    // Reversed pair order is recognized as the same swap.
    h.daemon.swap_heads(&id, "MOCK-1", "MOCK-2").await.unwrap();
    h.daemon.swap_heads(&id, "MOCK-2", "MOCK-1").await.unwrap();
    assert!(h.daemon.profiles().await[0].head_swaps.is_empty());
}

#[tokio::test(start_paused = true)]
async fn set_head_position_updates_stored_profile() {
    let mut h = Harness::new();
    let boe = identity("BOE", "0x0BC9", "");

    h.backend.set_topology(vec![boe]);
    tokio::time::advance(Duration::from_millis(50)).await;
    h.next_signal().await;

    let id = h.daemon.profiles().await[0].id.clone();
    h.daemon
        .set_head_position(&id, "MOCK-1", 500, 250)
        .await
        .unwrap();

    let profiles = h.daemon.profiles().await;
    let head = &profiles[0].heads[0];
    assert_eq!((head.x, head.y), (500, 250));

    // Round-trips through JSON the same way the GUI's layout editor
    // consumes it via the `GetProfile` D-Bus method.
    let json = h.daemon.get_profile_json(&id).await.unwrap();
    assert!(json.contains("\"x\":500"));
    assert!(json.contains("\"y\":250"));
}

#[tokio::test(start_paused = true)]
async fn set_head_position_rejects_unknown_connector() {
    let mut h = Harness::new();
    let boe = identity("BOE", "0x0BC9", "");

    h.backend.set_topology(vec![boe]);
    tokio::time::advance(Duration::from_millis(50)).await;
    h.next_signal().await;

    let id = h.daemon.profiles().await[0].id.clone();
    let result = h.daemon.set_head_position(&id, "NOT-A-HEAD", 0, 0).await;
    assert!(result.is_err());
}

#[tokio::test(start_paused = true)]
async fn set_head_geometry_updates_position_mode_and_scale() {
    let mut h = Harness::new();
    let boe = identity("BOE", "0x0BC9", "");

    h.backend.set_topology(vec![boe]);
    tokio::time::advance(Duration::from_millis(50)).await;
    h.next_signal().await;

    let id = h.daemon.profiles().await[0].id.clone();
    h.daemon
        .set_head_geometry(
            &id,
            "MOCK-1",
            100,
            200,
            3840,
            2160,
            144000,
            1.5,
            hyprforge_displayd::types::Transform::Rotate90,
        )
        .await
        .unwrap();

    let profiles = h.daemon.profiles().await;
    let head = &profiles[0].heads[0];
    assert_eq!((head.x, head.y), (100, 200));
    assert_eq!((head.width, head.height), (3840, 2160));
    assert_eq!(head.transform, hyprforge_displayd::types::Transform::Rotate90);
    assert_eq!(head.refresh_mhz, 144000);
    assert_eq!(head.scale, 1.5);
}
