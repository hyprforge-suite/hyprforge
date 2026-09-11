use crate::backend::OutputBackend;
use crate::fingerprint::fingerprint;
use crate::matching::{build_layout_plan, find_match, MatchTier};
use crate::profile::{generate_profile_name, ExtraOutputPolicy, Profile};
use crate::types::{Head, HeadPlan, LayoutPlan, ModeSpec, TopologyEvent, Transform};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, Mutex};

/// Emitted for the D-Bus service (and tests) to observe what the daemon
/// just did, mirroring the `ProfileApplied` / `NewTopologySeen` D-Bus
/// signals 1:1.
#[derive(Debug, Clone, PartialEq)]
pub enum DaemonSignal {
    ProfileApplied {
        id: String,
        name: String,
        tier: String,
    },
    NewTopologySeen {
        fingerprint: String,
        summary: String,
    },
    /// Emitted whenever a matched profile's stored layout is overwritten
    /// because the live layout diverged from it (vision pillar #6:
    /// auto-learn, no explicit "save profile" step).
    ProfileAutoLearned {
        id: String,
        name: String,
    },
    /// A reversible layout change was just applied and is provisional for
    /// `seconds` more. The GUI draws its countdown from this.
    RevertPending {
        seconds: u32,
    },
    /// A provisional change stopped being provisional: either the user kept
    /// it (`reverted: false`) or it was rolled back, by the timer expiring
    /// or an explicit request (`reverted: true`).
    RevertResolved {
        reverted: bool,
    },
}

/// How long after the daemon's own `apply_configuration` call to suppress
/// auto-learn drift detection. Real compositors often echo the just-applied
/// config back through a fresh `done` event with minor rounding (e.g.
/// fractional scale), which would otherwise look like external drift and
/// immediately overwrite the profile we just applied.
const AUTO_LEARN_COOLDOWN: Duration = Duration::from_millis(1500);

/// Default quiet period after the last add/remove before acting on a new
/// topology.
pub const DEFAULT_DEBOUNCE: Duration = Duration::from_millis(500);

/// How long a reversible layout change stays provisional before rolling
/// itself back. Long enough to read a banner and click, short enough that a
/// blanked screen recovers on its own before the user reaches for a TTY.
pub const REVERT_WINDOW: Duration = Duration::from_secs(12);

/// How long an *unarmed* snapshot stays usable.
///
/// A snapshot is taken by the first mutation of an edit and armed by the
/// apply that follows, which is milliseconds later. But several paths
/// mutate without ever applying — `displayctl apply` is deliberately
/// irreversible, and the GUI can save geometry, swap heads or change a
/// policy without applying anything. Those leave a snapshot behind that
/// nothing will ever arm or clear.
///
/// Without a lifetime that snapshot lives as long as the daemon, and the
/// *next* reversible apply arms it — so letting the window lapse would
/// restore a profile list captured hours ago, discarding every edit and
/// every auto-learned profile made in between. An expiry bounds that to
/// an interval in which "put it back how it was" still means something.
pub const SNAPSHOT_FRESHNESS: Duration = Duration::from_secs(120);

/// The state captured before a layout edit, so it can be put back.
///
/// Both halves are needed: restoring only the compositor would leave the bad
/// geometry sitting in the profile store, where the next hotplug would
/// re-apply it — the change would come back on its own after being "undone".
struct PendingRevert {
    /// Live heads as they were before the edit.
    heads: Vec<Head>,
    /// The whole profile list as it was before the edit.
    profiles: Vec<Profile>,
    /// Distinguishes this revert from a later one that replaced it, so a
    /// stale timer can't roll back a change it doesn't own.
    generation: u64,
    /// False until a timer is actually running (the snapshot is taken at the
    /// first mutation, which may be several D-Bus calls before the apply).
    armed: bool,
    /// When it was captured, so an unarmed one can expire. See
    /// [`SNAPSHOT_FRESHNESS`].
    taken_at: tokio::time::Instant,
}

impl PendingRevert {
    /// Whether this snapshot is too old to be worth restoring.
    ///
    /// Only ever true of an unarmed snapshot: once a timer is running the
    /// snapshot is owned by that timer, and age is no longer what decides
    /// its fate.
    fn is_stale(&self) -> bool {
        !self.armed && self.taken_at.elapsed() > SNAPSHOT_FRESHNESS
    }
}

pub struct Daemon {
    backend: Arc<dyn OutputBackend>,
    storage_path: PathBuf,
    /// Where the Hyprland-native fallback is written. Held rather than
    /// looked up at write time so a mock run can be pointed somewhere
    /// harmless — see [`DaemonPaths`].
    monitors_lua_path: PathBuf,
    greet_monitors_path: PathBuf,
    profiles: Mutex<Vec<Profile>>,
    competing_monitor_rules: Mutex<Vec<String>>,
    suppress_learn_until: Mutex<Option<tokio::time::Instant>>,
    /// The last plan that was committed and then *still* wasn't reflected in
    /// the layout that came back. Stops the daemon retrying a configuration
    /// the compositor won't accept — see [`Daemon::handle_settled_topology`].
    unsatisfied_plan: Mutex<Option<LayoutPlan>>,
    pending_revert: Mutex<Option<PendingRevert>>,
    revert_generation: std::sync::atomic::AtomicU64,
    /// Set once [`Daemon::run`] starts, so work spawned outside the run loop
    /// (the revert timer) can still emit signals.
    signal_tx: Mutex<Option<mpsc::UnboundedSender<DaemonSignal>>>,
    debounce: Duration,
}

impl Daemon {
    /// Every path the daemon will write is named by the caller.
    ///
    /// There is deliberately no convenience constructor that fills in the
    /// real locations for you. The previous one took only `storage_path` and
    /// quietly defaulted `monitors.lua` to the real config — so the
    /// integration tests, which carefully used a temp dir for profiles,
    /// wrote mock outputs straight into the developer's own Hyprland config
    /// every time they settled a topology. Making the caller say where
    /// everything goes is what stops that being possible.
    pub fn with_paths(backend: Arc<dyn OutputBackend>, paths: DaemonPaths) -> anyhow::Result<Self> {
        let DaemonPaths { storage_path, monitors_lua_path, greet_monitors_path } = paths;
        let profiles = crate::storage::load(&storage_path)?;
        Ok(Daemon {
            backend,
            storage_path,
            monitors_lua_path,
            greet_monitors_path,
            profiles: Mutex::new(profiles),
            competing_monitor_rules: Mutex::new(Vec::new()),
            suppress_learn_until: Mutex::new(None),
            unsatisfied_plan: Mutex::new(None),
            pending_revert: Mutex::new(None),
            revert_generation: std::sync::atomic::AtomicU64::new(0),
            signal_tx: Mutex::new(None),
            debounce: DEFAULT_DEBOUNCE,
        })
    }

    /// Overrides the default debounce window — used by tests (to run fast
    /// under `tokio::time::pause`) and available to the CLI for tuning.
    pub fn with_debounce(mut self, debounce: Duration) -> Self {
        self.debounce = debounce;
        self
    }

    pub async fn profiles(&self) -> Vec<Profile> {
        self.profiles.lock().await.clone()
    }

    pub async fn set_competing_monitor_rules(&self, rules: Vec<String>) {
        *self.competing_monitor_rules.lock().await = rules;
    }

    pub async fn competing_monitor_rules(&self) -> Vec<String> {
        self.competing_monitor_rules.lock().await.clone()
    }

    pub fn current_fingerprint(&self) -> anyhow::Result<String> {
        let heads = self.backend.list_outputs()?;
        let identities: Vec<_> = heads.iter().map(|h| h.identity.clone()).collect();
        Ok(fingerprint(&identities))
    }

    /// The profile that currently governs the connected outputs, if any.
    ///
    /// This is the daemon's own match, so it covers superset and subset
    /// matches — which a caller cannot work out for itself, since a profile
    /// only shares its id with the fingerprint when the match is *exact*.
    /// Without this, a client starting up after the daemon has already
    /// settled has no way to learn which profile is in effect short of
    /// waiting for the next `ProfileApplied` signal, which may never come.
    pub async fn current_profile_id(&self) -> anyhow::Result<Option<String>> {
        let heads = self.backend.list_outputs()?;
        if heads.is_empty() {
            return Ok(None);
        }
        let connected: Vec<_> = heads.iter().map(|h| h.identity.clone()).collect();
        let profiles = self.profiles.lock().await;
        Ok(find_match(&profiles, &connected).map(|m| m.profile.id.clone()))
    }

    /// JSON-encoded snapshot of the currently-applied layout, for the
    /// `GetCurrentLayout` D-Bus method.
    pub fn current_layout_json(&self) -> anyhow::Result<String> {
        let heads = self.backend.list_outputs()?;
        Ok(serde_json::to_string(&heads)?)
    }

    /// The modes a *currently-connected* head advertises — `(width,
    /// height, refresh_mhz, preferred)` — for the Displays module's
    /// resolution dropdown. Stored profiles only ever remember the one
    /// configured mode, not the full list a head supports, so this reads
    /// live backend state; returns empty if `connector_hint` isn't
    /// currently connected (e.g. editing a profile for a different,
    /// not-plugged-in setup), which callers should treat as "fall back to
    /// showing just the stored mode."
    pub fn available_modes(&self, connector_hint: &str) -> anyhow::Result<Vec<(i32, i32, i32, bool)>> {
        let heads = self.backend.list_outputs()?;
        Ok(heads
            .iter()
            .find(|h| h.connector == connector_hint)
            .map(|h| {
                h.modes
                    .iter()
                    .map(|m| (m.width, m.height, m.refresh_mhz, m.preferred))
                    .collect()
            })
            .unwrap_or_default())
    }

    // --- reversible layout changes (vision pillar #4) ---------------------

    async fn emit(&self, signal: DaemonSignal) {
        if let Some(tx) = self.signal_tx.lock().await.as_ref() {
            let _ = tx.send(signal);
        }
    }

    /// Records the pre-edit state, if nothing is pending already.
    ///
    /// Called by every profile-mutating method while it holds the profiles
    /// lock. The first mutation of an edit wins: the GUI's "Save & Apply"
    /// issues several `SetHeadGeometry` calls and *then* `ApplyProfile`, so
    /// snapshotting at apply time would capture the already-edited profile
    /// and revert to nothing.
    async fn ensure_revert_snapshot(&self, profiles: &[Profile]) {
        let mut pending = self.pending_revert.lock().await;
        // The first mutation of an edit wins — unless what is held is a
        // leftover from an edit that was never applied, in which case
        // "first" would mean an arbitrarily old state.
        match pending.as_ref() {
            Some(held) if !held.is_stale() => return,
            Some(_) => {
                tracing::debug!(
                    "discarding an unapplied revert snapshot older than {}s",
                    SNAPSHOT_FRESHNESS.as_secs()
                );
            }
            None => {}
        }
        let Ok(heads) = self.backend.list_outputs() else {
            // No readable live state means nothing trustworthy to restore;
            // better to leave the change irreversible than to "revert" into
            // a layout we made up.
            tracing::warn!("could not snapshot live outputs; this change won't be reversible");
            return;
        };
        *pending = Some(PendingRevert {
            heads,
            profiles: profiles.to_vec(),
            generation: self
                .revert_generation
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst)
                + 1,
            armed: false,
            taken_at: tokio::time::Instant::now(),
        });
    }

    /// Starts the countdown on an already-captured snapshot, and returns how
    /// long the caller has to confirm. Idempotent within one edit: a second
    /// apply before the first resolves keeps the original deadline and the
    /// original (pre-edit) snapshot.
    pub async fn arm_revert(self: &Arc<Self>) -> Option<Duration> {
        let generation = {
            let mut pending = self.pending_revert.lock().await;
            // Never arm a stale snapshot. Arming one is the failure this
            // guards: the countdown would look completely normal and roll
            // back to a state nobody recognises. Dropping it makes the
            // change irreversible instead, which is the same contract
            // `ensure_revert_snapshot` already gives when live outputs
            // cannot be read.
            if pending.as_ref().is_some_and(PendingRevert::is_stale) {
                tracing::warn!(
                    "the pending revert snapshot is older than {}s and was never applied; \
                     this change won't be reversible",
                    SNAPSHOT_FRESHNESS.as_secs()
                );
                *pending = None;
            }
            let p = pending.as_mut()?;
            if p.armed {
                return Some(REVERT_WINDOW);
            }
            p.armed = true;
            p.generation
        };

        self.emit(DaemonSignal::RevertPending {
            seconds: REVERT_WINDOW.as_secs() as u32,
        })
        .await;

        let daemon = self.clone();
        tokio::spawn(async move {
            tokio::time::sleep(REVERT_WINDOW).await;
            daemon.expire_revert(generation).await;
        });
        Some(REVERT_WINDOW)
    }

    /// Keeps the provisional change: drops the snapshot so the timer, when
    /// it fires, finds nothing of its generation to roll back.
    pub async fn confirm_layout(&self) -> anyhow::Result<()> {
        let had_pending = self.pending_revert.lock().await.take().is_some();
        if had_pending {
            self.emit(DaemonSignal::RevertResolved { reverted: false })
                .await;
        }
        Ok(())
    }

    /// Rolls back to the snapshot immediately, without waiting out the
    /// countdown.
    pub async fn revert_layout(&self) -> anyhow::Result<()> {
        let pending = self.pending_revert.lock().await.take();
        let Some(pending) = pending else {
            anyhow::bail!("no layout change is pending confirmation");
        };
        self.restore(pending).await?;
        self.emit(DaemonSignal::RevertResolved { reverted: true })
            .await;
        Ok(())
    }

    /// Timer callback. Does nothing unless the snapshot still in place is
    /// the one this timer was started for.
    async fn expire_revert(&self, generation: u64) {
        let pending = {
            let mut guard = self.pending_revert.lock().await;
            match guard.as_ref() {
                Some(p) if p.generation == generation => guard.take(),
                // Confirmed, reverted, or superseded — not ours to undo.
                _ => None,
            }
        };
        let Some(pending) = pending else {
            return;
        };
        tracing::info!("layout change was not confirmed in time; reverting");
        if let Err(e) = self.restore(pending).await {
            tracing::error!(error = %e, "failed to revert layout");
        }
        self.emit(DaemonSignal::RevertResolved { reverted: true })
            .await;
    }

    /// Puts back both halves of the snapshot: the profile store first (so a
    /// later hotplug re-applies the good layout), then the live compositor
    /// state.
    async fn restore(&self, pending: PendingRevert) -> anyhow::Result<()> {
        {
            let mut profiles = self.profiles.lock().await;
            *profiles = pending.profiles;
            // Propagated, not swallowed: a revert that restores the live
            // layout but not the store is worse than one that fails
            // outright, because the next hotplug re-applies the layout the
            // user just rejected. The automatic-timeout caller logs it; the
            // explicit `RevertLayout` caller returns it to the app.
            self.persist(&profiles)?;
        }

        let plan = plan_from_heads(&pending.heads);
        // A snapshot with nothing enabled would trip the same safety rail as
        // any other plan; there's no good restore to make, so leave the
        // compositor alone rather than fail loudly at it.
        if plan.validate().is_err() {
            tracing::warn!("snapshot had no enabled outputs; restored profiles only");
            return Ok(());
        }
        // The restore is our own change, so don't let its echo look like the
        // user externally rearranging their displays.
        *self.suppress_learn_until.lock().await =
            Some(tokio::time::Instant::now() + AUTO_LEARN_COOLDOWN);
        self.backend.apply_configuration(&plan)
    }

    /// Force-applies `profile_id` regardless of whether it currently
    /// matches the connected fingerprint — used by `apply <profile-id>`
    /// and the D-Bus `ApplyProfile` method.
    pub async fn apply_profile(&self, profile_id: &str) -> anyhow::Result<()> {
        let heads = self.backend.list_outputs()?;
        let mut profiles = self.profiles.lock().await;
        let idx = profiles
            .iter()
            .position(|p| p.id == profile_id)
            .ok_or_else(|| anyhow::anyhow!("no such profile: {profile_id}"))?;

        self.ensure_revert_snapshot(&profiles).await;

        let plan = build_layout_plan(&profiles[idx], &heads);
        plan.validate()?;
        // Same reasoning as `restore`: this commit is our own, so its echo
        // must not read as the user externally rearranging their displays.
        // Without this, the settle that follows compares the *pre-commit*
        // snapshot against the profile it just applied, sees them diverge,
        // and "learns" the old layout straight back over the new one —
        // making a deliberate mode or scale change appear to do nothing.
        *self.suppress_learn_until.lock().await =
            Some(tokio::time::Instant::now() + AUTO_LEARN_COOLDOWN);
        self.backend.apply_configuration(&plan)?;
        profiles[idx].touch();
        self.persist(&profiles)?;
        Ok(())
    }

    pub async fn rename_profile(&self, profile_id: &str, new_name: &str) -> anyhow::Result<()> {
        let mut profiles = self.profiles.lock().await;
        let profile = profiles
            .iter_mut()
            .find(|p| p.id == profile_id)
            .ok_or_else(|| anyhow::anyhow!("no such profile: {profile_id}"))?;
        profile.name = new_name.to_string();
        self.persist(&profiles)?;
        Ok(())
    }

    /// Forgets a stored profile entirely.
    ///
    /// Note this is not permanent for the *currently connected* topology:
    /// auto-learn (vision pillar #6) will re-learn a fresh profile for it on
    /// the next topology settle, since "no stored profile matches" is
    /// precisely the condition that triggers learning. Deleting is therefore
    /// a reset-to-defaults for the current setup, and a true forget only for
    /// topologies that aren't plugged in — callers should say so rather than
    /// promising the entry stays gone.
    pub async fn delete_profile(&self, profile_id: &str) -> anyhow::Result<()> {
        let mut profiles = self.profiles.lock().await;
        let idx = profiles
            .iter()
            .position(|p| p.id == profile_id)
            .ok_or_else(|| anyhow::anyhow!("no such profile: {profile_id}"))?;
        profiles.remove(idx);
        self.persist(&profiles)?;
        Ok(())
    }

    /// Toggles a swap override: whichever connector the given pair of
    /// stored `connector_hint`s would each normally resolve to, resolve to
    /// the other's instead. Used for duplicate/blank-serial identities
    /// where per-head assignment can't be derived from EDID alone.
    ///
    /// Idempotent by pair rather than append-only: calling this again with
    /// the same (or reversed) pair removes the override instead of piling
    /// up a second entry that would cancel the first out — a GUI button
    /// bound to this call behaves as a plain on/off toggle.
    pub async fn swap_heads(
        &self,
        profile_id: &str,
        connector_a: &str,
        connector_b: &str,
    ) -> anyhow::Result<()> {
        let mut profiles = self.profiles.lock().await;
        self.ensure_revert_snapshot(&profiles).await;
        let profile = profiles
            .iter_mut()
            .find(|p| p.id == profile_id)
            .ok_or_else(|| anyhow::anyhow!("no such profile: {profile_id}"))?;
        let matches_pair = |p: &(String, String)| {
            (p.0 == connector_a && p.1 == connector_b) || (p.0 == connector_b && p.1 == connector_a)
        };
        if let Some(pos) = profile.head_swaps.iter().position(matches_pair) {
            profile.head_swaps.remove(pos);
        } else {
            profile
                .head_swaps
                .push((connector_a.to_string(), connector_b.to_string()));
        }
        self.persist(&profiles)?;
        Ok(())
    }

    /// Sets a head's full stored geometry within a profile — position,
    /// mode, scale, and orientation. The single write path for everything
    /// the Displays module edits: both the drag-arrange canvas and the
    /// per-head property panel land here.
    #[allow(clippy::too_many_arguments)]
    pub async fn set_head_geometry(
        &self,
        profile_id: &str,
        connector_hint: &str,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
        refresh_mhz: i32,
        scale: f64,
        transform: Transform,
    ) -> anyhow::Result<()> {
        let mut profiles = self.profiles.lock().await;
        self.ensure_revert_snapshot(&profiles).await;
        let profile = profiles
            .iter_mut()
            .find(|p| p.id == profile_id)
            .ok_or_else(|| anyhow::anyhow!("no such profile: {profile_id}"))?;
        let head = profile
            .heads
            .iter_mut()
            .find(|h| h.connector_hint == connector_hint)
            .ok_or_else(|| anyhow::anyhow!("no such head: {connector_hint}"))?;
        head.x = x;
        head.y = y;
        head.width = width;
        head.height = height;
        head.refresh_mhz = refresh_mhz;
        head.scale = scale;
        head.transform = transform;
        self.persist(&profiles)?;
        Ok(())
    }

    /// JSON-encoded snapshot of one stored profile (full head geometry
    /// included) — for the Displays module's layout editor, which needs
    /// more detail than `ListProfiles`' summary row provides.
    pub async fn get_profile_json(&self, profile_id: &str) -> anyhow::Result<String> {
        let profiles = self.profiles.lock().await;
        let profile = profiles
            .iter()
            .find(|p| p.id == profile_id)
            .ok_or_else(|| anyhow::anyhow!("no such profile: {profile_id}"))?;
        Ok(serde_json::to_string(profile)?)
    }

    pub async fn set_extra_output_policy(
        &self,
        profile_id: &str,
        policy: ExtraOutputPolicy,
    ) -> anyhow::Result<()> {
        let mut profiles = self.profiles.lock().await;
        self.ensure_revert_snapshot(&profiles).await;
        let profile = profiles
            .iter_mut()
            .find(|p| p.id == profile_id)
            .ok_or_else(|| anyhow::anyhow!("no such profile: {profile_id}"))?;
        profile.extra_output_policy = policy;
        self.persist(&profiles)?;
        Ok(())
    }

    /// Runs the debounce + match + apply/learn loop until `events` closes.
    /// `signal_tx` receives one [`DaemonSignal`] per topology settle.
    pub async fn run(
        &self,
        mut events: mpsc::UnboundedReceiver<TopologyEvent>,
        signal_tx: mpsc::UnboundedSender<DaemonSignal>,
    ) {
        // Share the sender with work that outlives a single loop iteration —
        // specifically the revert timer, which is spawned from a D-Bus call
        // rather than from here.
        *self.signal_tx.lock().await = Some(signal_tx.clone());

        loop {
            let Some(TopologyEvent::Snapshot(mut heads)) = events.recv().await else {
                return;
            };

            // Debounce: keep absorbing snapshots that arrive within the
            // window, acting only once things settle.
            loop {
                match tokio::time::timeout(self.debounce, events.recv()).await {
                    Ok(Some(TopologyEvent::Snapshot(newer))) => heads = newer,
                    Ok(None) => return,
                    Err(_timeout) => break,
                }
            }

            self.handle_settled_topology(heads, &signal_tx).await;
        }
    }

    async fn handle_settled_topology(
        &self,
        heads: Vec<Head>,
        signal_tx: &mpsc::UnboundedSender<DaemonSignal>,
    ) {
        if heads.is_empty() {
            // Zero connected outputs is a degenerate, transient state (e.g.
            // mid-hotplug). An empty identity multiset is trivially a
            // sub-multiset of every stored profile, which would otherwise
            // "subset-match" whatever profile was last used and then fail
            // `LayoutPlan::validate` silently. Treat it as a pure no-op:
            // never matches, never learns, never touches configuration.
            tracing::debug!("zero outputs connected; ignoring");
            return;
        }

        let connected: Vec<_> = heads.iter().map(|h| h.identity.clone()).collect();
        let fp = fingerprint(&connected);

        let mut profiles = self.profiles.lock().await;
        let matched = find_match(&profiles, &connected).map(|m| (m.profile.id.clone(), m.tier));

        let Some((profile_id, tier)) = matched else {
            tracing::info!(fingerprint = %fp, "no stored profile matches this topology; not touching output configuration");
            let existing_names: Vec<String> = profiles.iter().map(|p| p.name.clone()).collect();
            let name = generate_profile_name(&heads, &existing_names);
            let profile = Profile::from_heads(fp.clone(), name.clone(), &heads);
            profiles.push(profile);
            // Background path: nobody is waiting on this call, so a
            // failure is logged inside `persist` and the daemon carries on
            // with the layout it has already applied.
            let _ = self.persist(&profiles);
            let _ = signal_tx.send(DaemonSignal::NewTopologySeen {
                fingerprint: fp.clone(),
                summary: format!("{name} (learned)"),
            });
            // Write the fallback here too, not only where a matched
            // layout is confirmed. A newly learned topology is precisely
            // the case with no fallback for it yet, so returning without
            // one leaves `monitors.lua` describing some *other* set of
            // monitors until the next settle happens to take the matched
            // path — and that file is what Hyprland applies when this
            // daemon is not running.
            //
            // These heads are as safe to record as the ones on that
            // path: nothing was applied to reach them, so they are the
            // layout already on screen.
            tokio::spawn(write_monitors_fallback(
                heads.clone(),
                self.monitors_lua_path.clone(),
                self.greet_monitors_path.clone(),
            ));
            return;
        };

        let profile_idx = profiles.iter().position(|p| p.id == profile_id).unwrap();
        let plan = build_layout_plan(&profiles[profile_idx], &heads);

        if let Err(e) = plan.validate() {
            tracing::error!(error = %e, profile = %profile_id, "refusing to apply invalid plan");
            // Nothing was applied, so the live heads are untouched and
            // worth recording — the stored plan is the broken thing, not
            // what is on screen. Without this a profile that can never
            // validate leaves the fallback stale indefinitely.
            tokio::spawn(write_monitors_fallback(
                heads.clone(),
                self.monitors_lua_path.clone(),
                self.greet_monitors_path.clone(),
            ));
            return;
        }

        // Commit only if the compositor isn't already in this state. Every
        // commit produces a `done` event, which settles back here as a fresh
        // snapshot still matching the same profile — so committing
        // unconditionally re-commits on its own echo, forever (see
        // `LayoutPlan::is_satisfied_by`).
        //
        // Everything below still runs: the profile is in effect either way,
        // so it's still touched, persisted and signalled. Those don't feed
        // back into the event loop, so they can't sustain the cycle.
        let already_satisfied = plan.is_satisfied_by(&heads);
        if already_satisfied {
            *self.unsatisfied_plan.lock().await = None;
            // Only once `heads` are confirmed to already match the plan
            // currently in effect — never from a plan we just sent but
            // haven't seen echoed back yet, which the `else` branch below
            // handles. That echo arrives as a later settle, which re-enters
            // here and writes the fallback then.
            //
            // Spawned rather than awaited: it shells out to `hyprctl`, and
            // this settle's own signal emission below must never wait on
            // that round-trip.
            tokio::spawn(write_monitors_fallback(
                heads.clone(),
                self.monitors_lua_path.clone(),
                self.greet_monitors_path.clone(),
            ));
        } else {
            // A commit that doesn't take is the other way to loop forever.
            // `apply_configuration` reports success once the request is sent,
            // but the compositor can decline the result — an output that
            // won't run at its preferred mode comes back at a different one,
            // so the plan is never satisfied and every settle commits it
            // again. Retry a given plan once, then leave it alone until
            // something actually changes.
            let already_tried = self.unsatisfied_plan.lock().await.as_ref() == Some(&plan);
            if already_tried {
                tracing::warn!(
                    profile = %profile_id,
                    "compositor did not accept this layout; not retrying until the topology changes"
                );
                // Record what is actually on screen before giving up.
                // This branch is where a layout the hardware will not do
                // comes to rest — the fractional-scale case documented
                // below is one users really hit — so it is not a
                // transient state, it is the *permanent* one for that
                // profile. Returning without writing left `monitors.lua`
                // describing some other set of monitors for as long as
                // the profile stayed in effect: greeter at the wrong
                // scale, layout lost whenever the daemon is not running.
                //
                // These heads are the same kind as the satisfied branch
                // writes: observed, not requested. The compositor
                // declined the plan, so what is here is what it chose.
                tokio::spawn(write_monitors_fallback(
                    heads.clone(),
                    self.monitors_lua_path.clone(),
                    self.greet_monitors_path.clone(),
                ));
                return;
            }
            if let Err(e) = self.backend.apply_configuration(&plan) {
                tracing::error!(error = %e, profile = %profile_id, "failed to apply layout");
                // The change did not land, so the screen still shows what
                // these heads describe. Same argument as the two branches
                // above: record the working layout rather than leave the
                // fallback pointing at a different set of monitors.
                tokio::spawn(write_monitors_fallback(
                    heads.clone(),
                    self.monitors_lua_path.clone(),
                    self.greet_monitors_path.clone(),
                ));
                return;
            }
            *self.unsatisfied_plan.lock().await = Some(plan.clone());
        }

        // Check the cooldown left by our *previous* apply before
        // overwriting it with a fresh one for this apply. This settle's
        // `heads` reflect what was observed before we just re-applied the
        // stored plan, so — unless that observation itself landed inside
        // the previous apply's cooldown window (i.e. it's an echo of our
        // own prior change) — it's a legitimate point to check for drift.
        let was_within_cooldown = {
            let guard = self.suppress_learn_until.lock().await;
            guard
                .map(|until| tokio::time::Instant::now() < until)
                .unwrap_or(false)
        };
        *self.suppress_learn_until.lock().await =
            Some(tokio::time::Instant::now() + AUTO_LEARN_COOLDOWN);

        profiles[profile_idx].touch();
        let name = profiles[profile_idx].name.clone();
        let tier_str = match tier {
            MatchTier::Exact => "exact",
            MatchTier::Superset => "superset",
            MatchTier::Subset => "subset",
        };
        if already_satisfied {
            tracing::debug!(
                profile_id = %profile_id,
                profile_name = %name,
                "live layout already matches this profile; nothing to commit"
            );
        } else {
            tracing::info!(
                fingerprint = %fp,
                profile_id = %profile_id,
                profile_name = %name,
                tier = tier_str,
                heads = ?plan.heads,
                "applied layout"
            );
        }
        // Background path: nobody is waiting on this call, so a
        // failure is logged inside `persist` and the daemon carries on
        // with the layout it has already applied.
        let _ = self.persist(&profiles);
        let _ = signal_tx.send(DaemonSignal::ProfileApplied {
            id: profile_id.clone(),
            name,
            tier: tier_str.to_string(),
        });

        // Auto-learn: only meaningful for an exact match, where every
        // connected head is covered by the profile, so any live/stored
        // divergence unambiguously means something external changed the
        // layout (manual wlr-randr, a future drag-arrange GUI).
        if tier == MatchTier::Exact && !was_within_cooldown {
            self.maybe_auto_learn(&mut profiles, profile_idx, &heads, signal_tx)
                .await;
        }

        // A superset match means a profile covers *some* of what's plugged
        // in. The uncovered outputs get placed by the extra-output policy,
        // but nothing ever remembers them: auto-learn above is exact-only, so
        // the profile stays as it was and the extra display exists nowhere in
        // the profile store. The Settings module edits the stored profile, so
        // that display is invisible there too — the user plugs in a second
        // monitor and simply cannot see or arrange it.
        //
        // Learn a profile for the topology that's actually connected, from
        // the live heads — which are the truth about what's on screen, even
        // when that isn't what was asked for.
        //
        // Deliberately *not* conditional on the layout matching the plan. A
        // stored profile can request something the compositor won't do — a
        // scale whose logical size isn't a whole number of pixels is the easy
        // way in, 2560 at 1.75 giving 1462.86 — and then no layout ever
        // satisfies it. Gating on satisfaction meant such a profile blocked
        // learning forever, which is precisely the state a user hits after
        // setting a scale that didn't take: a second display that never
        // appears anywhere, with no way to reach it.
        //
        // The original profile is left alone for when this set isn't plugged
        // in, and the new one exact-matches from here on.
        if tier == MatchTier::Superset {
            self.learn_current_topology(&mut profiles, &fp, &heads, signal_tx)
                .await;
        }
    }

    /// Stores a profile describing exactly what's connected now, if there
    /// isn't one already.
    async fn learn_current_topology(
        &self,
        profiles: &mut Vec<Profile>,
        fingerprint: &str,
        heads: &[Head],
        signal_tx: &mpsc::UnboundedSender<DaemonSignal>,
    ) {
        if profiles.iter().any(|p| p.id == fingerprint) {
            return;
        }
        let existing_names: Vec<String> = profiles.iter().map(|p| p.name.clone()).collect();
        let name = generate_profile_name(heads, &existing_names);
        profiles.push(Profile::from_heads(
            fingerprint.to_string(),
            name.clone(),
            heads,
        ));
        tracing::info!(
            fingerprint = %fingerprint,
            profile_name = %name,
            "learned a profile for the connected set, which a stored profile only partly covered"
        );
        // Background path: nobody is waiting on this call, so a
        // failure is logged inside `persist` and the daemon carries on
        // with the layout it has already applied.
        let _ = self.persist(profiles);
        let _ = signal_tx.send(DaemonSignal::NewTopologySeen {
            fingerprint: fingerprint.to_string(),
            summary: format!("{name} (learned)"),
        });
    }

    async fn maybe_auto_learn(
        &self,
        profiles: &mut [Profile],
        profile_idx: usize,
        heads: &[Head],
        signal_tx: &mpsc::UnboundedSender<DaemonSignal>,
    ) {
        let fresh = Profile::from_heads(
            profiles[profile_idx].id.clone(),
            profiles[profile_idx].name.clone(),
            heads,
        );
        let mut fresh_sorted = fresh.heads.clone();
        let mut stored_sorted = profiles[profile_idx].heads.clone();
        let by_hint = |a: &crate::profile::HeadRecord, b: &crate::profile::HeadRecord| {
            a.connector_hint.cmp(&b.connector_hint)
        };
        fresh_sorted.sort_by(by_hint);
        stored_sorted.sort_by(by_hint);

        if fresh_sorted == stored_sorted {
            return;
        }

        profiles[profile_idx].heads = fresh.heads;
        profiles[profile_idx].touch();
        let id = profiles[profile_idx].id.clone();
        let name = profiles[profile_idx].name.clone();
        tracing::info!(profile_id = %id, "auto-learned updated layout from external change");
        // Background path: nobody is waiting on this call, so a
        // failure is logged inside `persist` and the daemon carries on
        // with the layout it has already applied.
        let _ = self.persist(profiles);
        let _ = signal_tx.send(DaemonSignal::ProfileAutoLearned { id, name });
    }

    /// Reversible-apply entry point for the GUI: applies `profile_id`, then
    /// starts the confirm/revert countdown. Returns the number of seconds
    /// the caller has to confirm.
    ///
    /// The plain [`Daemon::apply_profile`] stays immediate and irreversible,
    /// which is what a scripted `displayctl apply` wants — a CLI caller has
    /// no banner to click and shouldn't have their change silently undone.
    pub async fn apply_profile_reversible(self: &Arc<Self>, profile_id: &str) -> anyhow::Result<u32> {
        self.apply_profile(profile_id).await?;
        self.arm_revert().await;
        Ok(REVERT_WINDOW.as_secs() as u32)
    }

    /// Writes the profile store, logging and returning any failure.
    ///
    /// Returning it matters: every user-initiated change goes out over D-Bus
    /// as `Ok(())`, and this used to swallow the error, so a rename or a
    /// geometry edit reported success to the Settings app while nothing
    /// reached the disk — the change then vanished at the next daemon
    /// restart with no explanation anywhere but the daemon's own log
    /// (vision pillar #3: errors surface in the app, never "check the
    /// logs"). Background callers still only log, because there's nobody
    /// waiting on them to tell.
    fn persist(&self, profiles: &[Profile]) -> anyhow::Result<()> {
        crate::storage::save(&self.storage_path, profiles).map_err(|e| {
            tracing::error!(error = %e, "failed to persist display profiles");
            anyhow::anyhow!("couldn't save your display profiles: {e}")
        })
    }
}

/// Best-effort regenerates the static, Hyprland-native fallback
/// (`monitors.lua`) from the live, settled heads — see the
/// `monitors_codegen` module docs. Unlike everything else this daemon
/// does, this file is applied by Hyprland itself on its own
/// startup/reload, so it's what keeps a layout from reverting when the
/// daemon isn't the one running.
///
/// A free function taking owned `heads`, not a `&self` method: it's
/// spawned as a detached background task (it shells out to `hyprctl`, and
/// the settle path that triggers it must never wait on that round-trip),
/// so its future has to be `'static` and touches no daemon state.
async fn write_monitors_fallback(heads: Vec<Head>, path: PathBuf, greet_path: PathBuf) {
    let descriptions = crate::hyprctl_monitors::connector_descriptions().await;
    let entries = crate::monitors_codegen::entries_from_heads(&heads, &descriptions);
    let lua = crate::monitors_codegen::generate(&entries);
    if let Err(e) = hyprforge_core::paths::write_atomic(&path, &lua) {
        tracing::warn!(error = %e, "failed to write the monitors.lua fallback");
    }

    // The same layout again, where the greeter can read it. Not a
    // convenience: the greeter runs as another user and cannot traverse
    // into a home directory, so without this copy it falls back to
    // scale 1 and a login screen on a 1.6-scaled display looks like it
    // picked the wrong resolution.
    //
    // Best-effort and separately reported. The export directory is an
    // install step (see crates/hyprforge-greet/config), so its absence
    // is the normal state on a machine with no greeter and must never
    // colour the write above, which is the load-bearing one.
    if let Err(e) = hyprforge_core::paths::write_atomic(&greet_path, &lua) {
        tracing::warn!(
            error = %e,
            path = %greet_path.display(),
            "couldn't export the display layout for the greeter, so the login screen \
             will use its fallback scale; the session's own monitors.lua is written"
        );
    }
}

/// Every file the daemon owns.
///
/// Held together so there is exactly one place that decides where the
/// daemon writes — and so a mock run can move *all* of it at once. Getting
/// that wrong is not theoretical: a `--mock` run once wrote a `MOCK-1`
/// output into a real user's `monitors.lua`, leaving their actual display
/// absent from the very file that exists to keep its layout when the daemon
/// isn't running.
#[derive(Debug, Clone)]
pub struct DaemonPaths {
    pub storage_path: PathBuf,
    pub monitors_lua_path: PathBuf,
    /// The copy the greeter reads. Outside `$HOME` by necessity, and in
    /// `DaemonPaths` rather than a constant for exactly the reason the
    /// doc comment above gives: a mock run must not reach it either.
    pub greet_monitors_path: PathBuf,
}

impl DaemonPaths {
    /// The real locations, for a real run.
    pub fn real() -> Self {
        DaemonPaths {
            storage_path: hyprforge_core::paths::display_profiles_path(),
            monitors_lua_path: hyprforge_core::paths::monitors_lua_path(),
            greet_monitors_path: hyprforge_look::theme::export_dir().join("monitors.lua"),
        }
    }

    /// Throwaway locations under `dir`, for a mock run. Nothing here may
    /// point at anything Hyprland reads.
    pub fn mock_under(dir: &std::path::Path) -> Self {
        DaemonPaths {
            storage_path: dir.join("display-profiles.toml"),
            monitors_lua_path: dir.join("monitors.lua"),
            greet_monitors_path: dir.join("greet-monitors.lua"),
        }
    }
}

/// Turns a live snapshot back into an applicable plan. Used only to restore
/// a snapshot verbatim, so every field is carried across as observed rather
/// than re-derived from a profile.
fn plan_from_heads(heads: &[Head]) -> LayoutPlan {
    LayoutPlan {
        heads: heads
            .iter()
            .map(|h| HeadPlan {
                connector: h.connector.clone(),
                enabled: h.enabled,
                mode: h.enabled.then(|| {
                    h.current_mode
                        .map(|m| ModeSpec::Exact {
                            width: m.width,
                            height: m.height,
                            refresh_mhz: m.refresh_mhz,
                        })
                        .unwrap_or(ModeSpec::Preferred)
                }),
                position: h.position,
                transform: h.transform,
                scale: h.scale,
            })
            .collect(),
    }
}
