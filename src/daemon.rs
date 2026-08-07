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
}

pub struct Daemon {
    backend: Arc<dyn OutputBackend>,
    storage_path: PathBuf,
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
    pub fn new(backend: Arc<dyn OutputBackend>, storage_path: PathBuf) -> anyhow::Result<Self> {
        let profiles = crate::storage::load(&storage_path)?;
        Ok(Daemon {
            backend,
            storage_path,
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
        if pending.is_some() {
            return;
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
        });
    }

    /// Starts the countdown on an already-captured snapshot, and returns how
    /// long the caller has to confirm. Idempotent within one edit: a second
    /// apply before the first resolves keeps the original deadline and the
    /// original (pre-edit) snapshot.
    pub async fn arm_revert(self: &Arc<Self>) -> Option<Duration> {
        let generation = {
            let mut pending = self.pending_revert.lock().await;
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
            self.persist(&profiles);
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
        self.persist(&profiles);
        Ok(())
    }

    pub async fn rename_profile(&self, profile_id: &str, new_name: &str) -> anyhow::Result<()> {
        let mut profiles = self.profiles.lock().await;
        let profile = profiles
            .iter_mut()
            .find(|p| p.id == profile_id)
            .ok_or_else(|| anyhow::anyhow!("no such profile: {profile_id}"))?;
        profile.name = new_name.to_string();
        self.persist(&profiles);
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
        self.persist(&profiles);
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
        self.persist(&profiles);
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
        self.persist(&profiles);
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
        self.persist(&profiles);
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
            self.persist(&profiles);
            let _ = signal_tx.send(DaemonSignal::NewTopologySeen {
                fingerprint: fp.clone(),
                summary: format!("{name} (learned)"),
            });
            return;
        };

        let profile_idx = profiles.iter().position(|p| p.id == profile_id).unwrap();
        let plan = build_layout_plan(&profiles[profile_idx], &heads);

        if let Err(e) = plan.validate() {
            tracing::error!(error = %e, profile = %profile_id, "refusing to apply invalid plan");
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
                return;
            }
            if let Err(e) = self.backend.apply_configuration(&plan) {
                tracing::error!(error = %e, profile = %profile_id, "failed to apply layout");
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
        self.persist(&profiles);
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
        self.persist(profiles);
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

    fn persist(&self, profiles: &[Profile]) {
        if let Err(e) = crate::storage::save(&self.storage_path, profiles) {
            tracing::error!(error = %e, "failed to persist display profiles");
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
