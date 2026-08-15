//! Screen colour temperature, via hyprsunset.
//!
//! A schedule: each `profile` block names a time of day and the
//! temperature, gamma and identity to hold from then until the next one.
//! hyprsunset picks the profile whose `time` has most recently passed, so
//! **order in the file doesn't matter but coverage does** — a schedule
//! whose earliest profile is 21:00 leaves the morning to whichever profile
//! is last, which is rarely what someone means.
//!
//! Applying is live: `hyprctl hyprsunset temperature <k>` takes effect
//! immediately, so the app can show the change rather than describe it.

use hyprforge_core::hyprlang;
use serde::{Deserialize, Serialize};

/// Warmest and coolest hyprsunset accepts. 6500K is neutral daylight;
/// below ~2000K the screen is essentially orange.
pub const MIN_TEMPERATURE: i64 = 1000;
pub const MAX_TEMPERATURE: i64 = 20_000;
/// The wiki's absolute ceiling for `max-gamma`, as a percentage.
pub const MAX_GAMMA_PERCENT: i64 = 200;

/// One `profile { … }` block.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Profile {
    /// `HH:MM`, when this profile starts applying.
    pub time: String,
    pub temperature: i64,
    /// Perceived brightness, below what the panel's own minimum allows.
    #[serde(default = "one")]
    pub gamma: f64,
    /// Ignore `temperature` entirely and only apply `gamma`.
    #[serde(default)]
    pub identity: bool,
}

fn one() -> f64 {
    1.0
}

impl Default for Profile {
    fn default() -> Self {
        Profile {
            time: "00:00".to_string(),
            temperature: 6000,
            gamma: 1.0,
            identity: false,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Settings {
    /// Percentage ceiling for gamma. `None` leaves hyprsunset's default.
    #[serde(default)]
    pub max_gamma: Option<i64>,
    #[serde(default, rename = "profile")]
    pub profiles: Vec<Profile>,
}

impl Settings {
    pub fn is_empty(&self) -> bool {
        self.profiles.is_empty() && self.max_gamma.is_none()
    }

    /// Profiles that can't be written, with the reason.
    pub fn invalid(&self) -> Vec<(usize, String)> {
        let mut out = Vec::new();
        for (i, p) in self.profiles.iter().enumerate() {
            if parse_time(&p.time).is_none() {
                out.push((i, "time must be HH:MM".to_string()));
            } else if !(MIN_TEMPERATURE..=MAX_TEMPERATURE).contains(&p.temperature) {
                out.push((
                    i,
                    format!("temperature must be between {MIN_TEMPERATURE} and {MAX_TEMPERATURE}"),
                ));
            } else if !p.gamma.is_finite() || p.gamma <= 0.0 {
                out.push((i, "gamma must be greater than 0".to_string()));
            }
        }
        let mut seen: Vec<u32> = Vec::new();
        for (i, p) in self.profiles.iter().enumerate() {
            if let Some(minutes) = parse_time(&p.time) {
                if seen.contains(&minutes) {
                    out.push((i, format!("a second profile at {} — only one applies", p.time)));
                }
                seen.push(minutes);
            }
        }
        out
    }

    /// Whether the schedule leaves part of the day uncovered by an
    /// explicit profile.
    ///
    /// Not an error — hyprsunset wraps to the last profile of the day —
    /// but it surprises people: a schedule starting at 21:00 means the
    /// *night* setting is what runs all morning.
    pub fn has_midnight_gap(&self) -> bool {
        !self.profiles.is_empty()
            && !self
                .profiles
                .iter()
                .any(|p| parse_time(&p.time) == Some(0))
    }
}

/// `HH:MM` to minutes past midnight, or `None` if it isn't a real time.
///
/// Strict on purpose: hyprsunset takes the string as written, and `25:00`
/// or `9:5` produce a profile that never fires — a schedule silently
/// missing an entry.
pub fn parse_time(time: &str) -> Option<u32> {
    let (h, m) = time.trim().split_once(':')?;
    if h.len() != 2 || m.len() != 2 {
        return None;
    }
    let (h, m): (u32, u32) = (h.parse().ok()?, m.parse().ok()?);
    if h > 23 || m > 59 {
        return None;
    }
    Some(h * 60 + m)
}

/// Renders the generated `sunset.conf`.
pub fn generate(settings: &Settings) -> String {
    let bad: Vec<usize> = settings.invalid().into_iter().map(|(i, _)| i).collect();
    let mut out = hyprlang::header("colour temperature settings");
    if let Some(max_gamma) = settings.max_gamma {
        out.push_str(&hyprlang::keyword("max-gamma", max_gamma));
    }
    for (i, profile) in settings.profiles.iter().enumerate() {
        if bad.contains(&i) {
            continue;
        }
        out.push('\n');
        out.push_str(&render_one(profile));
    }
    out
}

/// One `profile` block, exactly as [`generate`] writes it.
pub fn render_one(profile: &Profile) -> String {
    let mut fields = vec![
        ("time", profile.time.trim().to_string()),
        ("temperature", profile.temperature.to_string()),
        ("gamma", format!("{}", profile.gamma)),
    ];
    // Only when set: `identity = false` is the default, and writing it
    // suggests a decision nobody made.
    if profile.identity {
        fields.push(("identity", "true".to_string()));
    }
    hyprlang::block("profile", &fields)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn profile(time: &str, temperature: i64) -> Profile {
        Profile { time: time.into(), temperature, ..Profile::default() }
    }

    #[test]
    fn a_profile_block_carries_time_temperature_and_gamma() {
        let out = render_one(&profile("21:00", 5500));
        assert_eq!(out, "profile {\n    time = 21:00\n    temperature = 5500\n    gamma = 1\n}\n");
    }

    #[test]
    fn identity_is_only_written_when_set() {
        let mut p = profile("21:00", 5500);
        assert!(!render_one(&p).contains("identity"));
        p.identity = true;
        assert!(render_one(&p).contains("identity = true"));
    }

    /// hyprsunset takes the time string as written, so `25:00` or `9:5`
    /// produce a profile that never fires — an entry silently missing
    /// from the schedule.
    #[test]
    fn only_a_real_hh_mm_time_is_accepted() {
        assert_eq!(parse_time("00:00"), Some(0));
        assert_eq!(parse_time("21:30"), Some(21 * 60 + 30));
        assert_eq!(parse_time("23:59"), Some(23 * 60 + 59));
        for bad in ["25:00", "9:5", "21:60", "2100", "", "ab:cd", "21:00:00"] {
            assert_eq!(parse_time(bad), None, "{bad} was accepted");
        }
    }

    #[test]
    fn an_unusable_profile_is_reported_and_skipped() {
        let mut s = Settings::default();
        s.profiles.push(profile("9:5", 5500));
        s.profiles.push(profile("21:00", 5500));
        assert_eq!(s.invalid().len(), 1);
        let out = generate(&s);
        assert_eq!(out.matches("profile {").count(), 1, "{out}");
    }

    #[test]
    fn an_out_of_range_temperature_is_refused_with_its_limits() {
        let mut s = Settings::default();
        s.profiles.push(profile("00:00", 500));
        assert!(s.invalid()[0].1.contains("1000"), "{:?}", s.invalid());
    }

    #[test]
    fn a_non_positive_gamma_is_refused() {
        for bad in [0.0, -1.0, f64::NAN] {
            let mut s = Settings::default();
            let mut p = profile("00:00", 6000);
            p.gamma = bad;
            s.profiles.push(p);
            assert_eq!(s.invalid().len(), 1, "{bad} was accepted");
        }
    }

    #[test]
    fn two_profiles_at_the_same_time_are_reported() {
        let mut s = Settings::default();
        s.profiles.push(profile("21:00", 5500));
        s.profiles.push(profile("21:00", 4000));
        assert!(s.invalid()[0].1.contains("21:00"), "{:?}", s.invalid());
    }

    /// A schedule starting at 21:00 means the *night* setting runs all
    /// morning, because hyprsunset wraps to the last profile of the day.
    #[test]
    fn a_schedule_with_no_midnight_profile_is_flagged() {
        let mut s = Settings::default();
        s.profiles.push(profile("21:00", 5500));
        assert!(s.has_midnight_gap());
        s.profiles.push(profile("00:00", 6500));
        assert!(!s.has_midnight_gap());
    }

    #[test]
    fn an_empty_schedule_is_not_flagged_as_a_gap() {
        assert!(!Settings::default().has_midnight_gap());
    }

    #[test]
    fn an_empty_set_is_still_a_loadable_file() {
        let out = generate(&Settings::default());
        assert!(out.starts_with("# Generated by Hyprforge"));
        assert!(!out.contains("profile {"));
    }
}
