use serde::{Deserialize, Serialize};

/// A physical/virtual output's stable identity, from EDID data as exposed
/// via `wlr-output-management-v1`. `make`/`model`/`serial` are all
/// `since="2"` on the wire — absent on older compositors, and frequently
/// blank in practice even when present (this machine's built-in panel
/// reports an empty serial). A blank/duplicate identity is a normal case
/// handled explicitly in `matching::assign_heads`, never a bug to work
/// around.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Identity {
    pub make: String,
    pub model: String,
    pub serial: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Mode {
    pub width: i32,
    pub height: i32,
    /// Vertical refresh rate in mHz, as the protocol reports it.
    pub refresh_mhz: i32,
    pub preferred: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Transform {
    #[default]
    Normal,
    Rotate90,
    Rotate180,
    Rotate270,
    Flipped,
    Flipped90,
    Flipped180,
    Flipped270,
}

/// A connected output at a point in time, as reported by the backend.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Head {
    /// Compositor-assigned connector name (e.g. `eDP-2`, `DP-4`). Stable
    /// within a session but **not** across replug/dock/undock — never used
    /// as a matching key, only as an assignment tiebreaker and the handle
    /// backends use to address the physical output.
    pub connector: String,
    pub identity: Identity,
    pub description: String,
    pub modes: Vec<Mode>,
    pub current_mode: Option<Mode>,
    pub position: (i32, i32),
    pub transform: Transform,
    pub scale: f64,
    pub enabled: bool,
}

impl Head {
    pub fn preferred_mode(&self) -> Option<Mode> {
        self.modes
            .iter()
            .find(|m| m.preferred)
            .or_else(|| self.modes.first())
            .copied()
    }

    /// The mode to treat as "this head's mode" for naming/placement
    /// purposes: current if enabled, else preferred, else the first
    /// available.
    pub fn effective_mode(&self) -> Option<Mode> {
        self.current_mode.or_else(|| self.preferred_mode())
    }
}

/// Emitted by an [`crate::backend::OutputBackend`] whenever the compositor
/// reports a settled configuration snapshot (a `wlr-output-management-v1`
/// `done` event, or the mock backend's equivalent). Always a full
/// snapshot, never a diff — callers debounce and act on the latest one.
#[derive(Debug, Clone)]
pub enum TopologyEvent {
    Snapshot(Vec<Head>),
}

/// A literal mode, or "whatever the head reports as preferred" — used when
/// placing a head we don't have stored geometry for (extra-output policy).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ModeSpec {
    Preferred,
    Exact {
        width: i32,
        height: i32,
        refresh_mhz: i32,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct HeadPlan {
    pub connector: String,
    pub enabled: bool,
    /// `None` only when `enabled` is false — a disabled head's mode is
    /// irrelevant.
    pub mode: Option<ModeSpec>,
    pub position: (i32, i32),
    pub transform: Transform,
    pub scale: f64,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct LayoutPlan {
    pub heads: Vec<HeadPlan>,
}

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum ValidationError {
    #[error("refusing to apply a configuration with zero enabled outputs")]
    ZeroEnabledOutputs,
}

impl LayoutPlan {
    /// The non-negotiable safety rail: never apply a plan that would leave
    /// every output disabled. Must be called before any backend
    /// `apply_configuration`.
    pub fn validate(&self) -> Result<(), ValidationError> {
        if !self.heads.iter().any(|h| h.enabled) {
            return Err(ValidationError::ZeroEnabledOutputs);
        }
        Ok(())
    }

    /// Whether `heads` already describes this plan, i.e. applying it would
    /// change nothing.
    ///
    /// This is what stops the daemon re-applying forever. Hyprland emits a
    /// fresh `done` event for *every* configuration commit, including one
    /// that sets each property to the value it already had — so an
    /// unconditional apply feeds its own event back into the debounce loop
    /// and re-applies at the debounce interval indefinitely. Checking
    /// before applying breaks the cycle at the source, and skips a pointless
    /// mode set on every hotplug besides.
    pub fn is_satisfied_by(&self, heads: &[Head]) -> bool {
        self.heads.iter().all(|plan| {
            let Some(live) = heads.iter().find(|h| h.connector == plan.connector) else {
                return false;
            };
            if live.enabled != plan.enabled {
                return false;
            }
            // A disabled head's geometry is unobservable, so nothing else
            // about it can be out of date.
            if !plan.enabled {
                return true;
            }
            let mode_matches = match plan.mode {
                None => true,
                Some(ModeSpec::Exact {
                    width,
                    height,
                    refresh_mhz,
                }) => live.current_mode.is_some_and(|m| {
                    m.width == width && m.height == height && m.refresh_mhz == refresh_mhz
                }),
                Some(ModeSpec::Preferred) => match (live.current_mode, live.preferred_mode()) {
                    (Some(current), Some(preferred)) => {
                        current.width == preferred.width
                            && current.height == preferred.height
                            && current.refresh_mhz == preferred.refresh_mhz
                    }
                    _ => false,
                },
            };
            mode_matches
                && live.position == plan.position
                && live.transform == plan.transform
                && same_scale(live.scale, plan.scale)
        })
    }
}

/// Compares scales at the precision the protocol actually carries.
///
/// `wlr-output-management` transports scale as a `wl_fixed` — 1/256ths — so
/// a profile storing 1.6 reads back as 1.6015625 (410/256) once the
/// compositor has quantized it. Comparing the raw f64s would call those
/// unequal forever, which would defeat [`LayoutPlan::is_satisfied_by`] for
/// any scale that isn't already a multiple of 1/256 and reinstate the
/// re-apply loop it exists to prevent.
fn same_scale(a: f64, b: f64) -> bool {
    const WL_FIXED_DENOMINATOR: f64 = 256.0;
    (a * WL_FIXED_DENOMINATOR).round() == (b * WL_FIXED_DENOMINATOR).round()
}
