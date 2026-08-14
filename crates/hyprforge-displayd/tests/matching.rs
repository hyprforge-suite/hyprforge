use hyprforge_displayd::backend::mock::MockBackend;
use hyprforge_displayd::backend::OutputBackend;
use hyprforge_displayd::daemon::{Daemon, DaemonPaths, DaemonSignal, REVERT_WINDOW};
use hyprforge_displayd::types::Identity;
use std::sync::Arc;
use std::time::Duration;

/// Yields enough times for spawned work to be polled.
///
/// Needed on both sides of a `tokio::time::advance`: *before*, so the revert
/// timer's task actually runs and registers its `sleep` (a freshly spawned
/// task has no timer for `advance` to fire yet), and *after*, so the woken
/// task runs through to its effects.
async fn settle() {
    for _ in 0..32 {
        tokio::task::yield_now().await;
    }
}

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
        let backend = Arc::new(MockBackend::new());
        // Every path under the temp dir, `monitors.lua` included. Settling a
        // topology regenerates that file, and pointing it at the real config
        // meant a test run rewrote the developer's own display setup with
        // mock outputs.
        let daemon = Arc::new(
            Daemon::with_paths(
                backend.clone() as Arc<dyn OutputBackend>,
                DaemonPaths::mock_under(dir.path()),
            )
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

    fn storage_path(&self) -> std::path::PathBuf {
        self._dir.path().join("display-profiles.toml")
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
async fn set_head_geometry_position_round_trips_through_get_profile() {
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
            500,
            250,
            1920,
            1080,
            60000,
            1.0,
            hyprforge_displayd::types::Transform::Normal,
        )
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
async fn set_head_geometry_rejects_unknown_connector() {
    let mut h = Harness::new();
    let boe = identity("BOE", "0x0BC9", "");

    h.backend.set_topology(vec![boe]);
    tokio::time::advance(Duration::from_millis(50)).await;
    h.next_signal().await;

    let id = h.daemon.profiles().await[0].id.clone();
    let result = h
        .daemon
        .set_head_geometry(
            &id,
            "NOT-A-HEAD",
            0,
            0,
            1920,
            1080,
            60000,
            1.0,
            hyprforge_displayd::types::Transform::Normal,
        )
        .await;
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

#[tokio::test(start_paused = true)]
async fn delete_profile_removes_it_and_persists_the_removal() {
    let mut h = Harness::new();
    let boe = identity("BOE", "0x0BC9", "");
    let dell = identity("DELL", "U2720Q", "ABC");

    // Learn two profiles so there's something left after the delete — a
    // delete that empties the list can't distinguish "removed the right one"
    // from "removed everything". The two sets must be disjoint, not nested:
    // [boe, dell] would superset-match the [boe] profile and extend it
    // rather than learning a second one.
    h.backend.set_topology(vec![boe]);
    tokio::time::advance(Duration::from_millis(50)).await;
    h.next_signal().await;
    h.backend.set_topology(vec![dell]);
    tokio::time::advance(Duration::from_millis(50)).await;
    h.next_signal().await;

    let profiles = h.daemon.profiles().await;
    assert_eq!(profiles.len(), 2);
    let doomed = profiles[0].id.clone();
    let survivor = profiles[1].id.clone();

    h.daemon.delete_profile(&doomed).await.unwrap();

    let remaining = h.daemon.profiles().await;
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].id, survivor);

    // The removal must have reached disk, not just the in-memory list.
    let reloaded = hyprforge_displayd::storage::load(&h.storage_path()).unwrap();
    assert_eq!(reloaded.len(), 1);
    assert_eq!(reloaded[0].id, survivor);
}

#[tokio::test(start_paused = true)]
async fn delete_profile_rejects_an_unknown_id() {
    let h = Harness::new();
    assert!(h.daemon.delete_profile("no-such-profile").await.is_err());
}

#[tokio::test(start_paused = true)]
async fn deleting_the_connected_topology_is_relearned_on_the_next_settle() {
    // Documents the caveat the GUI's confirm dialog warns about: a profile
    // for what's currently plugged in doesn't stay deleted, because "no
    // stored profile matches" is exactly the auto-learn trigger.
    let mut h = Harness::new();
    let boe = identity("BOE", "0x0BC9", "");

    h.backend.set_topology(vec![boe.clone()]);
    tokio::time::advance(Duration::from_millis(50)).await;
    h.next_signal().await;

    let id = h.daemon.profiles().await[0].id.clone();
    h.daemon.delete_profile(&id).await.unwrap();
    assert!(h.daemon.profiles().await.is_empty());

    // Any subsequent topology settle re-learns it.
    h.backend.set_topology(vec![]);
    tokio::time::advance(Duration::from_millis(50)).await;
    h.backend.set_topology(vec![boe]);
    tokio::time::advance(Duration::from_millis(50)).await;
    h.next_signal().await;

    assert_eq!(h.daemon.profiles().await.len(), 1);
}

// --- reversible layout changes ----------------------------------------

/// Drives the GUI's "Save & Apply" shape: edit geometry (which persists
/// immediately), then apply reversibly. The snapshot must come from the
/// *first* mutation, not the apply, or there's nothing good to go back to.
async fn edit_and_apply_reversibly(h: &Harness, id: &str) {
    h.daemon
        .set_head_geometry(
            id,
            "MOCK-1",
            500,
            600,
            1920,
            1080,
            60000,
            2.0,
            hyprforge_displayd::types::Transform::Rotate180,
        )
        .await
        .unwrap();
    h.daemon.apply_profile_reversible(id).await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn unconfirmed_layout_change_reverts_when_the_window_lapses() {
    let mut h = Harness::new();
    h.backend.set_topology(vec![identity("BOE", "0x0BC9", "")]);
    tokio::time::advance(Duration::from_millis(50)).await;
    h.next_signal().await;

    let id = h.daemon.profiles().await[0].id.clone();
    let before = h.daemon.profiles().await[0].heads[0].clone();

    edit_and_apply_reversibly(&h, &id).await;
    assert_eq!(h.daemon.profiles().await[0].heads[0].x, 500);

    // Say nothing and let the countdown run out.
    settle().await;
    tokio::time::advance(REVERT_WINDOW + Duration::from_secs(1)).await;
    settle().await;

    let after = h.daemon.profiles().await[0].heads[0].clone();
    assert_eq!(
        after, before,
        "an unconfirmed change must restore the stored profile too, or the \
         next hotplug would re-apply the layout the user just escaped"
    );

    let outputs = h.backend.list_outputs().unwrap();
    assert_eq!(outputs[0].position, (0, 0));
    assert_eq!(outputs[0].scale, 1.0);
}

#[tokio::test(start_paused = true)]
async fn confirmed_layout_change_sticks() {
    let mut h = Harness::new();
    h.backend.set_topology(vec![identity("BOE", "0x0BC9", "")]);
    tokio::time::advance(Duration::from_millis(50)).await;
    h.next_signal().await;

    let id = h.daemon.profiles().await[0].id.clone();
    edit_and_apply_reversibly(&h, &id).await;
    h.daemon.confirm_layout().await.unwrap();

    // Well past the deadline: the expired timer must find nothing to undo.
    settle().await;
    tokio::time::advance(REVERT_WINDOW * 3).await;
    settle().await;

    let head = h.daemon.profiles().await[0].heads[0].clone();
    assert_eq!((head.x, head.y), (500, 600));
    assert_eq!(head.scale, 2.0);
    assert_eq!(h.backend.list_outputs().unwrap()[0].position, (500, 600));
}

#[tokio::test(start_paused = true)]
async fn explicit_revert_rolls_back_without_waiting() {
    let mut h = Harness::new();
    h.backend.set_topology(vec![identity("BOE", "0x0BC9", "")]);
    tokio::time::advance(Duration::from_millis(50)).await;
    h.next_signal().await;

    let id = h.daemon.profiles().await[0].id.clone();
    let before = h.daemon.profiles().await[0].heads[0].clone();

    edit_and_apply_reversibly(&h, &id).await;
    h.daemon.revert_layout().await.unwrap();

    assert_eq!(h.daemon.profiles().await[0].heads[0], before);
    // Nothing is pending any more, so a second revert is an error rather
    // than a silent no-op that rolls back an unrelated later change.
    assert!(h.daemon.revert_layout().await.is_err());
}

#[tokio::test(start_paused = true)]
async fn confirming_when_nothing_is_pending_is_a_no_op() {
    let h = Harness::new();
    assert!(h.daemon.confirm_layout().await.is_ok());
}

#[tokio::test(start_paused = true)]
async fn a_lapsed_timer_cannot_roll_back_a_later_unrelated_change() {
    let mut h = Harness::new();
    h.backend.set_topology(vec![identity("BOE", "0x0BC9", "")]);
    tokio::time::advance(Duration::from_millis(50)).await;
    h.next_signal().await;

    let id = h.daemon.profiles().await[0].id.clone();

    // First edit, confirmed.
    edit_and_apply_reversibly(&h, &id).await;
    h.daemon.confirm_layout().await.unwrap();

    // Second, different edit — armed while the first timer is still asleep.
    h.daemon
        .set_head_geometry(
            &id,
            "MOCK-1",
            10,
            20,
            1920,
            1080,
            60000,
            1.0,
            hyprforge_displayd::types::Transform::Normal,
        )
        .await
        .unwrap();
    h.daemon.apply_profile_reversible(&id).await.unwrap();
    h.daemon.confirm_layout().await.unwrap();

    settle().await;
    tokio::time::advance(REVERT_WINDOW * 3).await;
    settle().await;

    // The first timer fired somewhere in there; generation matching must
    // have kept it from undoing the second, confirmed change.
    let head = h.daemon.profiles().await[0].heads[0].clone();
    assert_eq!((head.x, head.y), (10, 20));
}

/// The daemon must not re-apply its own echo.
///
/// Every commit makes the compositor emit a fresh `done` event, which
/// arrives back here as a snapshot that now exact-matches the profile just
/// applied. Committing unconditionally at that point re-commits, which
/// echoes again, forever — observed live against Hyprland 0.56.1 as one
/// apply every debounce interval until the daemon was killed. Nothing is
/// plugged or unplugged after the forced apply below, so a count that keeps
/// climbing means the loop is back.
#[tokio::test(start_paused = true)]
async fn an_applied_layout_does_not_retrigger_itself() {
    let mut h = Harness::new();
    let boe = identity("BOE", "0x0BC9", "");

    h.backend.set_topology(vec![boe]);
    tokio::time::advance(Duration::from_millis(50)).await;
    h.next_signal().await; // learned

    // Move the stored geometry away from what's live, so there is a real
    // change to commit — reconnecting an already-matching topology commits
    // nothing at all, which would test the wrong thing.
    let id = h.daemon.profiles().await[0].id.clone();
    h.daemon
        .set_head_geometry(
            &id,
            "MOCK-1",
            100,
            50,
            1920,
            1080,
            60000,
            1.0,
            hyprforge_displayd::types::Transform::Normal,
        )
        .await
        .unwrap();
    h.daemon.apply_profile(&id).await.unwrap();

    settle().await;
    tokio::time::advance(Duration::from_millis(50)).await;
    settle().await;

    let after_apply = h.backend.apply_count();
    assert!(
        after_apply >= 1,
        "expected the forced apply to commit the new geometry at least once"
    );

    // Let many debounce windows pass without touching the topology.
    for _ in 0..20 {
        settle().await;
        tokio::time::advance(Duration::from_millis(50)).await;
        settle().await;
    }

    assert_eq!(
        h.backend.apply_count(),
        after_apply,
        "daemon re-applied a layout that was already in effect — it is \
         feeding its own `done` events back into the apply path"
    );
}

/// The quantization guard behind the loop fix.
///
/// A scale is carried as a `wl_fixed` (1/256ths), so a profile storing 1.6
/// reads back from the compositor as 1.6015625. If those compare unequal
/// the layout never looks satisfied and the re-apply loop returns for every
/// fractional scale, which is the common case on a HiDPI laptop panel.
#[tokio::test(start_paused = true)]
async fn a_wl_fixed_rounded_scale_still_counts_as_satisfied() {
    use hyprforge_displayd::types::{HeadPlan, LayoutPlan, ModeSpec, Transform};

    let live = hyprforge_displayd::types::Head {
        connector: "eDP-2".to_string(),
        identity: identity("BOE", "0x0BC9", ""),
        description: "BOE 0x0BC9".to_string(),
        modes: vec![hyprforge_displayd::types::Mode {
            width: 2560,
            height: 1600,
            refresh_mhz: 165000,
            preferred: true,
        }],
        current_mode: Some(hyprforge_displayd::types::Mode {
            width: 2560,
            height: 1600,
            refresh_mhz: 165000,
            preferred: true,
        }),
        position: (0, 0),
        transform: Transform::Normal,
        // What the compositor reports back after being asked for 1.6.
        scale: 1.6015625,
        enabled: true,
    };
    let plan = LayoutPlan {
        heads: vec![HeadPlan {
            connector: "eDP-2".to_string(),
            enabled: true,
            mode: Some(ModeSpec::Exact {
                width: 2560,
                height: 1600,
                refresh_mhz: 165000,
            }),
            position: (0, 0),
            transform: Transform::Normal,
            scale: 1.6,
        }],
    };

    assert!(plan.is_satisfied_by(std::slice::from_ref(&live)));

    // A genuinely different scale must still register as needing an apply.
    let mut different = plan.clone();
    different.heads[0].scale = 2.0;
    assert!(!different.is_satisfied_by(std::slice::from_ref(&live)));
}

/// A deliberate geometry change must survive being applied.
///
/// A commit does not stop snapshots that were already in flight. When one of
/// those pre-commit snapshots is what the debounce loop settles on, it
/// diverges from the profile just applied — and auto-learn reads divergence
/// as the user having rearranged their displays externally, writing the old
/// geometry back over the new one. Observed live as a 1920x1200 apply that
/// snapped back to 2560x1600 and left the stored profile reset, so a mode
/// change looked like it did nothing at all. The cooldown the apply arms is
/// what tells our own echo apart from a real external change.
#[tokio::test(start_paused = true)]
async fn applying_an_edited_geometry_is_not_auto_learned_away() {
    let mut h = Harness::new();
    let boe = identity("BOE", "0x0BC9", "");

    h.backend.set_topology(vec![boe]);
    tokio::time::advance(Duration::from_millis(50)).await;
    h.next_signal().await; // learned at the mock's default 1920x1080

    // The layout as it stands before the edit — this is what a snapshot
    // already in flight when the commit lands would carry.
    let stale = h.backend.list_outputs().unwrap();

    let id = h.daemon.profiles().await[0].id.clone();
    h.daemon
        .set_head_geometry(
            &id,
            "MOCK-1",
            0,
            0,
            1280,
            720,
            60000,
            1.0,
            hyprforge_displayd::types::Transform::Normal,
        )
        .await
        .unwrap();
    h.daemon.apply_profile(&id).await.unwrap();

    // Deliver the stale, pre-commit snapshot *after* the commit.
    h.backend.push_snapshot(stale);

    for _ in 0..5 {
        settle().await;
        tokio::time::advance(Duration::from_millis(50)).await;
        settle().await;
    }

    let head = h.daemon.profiles().await[0].heads[0].clone();
    assert_eq!(
        (head.width, head.height),
        (1280, 720),
        "the applied geometry was auto-learned away and replaced with the \
         pre-apply layout"
    );
}

/// A commit the compositor won't honour must not be retried forever.
///
/// `apply_configuration` succeeds once the request is sent, but the result
/// can come back different — an output that can't run its advertised
/// preferred mode settles on another one. The plan is then never satisfied,
/// so every settle commits it again. Seen live when a second monitor
/// advertising 2560x1440@180 came up at 60: one apply every debounce
/// interval for as long as the daemon ran.
#[tokio::test(start_paused = true)]
async fn a_layout_the_compositor_refuses_is_not_retried_forever() {
    use hyprforge_displayd::types::{Head, Mode, Transform};

    let mut h = Harness::new();
    let boe = identity("BOE", "0x0BC9", "");
    let arzopa = identity("GWD", "ARZOPA", "2022110200001");

    h.backend.set_topology(vec![boe.clone()]);
    tokio::time::advance(Duration::from_millis(50)).await;
    h.next_signal().await; // learned the single-panel profile

    let preferred = Mode {
        width: 2560,
        height: 1440,
        refresh_mhz: 180000,
        preferred: true,
    };
    let actual = Mode {
        width: 2560,
        height: 1440,
        refresh_mhz: 60000,
        preferred: false,
    };
    // Second display advertises 180Hz as preferred but is stuck at 60 — so
    // the extend-right plan's `Preferred` can never be satisfied.
    let mut heads = h.backend.list_outputs().unwrap();
    heads.push(Head {
        connector: "DP-3".to_string(),
        identity: arzopa,
        description: "GWD ARZOPA".to_string(),
        modes: vec![preferred, actual],
        current_mode: Some(actual),
        position: (4096, 0),
        transform: Transform::Normal,
        scale: 1.0,
        enabled: true,
    });
    h.backend.lock_mode("DP-3");
    h.backend.push_snapshot(heads);

    // Let things run until the daemon has done whatever it is going to do.
    // The count is deliberately not compared against a fixed number: a
    // hotplug legitimately produces more than one plan (the matched profile,
    // then the newly learned one for the connected set). What must hold is
    // that it *stops* — so measure a quiet period, then another, and require
    // them to agree.
    for _ in 0..20 {
        settle().await;
        tokio::time::advance(Duration::from_millis(50)).await;
        settle().await;
    }
    let settled = h.backend.apply_count();

    for _ in 0..20 {
        settle().await;
        tokio::time::advance(Duration::from_millis(50)).await;
        settle().await;
    }

    assert_eq!(
        h.backend.apply_count(),
        settled,
        "daemon kept re-committing a layout the compositor won't accept"
    );
}

/// The daemon must be able to name the profile in effect for a superset
/// match, not just an exact one.
///
/// A profile only shares its id with the fingerprint when the match is
/// exact, so a client that compares the two sees "nothing matches" the
/// moment a display is added to a known setup. The Settings GUI did exactly
/// that and reported "hasn't matched a saved profile" while the daemon had
/// already matched and applied one.
#[tokio::test(start_paused = true)]
async fn current_profile_is_reported_for_a_superset_match() {
    let mut h = Harness::new();
    let boe = identity("BOE", "0x0BC9", "");
    let arzopa = identity("GWD", "ARZOPA", "2022110200001");

    h.backend.set_topology(vec![boe.clone()]);
    tokio::time::advance(Duration::from_millis(50)).await;
    h.next_signal().await;
    let learned = h.daemon.profiles().await[0].id.clone();

    assert_eq!(
        h.daemon.current_profile_id().await.unwrap(),
        Some(learned.clone()),
        "exact match should report the profile"
    );

    // Add a second, never-seen display: still a superset match on the same
    // profile, but the fingerprint no longer equals its id.
    h.backend.set_topology(vec![boe, arzopa]);
    tokio::time::advance(Duration::from_millis(50)).await;
    settle().await;

    assert_ne!(
        h.daemon.current_fingerprint().unwrap(),
        learned,
        "test is meaningless if the fingerprint still matches the id"
    );
    assert_eq!(
        h.daemon.current_profile_id().await.unwrap(),
        Some(learned),
        "superset match should still name the governing profile"
    );
}

/// Plugging a new display into a known setup must end up remembered.
///
/// A superset match covers only some of what's connected, and auto-learn is
/// exact-only, so the extra display was never written to any profile. Since
/// the Settings module edits stored profiles, the user's second monitor was
/// invisible in the UI — permanently, because nothing would ever adopt it.
#[tokio::test(start_paused = true)]
async fn a_display_added_to_a_known_setup_gets_learned() {
    let mut h = Harness::new();
    let boe = identity("BOE", "0x0BC9", "");
    let arzopa = identity("GWD", "ARZOPA", "2022110200001");

    h.backend.set_topology(vec![boe.clone()]);
    tokio::time::advance(Duration::from_millis(50)).await;
    h.next_signal().await;
    let laptop_only = h.daemon.profiles().await[0].id.clone();

    // Plug in a second, previously-unseen display: superset match.
    h.backend.set_topology(vec![boe, arzopa]);
    for _ in 0..6 {
        tokio::time::advance(Duration::from_millis(50)).await;
        settle().await;
    }

    let profiles = h.daemon.profiles().await;
    assert_eq!(
        profiles.len(),
        2,
        "expected the two-display setup to be learned alongside the laptop-only one"
    );

    let fp = h.daemon.current_fingerprint().unwrap();
    let learned = profiles
        .iter()
        .find(|p| p.id == fp)
        .expect("no profile for the connected set");
    assert_eq!(
        learned.heads.len(),
        2,
        "the learned profile must cover both displays, or the second stays \
         invisible in the editor"
    );

    // The laptop-only profile is still there for when the external isn't.
    assert!(profiles.iter().any(|p| p.id == laptop_only));

    // And it is now the profile in effect.
    assert_eq!(h.daemon.current_profile_id().await.unwrap(), Some(fp));
}

/// Learning the connected set must not depend on the layout being
/// achievable.
///
/// A profile can ask for something the compositor won't do — a scale whose
/// logical size isn't a whole number of pixels (2560 at 1.75 wants 1462.86)
/// is the easy way in. The layout then never matches the plan. Gating the
/// learn on that meant one such profile blocked it forever: the user's
/// second display stayed invisible no matter how many times they replugged
/// or restarted, because the only path to it required a match that could
/// never happen.
#[tokio::test(start_paused = true)]
async fn the_connected_set_is_learned_even_when_the_layout_never_matches() {
    let mut h = Harness::new();
    let boe = identity("BOE", "0x0BC9", "");
    let arzopa = identity("GWD", "ARZOPA", "2022110200001");

    h.backend.set_topology(vec![boe.clone()]);
    tokio::time::advance(Duration::from_millis(50)).await;
    h.next_signal().await;

    // MOCK-2 will refuse every mode it's given, so no plan covering it can
    // ever be satisfied.
    h.backend.lock_mode("MOCK-2");
    h.backend.set_topology(vec![boe, arzopa]);
    for _ in 0..8 {
        tokio::time::advance(Duration::from_millis(50)).await;
        settle().await;
    }

    let profiles = h.daemon.profiles().await;
    let fp = h.daemon.current_fingerprint().unwrap();
    assert!(
        profiles.iter().any(|p| p.id == fp && p.heads.len() == 2),
        "the connected set was never learned, so the second display has no \
         profile to appear in — profiles: {:?}",
        profiles.iter().map(|p| (&p.name, p.heads.len())).collect::<Vec<_>>()
    );
}
