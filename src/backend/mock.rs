use super::OutputBackend;
use crate::types::{Head, Identity, LayoutPlan, Mode, ModeSpec, Transform, TopologyEvent};
use std::sync::Mutex;
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};

struct MockState {
    heads: Vec<Head>,
    subscribers: Vec<UnboundedSender<TopologyEvent>>,
    /// Connectors whose mode a commit cannot change.
    mode_locked: std::collections::HashSet<String>,
}

/// Simulates a `wlr-output-management-v1` compositor entirely in memory:
/// connect/disconnect arbitrary fake outputs with configurable identities,
/// driven by a test harness or the `simulate-topology` CLI command. Every
/// integration test in `tests/matching.rs` exercises the real matching
/// path against this, not just unit-level pure functions.
pub struct MockBackend {
    state: Mutex<MockState>,
    applies: std::sync::atomic::AtomicUsize,
    /// Installed by [`MockBackend::hold_applies`] to make
    /// `apply_configuration` block, the way the real one does for up to
    /// 18s. Without a way to reproduce that, nothing could test what the
    /// rest of the daemon is able to do *while* a commit is in flight.
    hold: Mutex<Option<Hold>>,
}

/// The two halves of a held apply: a ping when one is entered, and the
/// release that lets it finish.
struct Hold {
    entered: std::sync::mpsc::Sender<()>,
    release: std::sync::mpsc::Receiver<()>,
}

impl Default for MockBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl MockBackend {
    /// Makes the *next* `apply_configuration` block until the returned
    /// sender is used, and pings the returned receiver once it is inside.
    ///
    /// The real backend blocks for up to 18s per commit — three attempts at
    /// 3s to test plus 3s to apply — and what the daemon can still do
    /// during that window is a property worth testing rather than assuming.
    pub fn hold_applies(&self) -> (std::sync::mpsc::Receiver<()>, std::sync::mpsc::Sender<()>) {
        let (entered_tx, entered_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        *self.hold.lock().unwrap() = Some(Hold { entered: entered_tx, release: release_rx });
        (entered_rx, release_tx)
    }

    pub fn new() -> Self {
        MockBackend {
            state: Mutex::new(MockState {
                heads: Vec::new(),
                subscribers: Vec::new(),
                mode_locked: std::collections::HashSet::new(),
            }),
            applies: std::sync::atomic::AtomicUsize::new(0),
            hold: Mutex::new(None),
        }
    }

    /// How many times `apply_configuration` has been called.
    ///
    /// Exists so tests can assert the daemon *stops* applying. A count that
    /// keeps climbing while nothing is plugged or unplugged is the signature
    /// of the daemon re-applying its own echo.
    pub fn apply_count(&self) -> usize {
        self.applies.load(std::sync::atomic::Ordering::SeqCst)
    }

    /// Replaces the connected set with synthetic heads for `identities`,
    /// one per identity, connector-named `MOCK-1`, `MOCK-2`, ... in order.
    /// Each gets a single 1920x1080@60Hz preferred mode and is enabled,
    /// laid out left-to-right — a plausible "the compositor did something
    /// reasonable on its own" starting point, mirroring how a real
    /// compositor behaves before Hyprforge has ever seen this topology.
    pub fn set_topology(&self, identities: Vec<Identity>) {
        let mut x = 0;
        let heads: Vec<Head> = identities
            .into_iter()
            .enumerate()
            .map(|(i, identity)| {
                let mode = Mode {
                    width: 1920,
                    height: 1080,
                    refresh_mhz: 60000,
                    preferred: true,
                };
                let head = Head {
                    connector: format!("MOCK-{}", i + 1),
                    identity,
                    description: String::new(),
                    modes: vec![mode],
                    current_mode: Some(mode),
                    position: (x, 0),
                    transform: Transform::Normal,
                    scale: 1.0,
                    enabled: true,
                };
                x += mode.width;
                head
            })
            .collect();
        self.push_snapshot(heads);
    }

    /// Makes `connector` keep whatever mode it currently has, no matter what
    /// is committed to it.
    ///
    /// Real hardware does this: an output that can't actually run its
    /// advertised preferred mode (bandwidth, cable, adapter) comes back at a
    /// different one, and the commit still reports success. The daemon has to
    /// notice it asked for something it didn't get and stop asking.
    pub fn lock_mode(&self, connector: &str) {
        self.state
            .lock()
            .unwrap()
            .mode_locked
            .insert(connector.to_string());
    }

    /// Directly pushes a full head snapshot, for tests that need specific
    /// geometry or connector names rather than the `set_topology` default.
    pub fn push_snapshot(&self, heads: Vec<Head>) {
        let mut state = self.state.lock().unwrap();
        state.heads = heads.clone();
        state
            .subscribers
            .retain(|tx| tx.send(TopologyEvent::Snapshot(heads.clone())).is_ok());
    }
}

impl OutputBackend for MockBackend {
    fn list_outputs(&self) -> anyhow::Result<Vec<Head>> {
        Ok(self.state.lock().unwrap().heads.clone())
    }

    fn apply_configuration(&self, plan: &LayoutPlan) -> anyhow::Result<()> {
        plan.validate()?;
        // Blocks here, before touching any state, so a test can observe the
        // daemon mid-commit. The lock is released first: holding it would
        // make this a test of the mock's own mutex rather than the daemon's.
        let held = self.hold.lock().unwrap().take();
        if let Some(hold) = held {
            let _ = hold.entered.send(());
            let _ = hold.release.recv();
        }
        self.applies
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let mut state = self.state.lock().unwrap();
        let locked = state.mode_locked.clone();
        for head_plan in &plan.heads {
            if let Some(head) = state
                .heads
                .iter_mut()
                .find(|h| h.connector == head_plan.connector)
            {
                head.enabled = head_plan.enabled;
                head.position = head_plan.position;
                head.transform = head_plan.transform;
                head.scale = head_plan.scale;
                if let Some(spec) = head_plan.mode {
                    if !locked.contains(&head.connector) {
                        head.current_mode = Some(resolve_mode(head, spec));
                    }
                }
            }
        }
        // Echo unconditionally, *including* when the commit changed nothing.
        // This used to return early on an unchanged state, on the assumption
        // that a real compositor only signals properties that actually
        // changed. Hyprland 0.56.1 does not: it emits a fresh `done` for
        // every commit, so suppressing the no-op echo here hid a live
        // re-apply loop from every test in the suite. The daemon is what has
        // to break that cycle (`LayoutPlan::is_satisfied_by`), so the mock
        // reproduces the compositor's actual behaviour and lets it.
        let heads = state.heads.clone();
        state
            .subscribers
            .retain(|tx| tx.send(TopologyEvent::Snapshot(heads.clone())).is_ok());
        Ok(())
    }

    fn subscribe(&self) -> UnboundedReceiver<TopologyEvent> {
        let (tx, rx) = mpsc::unbounded_channel();
        let mut state = self.state.lock().unwrap();
        // New subscribers immediately see the current state, mirroring a
        // real compositor's "all current heads on bind" behavior.
        let _ = tx.send(TopologyEvent::Snapshot(state.heads.clone()));
        state.subscribers.push(tx);
        rx
    }
}

fn resolve_mode(head: &Head, spec: ModeSpec) -> Mode {
    match spec {
        ModeSpec::Preferred => head.preferred_mode().unwrap_or(Mode {
            width: 1920,
            height: 1080,
            refresh_mhz: 60000,
            preferred: true,
        }),
        ModeSpec::Exact {
            width,
            height,
            refresh_mhz,
        } => head
            .modes
            .iter()
            .find(|m| m.width == width && m.height == height && m.refresh_mhz == refresh_mhz)
            .copied()
            .unwrap_or(Mode {
                width,
                height,
                refresh_mhz,
                preferred: false,
            }),
    }
}
