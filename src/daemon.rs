use crate::backend::OutputBackend;
use crate::fingerprint::fingerprint;
use crate::matching::{build_layout_plan, find_match, MatchTier};
use crate::profile::{generate_profile_name, ExtraOutputPolicy, Profile};
use crate::types::{Head, TopologyEvent};
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

pub struct Daemon {
    backend: Arc<dyn OutputBackend>,
    storage_path: PathBuf,
    profiles: Mutex<Vec<Profile>>,
    competing_monitor_rules: Mutex<Vec<String>>,
    suppress_learn_until: Mutex<Option<tokio::time::Instant>>,
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

        let plan = build_layout_plan(&profiles[idx], &heads);
        plan.validate()?;
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

    /// Persists a swap override: whichever connector the given pair of
    /// stored `connector_hint`s would each normally resolve to, resolve to
    /// the other's instead. Used for duplicate/blank-serial identities
    /// where per-head assignment can't be derived from EDID alone.
    pub async fn swap_heads(
        &self,
        profile_id: &str,
        connector_a: &str,
        connector_b: &str,
    ) -> anyhow::Result<()> {
        let mut profiles = self.profiles.lock().await;
        let profile = profiles
            .iter_mut()
            .find(|p| p.id == profile_id)
            .ok_or_else(|| anyhow::anyhow!("no such profile: {profile_id}"))?;
        profile
            .head_swaps
            .push((connector_a.to_string(), connector_b.to_string()));
        self.persist(&profiles);
        Ok(())
    }

    pub async fn set_extra_output_policy(
        &self,
        profile_id: &str,
        policy: ExtraOutputPolicy,
    ) -> anyhow::Result<()> {
        let mut profiles = self.profiles.lock().await;
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

        if let Err(e) = self.backend.apply_configuration(&plan) {
            tracing::error!(error = %e, profile = %profile_id, "failed to apply layout");
            return;
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
        tracing::info!(
            fingerprint = %fp,
            profile_id = %profile_id,
            profile_name = %name,
            tier = tier_str,
            heads = ?plan.heads,
            "applied layout"
        );
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

    fn persist(&self, profiles: &[Profile]) {
        if let Err(e) = crate::storage::save(&self.storage_path, profiles) {
            tracing::error!(error = %e, "failed to persist display profiles");
        }
    }
}
