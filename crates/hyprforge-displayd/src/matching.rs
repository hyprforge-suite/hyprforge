use crate::fingerprint::{identity_counts, is_sub_multiset};
use crate::profile::{ExtraOutputPolicy, HeadRecord, Profile};
use crate::types::{Head, HeadPlan, Identity, LayoutPlan, ModeSpec, Transform};
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum MatchTier {
    Exact,
    Superset,
    Subset,
}

pub struct MatchResult<'a> {
    pub profile: &'a Profile,
    pub tier: MatchTier,
}

/// Finds the best-matching stored profile for the currently-connected
/// identity multiset, per the priority spec: exact > superset > subset,
/// tie-broken within a tier by most-recently-used.
pub fn find_match<'a>(profiles: &'a [Profile], connected: &[Identity]) -> Option<MatchResult<'a>> {
    let connected_counts = identity_counts(connected);
    let mut best: Option<MatchResult<'a>> = None;

    for profile in profiles {
        let profile_identities = profile.identities();
        let profile_counts = identity_counts(&profile_identities);

        let tier = if profile_counts == connected_counts {
            MatchTier::Exact
        } else if is_sub_multiset(&profile_counts, &connected_counts) {
            MatchTier::Superset
        } else if is_sub_multiset(&connected_counts, &profile_counts) {
            MatchTier::Subset
        } else {
            continue;
        };

        let is_better = match &best {
            None => true,
            Some(current) => {
                tier < current.tier
                    || (tier == current.tier && profile.last_used > current.profile.last_used)
            }
        };
        if is_better {
            best = Some(MatchResult { profile, tier });
        }
    }

    best
}

fn head_record_identity(rec: &HeadRecord) -> Identity {
    rec.identity()
}

/// How much layout space a `width_px` output at `scale` occupies.
///
/// Compositor coordinates are logical, not physical: scaling a display *up*
/// makes everything bigger and so covers *less* layout space, which is why
/// this divides. A guard on non-positive scales keeps a malformed profile
/// from producing a divide-by-zero position.
fn logical_width(width_px: i32, scale: f64) -> i32 {
    if scale <= 0.0 {
        return width_px;
    }
    (width_px as f64 / scale).round() as i32
}

/// Resolves which connected connector each profile head record should
/// configure. Matching is unaffected by duplicate/blank-serial identities
/// (it works on the multiset), but *assignment* — which stored role goes to
/// which physical position — cannot be derived from EDID alone when two or
/// more heads share an identity. Falls back to connector-name ordering,
/// then applies any persisted `head_swaps` override.
///
/// Returns `connector_hint -> connected connector` for every profile head
/// whose identity is present among `connected` (heads only in the profile,
/// as on a subset match, are simply absent from the result).
pub fn assign_heads(profile: &Profile, connected: &[Head]) -> HashMap<String, String> {
    let mut profile_by_identity: HashMap<Identity, Vec<&HeadRecord>> = HashMap::new();
    for rec in &profile.heads {
        profile_by_identity
            .entry(head_record_identity(rec))
            .or_default()
            .push(rec);
    }

    let mut connected_by_identity: HashMap<Identity, Vec<&Head>> = HashMap::new();
    for head in connected {
        connected_by_identity
            .entry(head.identity.clone())
            .or_default()
            .push(head);
    }

    let mut assignment: HashMap<String, String> = HashMap::new();
    for (identity, mut recs) in profile_by_identity {
        let Some(mut candidates) = connected_by_identity.get(&identity).cloned() else {
            continue;
        };
        recs.sort_by(|a, b| a.connector_hint.cmp(&b.connector_hint));
        candidates.sort_by(|a, b| a.connector.cmp(&b.connector));
        for (rec, head) in recs.iter().zip(candidates.iter()) {
            assignment.insert(rec.connector_hint.clone(), head.connector.clone());
        }
    }

    for (a, b) in &profile.head_swaps {
        let av = assignment.get(a).cloned();
        let bv = assignment.get(b).cloned();
        if let (Some(av), Some(bv)) = (av, bv) {
            assignment.insert(a.clone(), bv);
            assignment.insert(b.clone(), av);
        }
    }

    assignment
}

/// Builds the [`LayoutPlan`] to apply for a matched profile against the
/// currently-connected heads. Heads covered by the profile keep their
/// stored geometry; any connected head the profile doesn't cover (only
/// possible on a superset match) is placed per `profile.extra_output_policy`.
pub fn build_layout_plan(profile: &Profile, connected: &[Head]) -> LayoutPlan {
    let assignment = assign_heads(profile, connected);

    let mut connector_to_record: HashMap<&str, &HeadRecord> = HashMap::new();
    for rec in &profile.heads {
        if let Some(connector) = assignment.get(&rec.connector_hint) {
            connector_to_record.insert(connector.as_str(), rec);
        }
    }

    let mut heads_plan = Vec::new();
    // (x, y, width_px, scale) of heads placed so far, for extend-right math.
    let mut placed: Vec<(i32, i32, i32, f64)> = Vec::new();

    for head in connected {
        if let Some(rec) = connector_to_record.get(head.connector.as_str()) {
            heads_plan.push(HeadPlan {
                connector: head.connector.clone(),
                enabled: rec.enabled,
                mode: Some(ModeSpec::Exact {
                    width: rec.width,
                    height: rec.height,
                    refresh_mhz: rec.refresh_mhz,
                }),
                position: (rec.x, rec.y),
                transform: rec.transform,
                scale: rec.scale,
            });
            if rec.enabled {
                placed.push((rec.x, rec.y, rec.width, rec.scale));
            }
        }
    }

    for head in connected {
        if connector_to_record.contains_key(head.connector.as_str()) {
            continue;
        }
        match profile.extra_output_policy {
            ExtraOutputPolicy::Disable => {
                heads_plan.push(HeadPlan {
                    connector: head.connector.clone(),
                    enabled: false,
                    mode: None,
                    position: (0, 0),
                    transform: Transform::Normal,
                    scale: 1.0,
                });
            }
            ExtraOutputPolicy::ExtendRight => {
                // Logical width is pixels *divided* by scale: a 2560px panel
                // at scale 1.6 occupies 1600 units of layout space, not 4096.
                // Multiplying here parked the new output far off the right of
                // everything else, with a dead gap the pointer had to cross.
                let rightmost = placed
                    .iter()
                    .map(|(x, _, w, s)| x + logical_width(*w, *s))
                    .max()
                    .unwrap_or(0);
                heads_plan.push(HeadPlan {
                    connector: head.connector.clone(),
                    enabled: true,
                    mode: Some(ModeSpec::Preferred),
                    position: (rightmost, 0),
                    transform: Transform::Normal,
                    scale: 1.0,
                });
                if let Some(mode) = head.preferred_mode() {
                    placed.push((rightmost, 0, mode.width, 1.0));
                }
            }
            ExtraOutputPolicy::Mirror => {
                let (x, y) = placed.first().map(|(x, y, ..)| (*x, *y)).unwrap_or((0, 0));
                heads_plan.push(HeadPlan {
                    connector: head.connector.clone(),
                    enabled: true,
                    mode: Some(ModeSpec::Preferred),
                    position: (x, y),
                    transform: Transform::Normal,
                    scale: 1.0,
                });
            }
        }
    }

    LayoutPlan { heads: heads_plan }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::Mode;

    fn identity(make: &str, model: &str, serial: &str) -> Identity {
        Identity {
            make: make.to_string(),
            model: model.to_string(),
            serial: serial.to_string(),
        }
    }

    fn head(connector: &str, id: Identity, w: i32, h: i32) -> Head {
        Head {
            connector: connector.to_string(),
            identity: id,
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

    fn head_record(hint: &str, id: &Identity, x: i32, y: i32, w: i32, h: i32) -> HeadRecord {
        HeadRecord {
            make: id.make.clone(),
            model: id.model.clone(),
            serial: id.serial.clone(),
            connector_hint: hint.to_string(),
            x,
            y,
            width: w,
            height: h,
            refresh_mhz: 60000,
            scale: 1.0,
            transform: Transform::Normal,
            enabled: true,
        }
    }

    fn profile(id: &str, heads: Vec<HeadRecord>, last_used: &str) -> Profile {
        Profile {
            id: id.to_string(),
            name: id.to_string(),
            last_used: last_used.to_string(),
            extra_output_policy: ExtraOutputPolicy::default(),
            head_swaps: Vec::new(),
            heads,
        }
    }

    #[test]
    fn exact_match_beats_everything() {
        let boe = identity("BOE", "0x0BC9", "");
        let dell = identity("DELL", "U2720Q", "ABC");
        let p = profile(
            "p1",
            vec![
                head_record("eDP-2", &boe, 0, 0, 2560, 1600),
                head_record("DP-4", &dell, 2560, 0, 3840, 2160),
            ],
            "2020-01-01T00:00:00Z",
        );
        let connected = vec![boe.clone(), dell.clone()];
        let result = find_match(std::slice::from_ref(&p), &connected).unwrap();
        assert_eq!(result.tier, MatchTier::Exact);
    }

    #[test]
    fn superset_match_when_extra_output_connected() {
        let boe = identity("BOE", "0x0BC9", "");
        let dell = identity("DELL", "U2720Q", "ABC");
        let p = profile(
            "p1",
            vec![head_record("eDP-2", &boe, 0, 0, 2560, 1600)],
            "2020-01-01T00:00:00Z",
        );
        let connected = vec![boe, dell];
        let result = find_match(std::slice::from_ref(&p), &connected).unwrap();
        assert_eq!(result.tier, MatchTier::Superset);
    }

    #[test]
    fn subset_match_when_fewer_outputs_connected() {
        let boe = identity("BOE", "0x0BC9", "");
        let dell = identity("DELL", "U2720Q", "ABC");
        let p = profile(
            "p1",
            vec![
                head_record("eDP-2", &boe, 0, 0, 2560, 1600),
                head_record("DP-4", &dell, 2560, 0, 3840, 2160),
            ],
            "2020-01-01T00:00:00Z",
        );
        let connected = vec![boe];
        let result = find_match(std::slice::from_ref(&p), &connected).unwrap();
        assert_eq!(result.tier, MatchTier::Subset);
    }

    #[test]
    fn no_match_when_disjoint() {
        let boe = identity("BOE", "0x0BC9", "");
        let other = identity("LG", "27GN950", "XYZ");
        let p = profile(
            "p1",
            vec![head_record("eDP-2", &boe, 0, 0, 2560, 1600)],
            "2020-01-01T00:00:00Z",
        );
        let connected = vec![other];
        assert!(find_match(std::slice::from_ref(&p), &connected).is_none());
    }

    #[test]
    fn tie_break_within_tier_by_most_recently_used() {
        let boe = identity("BOE", "0x0BC9", "");
        let older = profile(
            "old",
            vec![head_record("eDP-2", &boe, 0, 0, 2560, 1600)],
            "2020-01-01T00:00:00Z",
        );
        let newer = profile(
            "new",
            vec![head_record("eDP-2", &boe, 100, 0, 2560, 1600)],
            "2025-06-01T00:00:00Z",
        );
        let connected = vec![boe];
        let profiles = [older, newer];
        let result = find_match(&profiles, &connected).unwrap();
        assert_eq!(result.profile.id, "new");
    }

    #[test]
    fn exact_tier_beats_superset_from_a_different_profile() {
        let boe = identity("BOE", "0x0BC9", "");
        let dell = identity("DELL", "U2720Q", "ABC");
        let superset_profile = profile(
            "small",
            vec![head_record("eDP-2", &boe, 0, 0, 2560, 1600)],
            "2025-01-01T00:00:00Z",
        );
        let exact_profile = profile(
            "exact",
            vec![
                head_record("eDP-2", &boe, 0, 0, 2560, 1600),
                head_record("DP-4", &dell, 2560, 0, 3840, 2160),
            ],
            "2000-01-01T00:00:00Z",
        );
        let connected = vec![boe, dell];
        let profiles = [superset_profile, exact_profile];
        let result = find_match(&profiles, &connected).unwrap();
        assert_eq!(result.profile.id, "exact");
    }

    #[test]
    fn duplicate_identity_assigned_by_connector_name_order() {
        let boe = identity("BOE", "0x0BC9", "");
        let p = profile(
            "dup",
            vec![
                head_record("eDP-1", &boe, 0, 0, 1920, 1080),
                head_record("eDP-2", &boe, 1920, 0, 1920, 1080),
            ],
            "2020-01-01T00:00:00Z",
        );
        let connected = vec![
            head("eDP-2", boe.clone(), 1920, 1080),
            head("eDP-1", boe.clone(), 1920, 1080),
        ];
        let assignment = assign_heads(&p, &connected);
        assert_eq!(assignment.get("eDP-1"), Some(&"eDP-1".to_string()));
        assert_eq!(assignment.get("eDP-2"), Some(&"eDP-2".to_string()));
    }

    #[test]
    fn head_swap_override_flips_duplicate_assignment() {
        let boe = identity("BOE", "0x0BC9", "");
        let mut p = profile(
            "dup",
            vec![
                head_record("eDP-1", &boe, 0, 0, 1920, 1080),
                head_record("eDP-2", &boe, 1920, 0, 1920, 1080),
            ],
            "2020-01-01T00:00:00Z",
        );
        p.head_swaps.push(("eDP-1".to_string(), "eDP-2".to_string()));
        let connected = vec![
            head("eDP-1", boe.clone(), 1920, 1080),
            head("eDP-2", boe.clone(), 1920, 1080),
        ];
        let assignment = assign_heads(&p, &connected);
        assert_eq!(assignment.get("eDP-1"), Some(&"eDP-2".to_string()));
        assert_eq!(assignment.get("eDP-2"), Some(&"eDP-1".to_string()));
    }

    #[test]
    fn extend_right_places_uncovered_head_at_rightmost_edge() {
        let boe = identity("BOE", "0x0BC9", "");
        let dell = identity("DELL", "U2720Q", "ABC");
        let p = profile(
            "p1",
            vec![head_record("eDP-2", &boe, 0, 0, 2560, 1600)],
            "2020-01-01T00:00:00Z",
        );
        let connected = vec![
            head("eDP-2", boe, 2560, 1600),
            head("DP-4", dell, 3840, 2160),
        ];
        let plan = build_layout_plan(&p, &connected);
        plan.validate().unwrap();
        let dp4 = plan.heads.iter().find(|h| h.connector == "DP-4").unwrap();
        assert!(dp4.enabled);
        assert_eq!(dp4.position, (2560, 0));
    }

    #[test]
    fn extend_right_accounts_for_the_scale_of_what_it_places_beside() {
        // A 2560px panel at scale 1.6 occupies 1600 units of layout space,
        // so the second output belongs at x=1600. Multiplying instead put it
        // at 4096 — off past a dead gap two thirds the width of the desktop.
        let boe = identity("BOE", "0x0BC9", "");
        let arzopa = identity("GWD", "ARZOPA", "2022110200001");
        let mut rec = head_record("eDP-2", &boe, 0, 0, 2560, 1600);
        rec.scale = 1.6;
        let p = profile("p1", vec![rec], "2020-01-01T00:00:00Z");
        let connected = vec![
            head("eDP-2", boe, 2560, 1600),
            head("DP-3", arzopa, 2560, 1440),
        ];
        let plan = build_layout_plan(&p, &connected);
        plan.validate().unwrap();
        let dp3 = plan.heads.iter().find(|h| h.connector == "DP-3").unwrap();
        assert_eq!(dp3.position, (1600, 0));
    }

    #[test]
    fn logical_width_survives_a_zero_scale() {
        assert_eq!(logical_width(2560, 1.6), 1600);
        assert_eq!(logical_width(2560, 1.0), 2560);
        // A malformed profile must not produce an infinite coordinate.
        assert_eq!(logical_width(2560, 0.0), 2560);
    }

    #[test]
    fn disable_policy_disables_uncovered_head() {
        let boe = identity("BOE", "0x0BC9", "");
        let dell = identity("DELL", "U2720Q", "ABC");
        let mut p = profile(
            "p1",
            vec![head_record("eDP-2", &boe, 0, 0, 2560, 1600)],
            "2020-01-01T00:00:00Z",
        );
        p.extra_output_policy = ExtraOutputPolicy::Disable;
        let connected = vec![
            head("eDP-2", boe, 2560, 1600),
            head("DP-4", dell, 3840, 2160),
        ];
        let plan = build_layout_plan(&p, &connected);
        let dp4 = plan.heads.iter().find(|h| h.connector == "DP-4").unwrap();
        assert!(!dp4.enabled);
    }

    #[test]
    fn subset_match_plan_only_covers_connected_heads() {
        let boe = identity("BOE", "0x0BC9", "");
        let dell = identity("DELL", "U2720Q", "ABC");
        let p = profile(
            "p1",
            vec![
                head_record("eDP-2", &boe, 0, 0, 2560, 1600),
                head_record("DP-4", &dell, 2560, 0, 3840, 2160),
            ],
            "2020-01-01T00:00:00Z",
        );
        let connected = vec![head("eDP-2", boe, 2560, 1600)];
        let plan = build_layout_plan(&p, &connected);
        assert_eq!(plan.heads.len(), 1);
        plan.validate().unwrap();
    }

    #[test]
    fn zero_enabled_outputs_is_refused() {
        let boe = identity("BOE", "0x0BC9", "");
        let mut rec = head_record("eDP-2", &boe, 0, 0, 2560, 1600);
        rec.enabled = false;
        let p = profile("p1", vec![rec], "2020-01-01T00:00:00Z");
        let connected = vec![head("eDP-2", boe, 2560, 1600)];
        let plan = build_layout_plan(&p, &connected);
        assert_eq!(
            plan.validate(),
            Err(crate::types::ValidationError::ZeroEnabledOutputs)
        );
    }
}
