use crate::types::{Head, Identity, Transform};
use serde::{Deserialize, Serialize};

/// What to do with a currently-connected output that a matched profile
/// doesn't cover (only relevant on a superset match). No mirror primitive
/// exists in `wlr-output-management-v1`, so `Mirror` is implemented as
/// "same position, closest-matching mode" rather than a true clone.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExtraOutputPolicy {
    #[default]
    ExtendRight,
    Mirror,
    Disable,
}

/// One head's stored configuration within a profile.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HeadRecord {
    pub make: String,
    pub model: String,
    pub serial: String,
    /// The connector this head was on when saved. Used only as an
    /// assignment tiebreaker when multiple heads share the same identity
    /// (see `matching::assign_heads`) — never as part of the match key.
    pub connector_hint: String,
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    pub refresh_mhz: i32,
    pub scale: f64,
    #[serde(default)]
    pub transform: Transform,
    pub enabled: bool,
}

impl HeadRecord {
    pub fn identity(&self) -> Identity {
        Identity {
            make: self.make.clone(),
            model: self.model.clone(),
            serial: self.serial.clone(),
        }
    }

    fn from_head(head: &Head) -> Self {
        let mode = head.effective_mode();
        HeadRecord {
            make: head.identity.make.clone(),
            model: head.identity.model.clone(),
            serial: head.identity.serial.clone(),
            connector_hint: head.connector.clone(),
            x: head.position.0,
            y: head.position.1,
            width: mode.map(|m| m.width).unwrap_or(0),
            height: mode.map(|m| m.height).unwrap_or(0),
            refresh_mhz: mode.map(|m| m.refresh_mhz).unwrap_or(0),
            scale: head.scale,
            transform: head.transform,
            enabled: head.enabled,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Profile {
    /// Stable key: blake3 hash of the identity multiset. Never the only
    /// representation — `heads` below carries the full identity list too.
    pub id: String,
    /// User-facing, independently renamable — never used for matching.
    pub name: String,
    pub last_used: String,
    #[serde(default)]
    pub extra_output_policy: ExtraOutputPolicy,
    /// Persisted per-head assignment overrides for duplicate/blank-serial
    /// identities: `(connector_hint_a, connector_hint_b)` pairs whose
    /// resolved assignment should be swapped.
    #[serde(default)]
    pub head_swaps: Vec<(String, String)>,
    #[serde(rename = "head")]
    pub heads: Vec<HeadRecord>,
}

impl Profile {
    /// Builds a fresh profile snapshot from the currently-connected heads,
    /// used both for first-save and for auto-learn overwrites. `id` and
    /// `name` are supplied by the caller so auto-learn can preserve them
    /// across an update.
    pub fn from_heads(id: String, name: String, heads: &[Head]) -> Self {
        Profile {
            id,
            name,
            last_used: now_rfc3339(),
            extra_output_policy: ExtraOutputPolicy::default(),
            head_swaps: Vec::new(),
            heads: heads.iter().map(HeadRecord::from_head).collect(),
        }
    }

    pub fn identities(&self) -> Vec<Identity> {
        self.heads.iter().map(HeadRecord::identity).collect()
    }

    pub fn touch(&mut self) {
        self.last_used = now_rfc3339();
    }
}

fn now_rfc3339() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    humantime_rfc3339(now.as_secs())
}

/// Minimal RFC 3339 UTC formatter (`YYYY-MM-DDTHH:MM:SSZ`) — avoids pulling
/// in a full datetime crate for a single timestamp field.
fn humantime_rfc3339(secs: u64) -> String {
    const DAYS_PER_400Y: i64 = 146097;
    let days = (secs / 86400) as i64;
    let secs_of_day = secs % 86400;
    let (h, m, s) = (secs_of_day / 3600, (secs_of_day / 60) % 60, secs_of_day % 60);

    // Civil-from-days algorithm (Howard Hinnant), proleptic Gregorian.
    let z = days + 719468;
    let era = if z >= 0 { z } else { z - DAYS_PER_400Y + 1 } / DAYS_PER_400Y;
    let doe = (z - era * DAYS_PER_400Y) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m_num = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m_num <= 2 { y + 1 } else { y };

    format!("{y:04}-{m_num:02}-{d:02}T{h:02}:{m:02}:{s:02}Z")
}

/// Generates a descriptive profile name: `"{n} display{s}"`, plus
/// `" incl. {make} {model}"` for the largest-area enabled head, uniquified
/// against `existing_names` with a `" (2)"`-style suffix on collision.
pub fn generate_profile_name(heads: &[Head], existing_names: &[String]) -> String {
    let n = heads.len();
    let plural = if n == 1 { "" } else { "s" };
    let base = format!("{n} display{plural}");

    let largest = heads
        .iter()
        .filter(|h| h.enabled)
        .max_by_key(|h| {
            h.effective_mode()
                .map(|m| (m.width as i64) * (m.height as i64))
                .unwrap_or(0)
        });

    let full = match largest {
        Some(h) if !h.identity.make.is_empty() || !h.identity.model.is_empty() => {
            format!("{base} incl. {} {}", h.identity.make, h.identity.model)
        }
        _ => base,
    };

    if !existing_names.contains(&full) {
        return full;
    }
    let mut i = 2;
    loop {
        let candidate = format!("{full} ({i})");
        if !existing_names.contains(&candidate) {
            return candidate;
        }
        i += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::Mode;

    fn head(connector: &str, make: &str, model: &str, w: i32, h: i32) -> Head {
        Head {
            connector: connector.to_string(),
            identity: Identity {
                make: make.to_string(),
                model: model.to_string(),
                serial: String::new(),
            },
            description: String::new(),
            modes: vec![Mode {
                width: w,
                height: h,
                refresh_mhz: 60000,
                preferred: true,
            }],
            current_mode: Some(Mode {
                width: w,
                height: h,
                refresh_mhz: 60000,
                preferred: true,
            }),
            position: (0, 0),
            transform: Transform::Normal,
            scale: 1.0,
            enabled: true,
        }
    }

    #[test]
    fn names_single_display_with_model() {
        let heads = vec![head("eDP-2", "BOE", "0x0BC9", 2560, 1600)];
        let name = generate_profile_name(&heads, &[]);
        assert_eq!(name, "1 display incl. BOE 0x0BC9");
    }

    #[test]
    fn names_multiple_displays_by_largest_area() {
        let heads = vec![
            head("eDP-2", "BOE", "0x0BC9", 2560, 1600),
            head("DP-4", "DELL", "U2720Q", 3840, 2160),
        ];
        let name = generate_profile_name(&heads, &[]);
        assert_eq!(name, "2 displays incl. DELL U2720Q");
    }

    #[test]
    fn uniquifies_on_collision() {
        let heads = vec![head("eDP-2", "BOE", "0x0BC9", 2560, 1600)];
        let existing = vec!["1 display incl. BOE 0x0BC9".to_string()];
        let name = generate_profile_name(&heads, &existing);
        assert_eq!(name, "1 display incl. BOE 0x0BC9 (2)");
    }

    #[test]
    fn rfc3339_formats_known_epochs() {
        assert_eq!(humantime_rfc3339(0), "1970-01-01T00:00:00Z");
        assert_eq!(humantime_rfc3339(951868800), "2000-03-01T00:00:00Z");
        assert_eq!(humantime_rfc3339(1709209845), "2024-02-29T12:30:45Z");
        assert_eq!(humantime_rfc3339(1785801600), "2026-08-04T00:00:00Z");
    }
}
