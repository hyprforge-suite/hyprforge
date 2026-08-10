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

use hyprforge_core::geometry::{logical_size, nearest_valid_scale};

/// The scale this record will really run at, which is not always the one
/// it stores.
///
/// Hyprland takes a scale only if it divides the resolution cleanly in both
/// axes; asked for anything else it substitutes one that does, without
/// saying so. Every position in the plan is computed from the scale, so a
/// stored value the hardware can't take leaves the layout built against a
/// width nothing is using — and since we refuse to apply overlapping
/// outputs, that surfaces as a layout that just won't go on.
///
/// Snapping here rather than in `set_head_geometry` keeps the stored TOML
/// exactly as the user wrote it, and means the planned positions and the
/// value handed to the compositor agree on one number.
fn snapped_scale(rec: &HeadRecord) -> f64 {
    let snapped = nearest_valid_scale(rec.width, rec.height, rec.scale);
    if (snapped - rec.scale).abs() > 1e-9 {
        tracing::warn!(
            connector = %rec.connector_hint,
            asked = rec.scale,
            using = snapped,
            "scale doesn't divide {}x{} cleanly; the compositor would substitute one that does",
            rec.width,
            rec.height,
        );
    }
    snapped
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

/// The layout space a planned head occupies, as `(width, height)`.
fn plan_logical_size(plan: &HeadPlan, connected: &[Head]) -> (i32, i32) {
    let dims = match plan.mode {
        Some(ModeSpec::Exact { width, height, .. }) => Some((width, height)),
        _ => connected
            .iter()
            .find(|c| c.connector == plan.connector)
            .and_then(|c| c.effective_mode())
            .map(|m| (m.width, m.height)),
    };
    let (w, h) = dims.unwrap_or((1920, 1080));
    let (lw, lh) = (logical_size(w, plan.scale), logical_size(h, plan.scale));
    match plan.transform {
        Transform::Rotate90 | Transform::Rotate270 | Transform::Flipped90 | Transform::Flipped270 => {
            (lh, lw)
        }
        _ => (lw, lh),
    }
}

/// Slides overlapping heads apart, left to right, preserving their order.
///
/// Overlapping outputs give a desktop with a region owned by two monitors at
/// once; Hyprland warns that it "will cause issues", and it is easy to
/// arrive at without ever dragging anything — changing a head's scale
/// changes the space it occupies while every position stays put, so a setup
/// that was flush silently starts overlapping.
///
/// Heads placed by the mirror policy are exempt: sharing a position is the
/// entire point there, not a mistake to correct.
fn resolve_plan_overlaps(heads_plan: &mut [HeadPlan], connected: &[Head], exempt: &[String]) {
    let mut order: Vec<usize> = (0..heads_plan.len())
        .filter(|&i| heads_plan[i].enabled && !exempt.contains(&heads_plan[i].connector))
        .collect();
    order.sort_by_key(|&i| (heads_plan[i].position.0, heads_plan[i].position.1));

    let mut placed: Vec<(i32, i32, i32, i32)> = Vec::new();
    for &i in &order {
        let (w, h) = plan_logical_size(&heads_plan[i], connected);
        let (mut x, y) = heads_plan[i].position;
        // Push right past anything already placed that this would sit on.
        // Repeats because clearing one neighbour can run into the next.
        for _ in 0..placed.len() + 1 {
            let hit = placed
                .iter()
                .filter(|&&(px, py, pw, ph)| {
                    x < px + pw && px < x + w && y < py + ph && py < y + h
                })
                .map(|&(px, _, pw, _)| px + pw)
                .max();
            match hit {
                Some(edge) => x = edge,
                None => break,
            }
        }
        if x != heads_plan[i].position.0 {
            tracing::debug!(
                connector = %heads_plan[i].connector,
                from = heads_plan[i].position.0,
                to = x,
                "nudged an output right to stop it overlapping another"
            );
            heads_plan[i].position.0 = x;
        }
        placed.push((x, y, w, h));
    }
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
    // Mirror-policy heads share a position deliberately, so they're excluded
    // from overlap repair below.
    let mut mirrored: Vec<String> = Vec::new();

    for head in connected {
        if let Some(rec) = connector_to_record.get(head.connector.as_str()) {
            // Snapped before anything is derived from it, so the positions
            // below, the overlap repair, and the compositor all work from
            // the same scale.
            let scale = snapped_scale(rec);
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
                scale,
            });
            if rec.enabled {
                placed.push((rec.x, rec.y, rec.width, scale));
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
                    .map(|(x, _, w, s)| x + logical_size(*w, *s))
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
                mirrored.push(head.connector.clone());
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

    resolve_plan_overlaps(&mut heads_plan, connected, &mirrored);
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

    /// The way a user actually reaches an overlapping desktop: change a
    /// monitor's scale and every position stays where it was, so a layout
    /// that was flush now has two outputs claiming the same region.
    /// Hyprland warns that this "will cause issues".
    #[test]
    fn raising_a_scale_does_not_leave_outputs_overlapping() {
        let boe = identity("BOE", "0x0BC9", "");
        let arzopa = identity("GWD", "ARZOPA", "2022110200001");

        // eDP-2 was placed flush at scale 1.6 (2560/1.6 = 1600 wide), with
        // the external starting exactly at 1600. Dropping the scale to 1.0
        // widens it to 2560 and it swallows its neighbour.
        let mut left = head_record("eDP-2", &boe, 0, 0, 2560, 1600);
        left.scale = 1.0;
        let mut right = head_record("DP-3", &arzopa, 1600, 0, 2560, 1440);
        right.scale = 1.0;
        let p = profile("p1", vec![left, right], "2020-01-01T00:00:00Z");

        let connected = vec![
            head("eDP-2", boe, 2560, 1600),
            head("DP-3", arzopa, 2560, 1440),
        ];
        let plan = build_layout_plan(&p, &connected);

        let a = plan.heads.iter().find(|h| h.connector == "eDP-2").unwrap();
        let b = plan.heads.iter().find(|h| h.connector == "DP-3").unwrap();
        assert_eq!(a.position.0, 0, "the leftmost head shouldn't move");
        assert_eq!(
            b.position.0, 2560,
            "the external must be pushed clear of the now-wider panel"
        );
    }

    /// A scale can reach a profile from `displayctl set-geometry` or a
    /// hand-edited TOML without ever passing the GUI's dropdown. 175% on a
    /// 2560x1600 panel is one the hardware can't take: left alone, the
    /// compositor substitutes a scale of its own while every position is
    /// computed against 1463, and the layout comes back refused for
    /// overlapping.
    #[test]
    fn a_scale_the_panel_cannot_take_is_snapped_before_anything_uses_it() {
        let boe = identity("BOE", "0x0BC9", "");
        let mut rec = head_record("eDP-2", &boe, 0, 0, 2560, 1600);
        rec.scale = 1.75;
        let p = profile("p1", vec![rec], "2020-01-01T00:00:00Z");
        let connected = vec![head("eDP-2", boe, 2560, 1600)];

        let plan = build_layout_plan(&p, &connected);
        let s = plan.heads[0].scale;
        assert!((s - 1.75).abs() < 0.02, "should stay near what was asked: {s}");
        assert!(
            (2560.0 / s).fract().abs() < 1e-9 && (1600.0 / s).fract().abs() < 1e-9,
            "{s} doesn't divide 2560x1600 cleanly, so the compositor would substitute"
        );
    }

    /// The consequence of the above: two heads laid out flush against the
    /// snapped width stay flush, instead of the second landing inside the
    /// first because it was placed against a width nothing is using.
    #[test]
    fn a_layout_flush_against_a_snapped_scale_does_not_overlap() {
        let boe = identity("BOE", "0x0BC9", "");
        let arzopa = identity("GWD", "ARZOPA", "2022110200001");
        let mut left = head_record("eDP-2", &boe, 0, 0, 2560, 1600);
        left.scale = 1.75;
        // Flush against the width 1.75 really becomes: the nearest scale
        // this panel can take is 320/183, laying it out 1464 wide.
        let mut right = head_record("DP-3", &arzopa, 1464, 0, 2560, 1440);
        right.scale = 1.0;
        let p = profile("p1", vec![left, right], "2020-01-01T00:00:00Z");
        let connected = vec![
            head("eDP-2", boe, 2560, 1600),
            head("DP-3", arzopa, 2560, 1440),
        ];

        let plan = build_layout_plan(&p, &connected);
        plan.validate().unwrap();
        let b = plan.heads.iter().find(|h| h.connector == "DP-3").unwrap();
        assert_eq!(b.position.0, 1464, "a flush neighbour shouldn't be pushed");
    }

    #[test]
    fn a_flush_layout_is_left_exactly_as_it_is() {
        let boe = identity("BOE", "0x0BC9", "");
        let arzopa = identity("GWD", "ARZOPA", "2022110200001");
        let mut left = head_record("eDP-2", &boe, 0, 0, 2560, 1600);
        left.scale = 1.6;
        let mut right = head_record("DP-3", &arzopa, 1600, 0, 2560, 1440);
        right.scale = 1.0;
        let p = profile("p1", vec![left, right], "2020-01-01T00:00:00Z");
        let connected = vec![
            head("eDP-2", boe, 2560, 1600),
            head("DP-3", arzopa, 2560, 1440),
        ];
        let plan = build_layout_plan(&p, &connected);
        let b = plan.heads.iter().find(|h| h.connector == "DP-3").unwrap();
        assert_eq!(b.position.0, 1600, "nothing overlapped, so nothing moves");
    }

    #[test]
    fn stacked_monitors_are_left_alone() {
        // Vertically stacked heads share an x range but not a y one, so
        // they don't overlap and must not be shoved sideways.
        let boe = identity("BOE", "0x0BC9", "");
        let arzopa = identity("GWD", "ARZOPA", "2022110200001");
        let mut top = head_record("eDP-2", &boe, 0, 0, 2560, 1600);
        top.scale = 1.0;
        let mut bottom = head_record("DP-3", &arzopa, 0, 1600, 2560, 1440);
        bottom.scale = 1.0;
        let p = profile("p1", vec![top, bottom], "2020-01-01T00:00:00Z");
        let connected = vec![
            head("eDP-2", boe, 2560, 1600),
            head("DP-3", arzopa, 2560, 1440),
        ];
        let plan = build_layout_plan(&p, &connected);
        let b = plan.heads.iter().find(|h| h.connector == "DP-3").unwrap();
        assert_eq!(b.position, (0, 1600));
    }

    #[test]
    fn mirroring_is_allowed_to_share_a_position() {
        // Mirror puts an uncovered head on top of the first deliberately;
        // the overlap repair must not undo that.
        let boe = identity("BOE", "0x0BC9", "");
        let dell = identity("DELL", "U2720Q", "ABC");
        let mut p = profile(
            "p1",
            vec![head_record("eDP-2", &boe, 0, 0, 2560, 1600)],
            "2020-01-01T00:00:00Z",
        );
        p.extra_output_policy = ExtraOutputPolicy::Mirror;
        let connected = vec![
            head("eDP-2", boe, 2560, 1600),
            head("DP-4", dell, 1920, 1080),
        ];
        let plan = build_layout_plan(&p, &connected);
        let mirror = plan.heads.iter().find(|h| h.connector == "DP-4").unwrap();
        assert_eq!(mirror.position, (0, 0), "mirroring shares a position by design");
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
