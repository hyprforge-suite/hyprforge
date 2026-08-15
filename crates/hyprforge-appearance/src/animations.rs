//! Per-animation speed, curve and style.
//!
//! Animations look like they need a different model from the rest of this
//! module — `hl.animation({...})` is a call per animation, not a nested
//! config table, so it reads as append-shaped like `hl.bind`. Measured on
//! 0.56.1, it isn't: a later call for the same `leaf` **overrides** the
//! earlier one (`windows` went from 4.79/easeOutQuint to 9.99/linear on
//! the second call). So this is the same overlay as everything else in the
//! crate — emit only the leaves the module owns, sourced last — and it can
//! share one generated file and one require line with the settings.
//!
//! Nothing here hardcodes which animations exist. `hyprctl animations -j`
//! reports all 35 leaves with their current values and an `overridden`
//! flag — the exact analogue of `set` for `hyprctl getoption` — plus every
//! curve currently defined, including the ones a user declared with their
//! own `hl.curve` calls. A hardcoded list would go stale and would miss
//! the user's own curves, which are the interesting ones to pick from.
//!
//! **Curves are read, never written.** Authoring a bezier needs a
//! four-control-point editor with a live preview to be anything but
//! guesswork, so curves stay the user's to define and this module only
//! offers the ones that already exist.

use hyprforge_core::lua::lua_string;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::process::Command;

/// Hyprland's own bookkeeping, not something a user configures.
const INTERNAL_PREFIX: &str = "__internal";

/// One animation's settings, as `hl.animation` takes them.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Animation {
    pub enabled: bool,
    pub speed: f64,
    /// A curve name. Empty means Hyprland's built-in default.
    #[serde(default)]
    pub bezier: String,
    /// e.g. `popin 87%` or `fade`. Empty means the animation's own
    /// default style; not every leaf accepts one.
    #[serde(default)]
    pub style: String,
}

/// The animations this module owns, keyed by leaf name.
///
/// A `BTreeMap` for the same reason the settings use one: byte-stable
/// output, so a save doesn't reshuffle the file and bury a real change in
/// noise.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Animations {
    #[serde(default, flatten)]
    pub items: BTreeMap<String, Animation>,
}

impl Animations {
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn get(&self, leaf: &str) -> Option<&Animation> {
        self.items.get(leaf)
    }

    pub fn set(&mut self, leaf: &str, animation: Animation) {
        self.items.insert(leaf.to_string(), animation);
    }

    /// Hands the leaf back to Hyprland and the user's own config.
    pub fn clear(&mut self, leaf: &str) {
        self.items.remove(leaf);
    }

    /// Leaves whose stored values can't be written, with the reason.
    ///
    /// A negative or non-finite speed is the one that matters: it reaches
    /// the generated file as `speed = -1` or `speed = NaN.0`, and the
    /// second is a syntax error that takes every other line in the file
    /// with it.
    pub fn invalid(&self) -> Vec<(String, String)> {
        let mut out = Vec::new();
        for (leaf, a) in &self.items {
            if !a.speed.is_finite() {
                out.push((leaf.clone(), "speed must be an ordinary number".to_string()));
            } else if a.speed <= 0.0 {
                out.push((leaf.clone(), "speed must be greater than 0".to_string()));
            } else if leaf.trim().is_empty() {
                out.push((leaf.clone(), "an animation needs a name".to_string()));
            }
        }
        out
    }
}

/// Renders the owned animations as `hl.animation` calls.
///
/// Anything [`Animations::invalid`] rejects is skipped rather than
/// written: one bad value would otherwise take down the whole generated
/// file, and the editor surfaces the same problems separately so nothing
/// is dropped silently.
pub fn generate(animations: &Animations) -> String {
    let bad: Vec<String> = animations.invalid().into_iter().map(|(l, _)| l).collect();
    let mut out = String::new();
    for (leaf, a) in &animations.items {
        if bad.contains(leaf) {
            continue;
        }
        out.push_str(&render_one(leaf, a));
    }
    out
}

/// One `hl.animation` line, exactly as [`generate`] writes it. Public so
/// an editor can show the user the line their form produces — the same
/// function, so a preview can't drift into a plausible-looking lie.
pub fn render_one(leaf: &str, a: &Animation) -> String {
    let mut fields = vec![
        format!("leaf = {}", lua_string(leaf)),
        format!("enabled = {}", a.enabled),
        format!("speed = {}", render_speed(a.speed)),
    ];
    // Both are omitted rather than written empty: `bezier = ""` asks
    // Hyprland for a curve with no name instead of leaving the default
    // alone, and an empty style is not the same as no style.
    if !a.bezier.trim().is_empty() {
        fields.push(format!("bezier = {}", lua_string(a.bezier.trim())));
    }
    if !a.style.trim().is_empty() {
        fields.push(format!("style = {}", lua_string(a.style.trim())));
    }
    format!("hl.animation({{ {} }})\n", fields.join(", "))
}

/// Speeds are written the way Hyprland's own docs and configs write them
/// — `4.79`, `10` — so a generated file reads like a hand-written one.
fn render_speed(speed: f64) -> String {
    if speed.fract() == 0.0 && speed.abs() < 1e15 {
        format!("{}", speed as i64)
    } else {
        format!("{speed}")
    }
}

/// One animation as the running compositor currently has it.
#[derive(Debug, Clone, PartialEq)]
pub struct LiveAnimation {
    pub leaf: String,
    pub animation: Animation,
    /// Something has set this leaf, rather than it being Hyprland's own
    /// default — the analogue of `set` for a config option.
    pub overridden: bool,
}

/// A bezier curve that currently exists, for picking by name.
#[derive(Debug, Clone, PartialEq)]
pub struct Curve {
    pub name: String,
    /// The two control points, `(x0, y0, x1, y1)`, for a preview.
    pub points: (f64, f64, f64, f64),
}

#[derive(Debug, thiserror::Error)]
pub enum LiveError {
    #[error("couldn't run hyprctl: {0}")]
    Hyprctl(#[source] std::io::Error),
    #[error("hyprctl returned nothing — is Hyprland running?")]
    NoCompositor,
}

/// Every animation leaf and every defined curve, from the running
/// compositor.
pub fn live() -> Result<(Vec<LiveAnimation>, Vec<Curve>), LiveError> {
    let out = Command::new("hyprctl")
        .args(["animations", "-j"])
        .output()
        .map_err(LiveError::Hyprctl)?;
    let body = String::from_utf8_lossy(&out.stdout).to_string();
    if body.trim().is_empty() {
        return Err(LiveError::NoCompositor);
    }
    Ok(parse_live(&body))
}

/// Split out from [`live`] so the parsing is testable without a
/// compositor — the part that can actually be wrong.
pub fn parse_live(json: &str) -> (Vec<LiveAnimation>, Vec<Curve>) {
    let Ok(root) = serde_json::from_str::<serde_json::Value>(json) else {
        return (Vec::new(), Vec::new());
    };
    let animations = root
        .get(0)
        .and_then(|v| v.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|a| {
                    let leaf = a.get("name")?.as_str()?.to_string();
                    if leaf.starts_with(INTERNAL_PREFIX) {
                        return None;
                    }
                    Some(LiveAnimation {
                        leaf,
                        animation: Animation {
                            enabled: a.get("enabled")?.as_bool().unwrap_or(true),
                            speed: a.get("speed")?.as_f64().unwrap_or(0.0),
                            bezier: a.get("bezier").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                            style: a.get("style").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                        },
                        overridden: a.get("overridden").and_then(|v| v.as_bool()).unwrap_or(false),
                    })
                })
                .collect()
        })
        .unwrap_or_default();

    let curves = root
        .get(1)
        .and_then(|v| v.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|c| {
                    Some(Curve {
                        name: c.get("name")?.as_str()?.to_string(),
                        points: (
                            c.get("X0")?.as_f64()?,
                            c.get("Y0")?.as_f64()?,
                            c.get("X1")?.as_f64()?,
                            c.get("Y1")?.as_f64()?,
                        ),
                    })
                })
                .collect()
        })
        .unwrap_or_default();

    (animations, curves)
}

/// One animation recovered from a recorded `hl.animation({...})` call.
///
/// Takes plain `(kind, args)` rather than a
/// `hyprforge_lua_import::RecordedCall`, so this crate never depends on
/// the evaluator — and therefore never on `mlua`. This is
/// [`render_one`] in reverse.
pub fn animation_from_call(kind: &str, args: &[serde_json::Value]) -> Option<(String, Animation)> {
    if kind != "animation" {
        return None;
    }
    let table = args.first()?.as_object()?;
    let leaf = table.get("leaf")?.as_str()?.trim();
    if leaf.is_empty() || leaf.starts_with(INTERNAL_PREFIX) {
        return None;
    }
    Some((
        leaf.to_string(),
        Animation {
            enabled: table.get("enabled").and_then(|v| v.as_bool()).unwrap_or(true),
            // A call with no speed is one Hyprland would reject, so
            // there's nothing sensible to import.
            speed: table.get("speed").and_then(|v| v.as_f64())?,
            bezier: table
                .get("bezier")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            style: table
                .get("style")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn animation(speed: f64) -> Animation {
        Animation {
            enabled: true,
            speed,
            bezier: "easeOutQuint".to_string(),
            style: String::new(),
        }
    }

    #[test]
    fn a_line_matches_the_shape_hyprland_takes() {
        let line = render_one("windows", &animation(4.79));
        assert_eq!(
            line,
            "hl.animation({ leaf = [[windows]], enabled = true, speed = 4.79, bezier = [[easeOutQuint]] })\n"
        );
    }

    /// `bezier = ""` asks for a curve with no name rather than leaving
    /// the default alone, and an empty style is not the same as no style.
    #[test]
    fn empty_curve_and_style_are_omitted_rather_than_written_blank() {
        let a = Animation {
            enabled: true,
            speed: 3.0,
            bezier: String::new(),
            style: "   ".to_string(),
        };
        let line = render_one("fade", &a);
        assert!(!line.contains("bezier"), "{line}");
        assert!(!line.contains("style"), "{line}");
    }

    #[test]
    fn a_style_is_written_when_set() {
        let a = Animation {
            enabled: true,
            speed: 4.1,
            bezier: "easeOutQuint".to_string(),
            style: "popin 87%".to_string(),
        };
        assert!(render_one("windowsIn", &a).contains("style = [[popin 87%]]"));
    }

    /// Whole speeds read better unadorned, the way Hyprland's own docs
    /// and every hand-written config write them.
    #[test]
    fn a_whole_speed_is_written_without_a_decimal_point() {
        assert!(render_one("global", &animation(10.0)).contains("speed = 10,"));
        assert!(render_one("global", &animation(1.5)).contains("speed = 1.5,"));
    }

    /// A speed of zero or less is refused by Hyprland, and NaN would
    /// render as a syntax error taking the whole file with it.
    #[test]
    fn an_unusable_speed_is_reported_and_never_written() {
        for bad in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            let mut a = Animations::default();
            a.set("windows", animation(bad));
            a.set("fade", animation(2.0));
            assert_eq!(a.invalid().len(), 1, "{bad} was accepted");
            let lua = generate(&a);
            assert!(!lua.contains("windows"), "{bad}: {lua}");
            assert!(lua.contains("fade"), "{bad}: {lua}");
        }
    }

    #[test]
    fn an_empty_set_generates_nothing() {
        assert_eq!(generate(&Animations::default()), "");
    }

    #[test]
    fn output_is_stable_across_runs() {
        let mut a = Animations::default();
        a.set("windows", animation(4.79));
        a.set("fade", animation(3.03));
        assert_eq!(generate(&a), generate(&a));
    }

    /// Real `hyprctl animations -j` output, trimmed to three entries.
    const SAMPLE: &str = r#"[[
        {"name":"specialWorkspaceOut","overridden":false,"bezier":"","enabled":true,"speed":0.00,"style":""},
        {"name":"windows","overridden":true,"bezier":"easeOutQuint","enabled":true,"speed":4.79,"style":""},
        {"name":"__internal_fadeCTM","overridden":false,"bezier":"","enabled":true,"speed":0.00,"style":""}
    ],[
        {"name":"quick","X0":0.15,"Y0":0.0,"X1":0.1,"Y1":1.0},
        {"name":"linear","X0":0.0,"Y0":0.0,"X1":1.0,"Y1":1.0}
    ]]"#;

    #[test]
    fn live_output_yields_animations_and_curves() {
        let (animations, curves) = parse_live(SAMPLE);
        assert_eq!(animations.len(), 2, "internal leaves are excluded");
        let windows = animations.iter().find(|a| a.leaf == "windows").unwrap();
        assert!(windows.overridden);
        assert_eq!(windows.animation.speed, 4.79);
        assert_eq!(windows.animation.bezier, "easeOutQuint");

        assert_eq!(curves.len(), 2);
        assert_eq!(curves[0].name, "quick");
        assert_eq!(curves[0].points, (0.15, 0.0, 0.1, 1.0));
    }

    /// `overridden` is what tells a configured animation from one left at
    /// Hyprland's default, the same job `set` does for config options.
    #[test]
    fn an_unconfigured_leaf_is_marked_as_not_overridden() {
        let (animations, _) = parse_live(SAMPLE);
        let special = animations.iter().find(|a| a.leaf == "specialWorkspaceOut").unwrap();
        assert!(!special.overridden);
    }

    #[test]
    fn malformed_live_output_is_not_fatal() {
        assert_eq!(parse_live("not json"), (Vec::new(), Vec::new()));
        assert_eq!(parse_live("[]"), (Vec::new(), Vec::new()));
    }

    #[test]
    fn a_recorded_call_round_trips_back_into_an_animation() {
        let (leaf, a) = animation_from_call(
            "animation",
            &[serde_json::json!({
                "leaf": "windowsIn", "enabled": true, "speed": 4.1,
                "bezier": "easeOutQuint", "style": "popin 87%"
            })],
        )
        .unwrap();
        assert_eq!(leaf, "windowsIn");
        assert_eq!(a.speed, 4.1);
        assert_eq!(a.style, "popin 87%");
        // The forward direction agrees with the reverse.
        assert!(render_one(&leaf, &a).contains("speed = 4.1"));
    }

    #[test]
    fn a_call_without_a_speed_is_not_importable() {
        assert!(animation_from_call("animation", &[serde_json::json!({ "leaf": "fade" })]).is_none());
    }

    #[test]
    fn a_non_animation_call_yields_nothing() {
        assert!(animation_from_call("bind", &[serde_json::json!({ "leaf": "fade" })]).is_none());
        assert!(animation_from_call("animation", &[]).is_none());
    }

    #[test]
    fn internal_leaves_are_never_imported() {
        assert!(animation_from_call(
            "animation",
            &[serde_json::json!({ "leaf": "__internal_fadeCTM", "speed": 1.0 })]
        )
        .is_none());
    }
}
