//! Environment variables set for the session.
//!
//! `hl.env(NAME, VALUE)` — applied to everything Hyprland launches.
//!
//! **These are the most dangerous settings in the app.** This machine's
//! config sets `AQ_DRM_DEVICES = /dev/dri/card2`, which chooses the GPU
//! Hyprland renders on; a wrong value there is a session that doesn't come
//! up, edited from a settings app that is no longer running. So the rules
//! here are stricter than elsewhere:
//!
//! - A variable is never silently dropped or reordered.
//! - Names are validated against what a shell will actually accept, since
//!   Hyprland takes any string and an invalid name simply never reaches
//!   the child process.
//! - [`DANGEROUS`] names are flagged in the editor so a change to one is
//!   deliberate rather than incidental.

use hyprforge_core::lua::lua_string;
use hyprforge_core::supersede;
use serde::{Deserialize, Serialize};

/// Variables where a wrong value costs more than a wrong setting
/// usually does — the session failing to start, or hardware
/// acceleration silently falling back to software.
///
/// Not a blocklist. The editor still writes them; it just says so first.
pub const DANGEROUS: &[(&str, &str)] = &[
    ("AQ_DRM_DEVICES", "Chooses which GPU Hyprland renders on. A wrong path here is a session that won't start."),
    ("WLR_DRM_DEVICES", "Chooses which GPU is used. A wrong path here is a session that won't start."),
    ("LIBVA_DRIVER_NAME", "Selects the video acceleration driver. A wrong name disables hardware video decoding."),
    ("XDG_CURRENT_DESKTOP", "Tells apps and portals which desktop this is. Changing it affects screen sharing and file dialogs."),
    ("XDG_SESSION_TYPE", "Tells apps this is a Wayland session."),
    ("GBM_BACKEND", "Selects the graphics buffer backend. Usually only set for Nvidia."),
    ("__GLX_VENDOR_LIBRARY_NAME", "Selects the GLX vendor. Usually only set for Nvidia."),
];

pub fn danger(name: &str) -> Option<&'static str> {
    DANGEROUS
        .iter()
        .find(|(n, _)| *n == name.trim())
        .map(|(_, why)| *why)
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Variable {
    pub name: String,
    pub value: String,
    #[serde(default = "yes")]
    pub enabled: bool,
}

fn yes() -> bool {
    true
}

/// By hand, not derived: `#[serde(default)]` governs deserialization
/// only, and a derived `Default` would create every new variable
/// switched off.
impl Default for Variable {
    fn default() -> Self {
        Variable {
            name: String::new(),
            value: String::new(),
            enabled: true,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Settings {
    #[serde(default, rename = "variable")]
    pub variables: Vec<Variable>,
}

impl Settings {
    pub fn is_empty(&self) -> bool {
        self.variables.is_empty()
    }

    pub fn invalid(&self) -> Vec<(usize, String)> {
        let mut out = Vec::new();
        for (i, variable) in self.variables.iter().enumerate() {
            if let Some(problem) = check_name(&variable.name) {
                out.push((i, problem));
            }
        }
        // A later `hl.env` for the same name wins, so every *earlier* one
        // is dead weight that reads as if it were in effect.
        //
        // Which row gets flagged is not cosmetic: `generate` skips whatever
        // lands in here, so flagging the last occurrence — the one that
        // actually applies — dropped the winner from the file and wrote the
        // superseded value instead. Editing a variable and leaving the old
        // row above it made the *old* value take effect, in the one module
        // where a wrong value can stop the session starting.
        for i in supersede::superseded(&self.variables, |v| {
            v.enabled.then(|| v.name.trim().to_string())
        }) {
            let name = self.variables[i].name.trim();
            out.push((i, format!("{name} is set again below — this one has no effect")));
        }
        out
    }
}

/// Whether `name` is something a child process will actually receive.
///
/// Hyprland accepts any string, and an invalid name is simply never
/// visible to the program that needed it — so a typo here is a setting
/// that silently does nothing.
pub fn check_name(name: &str) -> Option<String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Some("needs a name".to_string());
    }
    if trimmed.starts_with(|c: char| c.is_ascii_digit()) {
        return Some("can't start with a digit".to_string());
    }
    if !trimmed
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_')
    {
        return Some("only letters, digits and underscores".to_string());
    }
    None
}

/// Renders the `hl.env` calls.
pub fn generate(settings: &Settings) -> String {
    let bad: Vec<usize> = settings.invalid().into_iter().map(|(i, _)| i).collect();
    let mut out = String::new();
    for (i, variable) in settings.variables.iter().enumerate() {
        if !variable.enabled || bad.contains(&i) {
            continue;
        }
        out.push_str(&format!(
            "hl.env({}, {})\n",
            lua_string(variable.name.trim()),
            // The value is *not* trimmed: a trailing space can be
            // meaningful in a path list, and silently changing it would
            // be the app editing something the user didn't.
            lua_string(&variable.value)
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn variable(name: &str, value: &str) -> Variable {
        Variable { name: name.into(), value: value.into(), enabled: true }
    }

    #[test]
    fn a_variable_renders_as_a_name_value_call() {
        let settings = Settings { variables: vec![variable("GTK_THEME", "Dracula")] };
        assert_eq!(generate(&settings), "hl.env([[GTK_THEME]], [[Dracula]])\n");
    }

    /// A trailing space can be meaningful in a path list, and trimming it
    /// would be the app editing something the user didn't.
    #[test]
    fn the_value_is_not_trimmed() {
        let settings = Settings { variables: vec![variable("PATH_LIST", "/a:/b ")] };
        assert!(generate(&settings).contains("[[/a:/b ]]"));
    }

    /// Hyprland takes any string, so an invalid name is a setting that
    /// silently never reaches the program that needed it.
    #[test]
    fn an_unusable_name_is_reported_and_skipped() {
        for bad in ["", "2FAST", "has space", "has-dash", "has$dollar"] {
            let settings = Settings { variables: vec![variable(bad, "x")] };
            assert_eq!(settings.invalid().len(), 1, "{bad} was accepted");
            assert_eq!(generate(&settings), "", "{bad} was written");
        }
    }

    #[test]
    fn ordinary_names_are_accepted() {
        for good in ["GTK_THEME", "_UNDERSCORE", "MIXED_case_9"] {
            assert_eq!(check_name(good), None, "{good} was refused");
        }
    }

    /// A later `hl.env` for the same name wins, so an earlier one reads
    /// as if it were in effect while doing nothing.
    ///
    /// The index is the whole point: this asserted `1` — the row that
    /// actually applies — which is both a message that isn't true of it
    /// ("set again below" when nothing is below it) and, because
    /// `generate` skips flagged rows, the reason the file ended up with
    /// the superseded value.
    #[test]
    fn the_row_that_loses_is_the_one_that_gets_flagged() {
        let settings = Settings {
            variables: vec![variable("GTK_THEME", "A"), variable("GTK_THEME", "B")],
        };
        let problems = settings.invalid();
        assert_eq!(problems.len(), 1);
        assert_eq!(problems[0].0, 0, "the dead row is the first one, not the one in effect");
        assert!(problems[0].1.contains("GTK_THEME"), "{}", problems[0].1);
    }

    /// The consequence of flagging the wrong row, stated directly: a user
    /// edits a variable, leaves the old row above it, and the old value is
    /// what the session gets.
    #[test]
    fn a_duplicated_variable_still_writes_the_value_that_wins() {
        let settings = Settings {
            variables: vec![variable("NVIDIA_DRM", "0"), variable("NVIDIA_DRM", "1")],
        };
        let out = generate(&settings);
        assert_eq!(out.matches("hl.env").count(), 1, "{out}");
        assert!(out.contains("[[1]]"), "the later value must be the one written: {out}");
        assert!(!out.contains("[[0]]"), "the superseded value must not be written: {out}");
    }

    /// Three in a row: only the last survives, and both losers say so.
    #[test]
    fn every_superseded_row_is_flagged_not_just_the_first() {
        let settings = Settings {
            variables: vec![
                variable("PATH_LIKE", "a"),
                variable("PATH_LIKE", "b"),
                variable("PATH_LIKE", "c"),
            ],
        };
        let flagged: Vec<usize> = settings.invalid().into_iter().map(|(i, _)| i).collect();
        assert_eq!(flagged, vec![0, 1]);
    }

    #[test]
    fn a_disabled_variable_is_kept_but_not_written() {
        let settings = Settings {
            variables: vec![Variable { name: "GTK_THEME".into(), value: "Dracula".into(), enabled: false }],
        };
        assert_eq!(generate(&settings), "");
        assert_eq!(settings.variables[0].value, "Dracula");
    }

    /// The variables where a wrong value means no session at all get
    /// flagged, so changing one is deliberate.
    #[test]
    fn gpu_variables_are_flagged_as_dangerous() {
        assert!(danger("AQ_DRM_DEVICES").unwrap().contains("won't start"));
        assert!(danger("LIBVA_DRIVER_NAME").is_some());
        assert!(danger("GTK_THEME").is_none());
        assert!(danger("  AQ_DRM_DEVICES  ").is_some(), "spacing must not hide it");
    }

    /// Flagging is advice, not a block — the user still owns the setting.
    #[test]
    fn a_dangerous_variable_is_still_written() {
        let settings = Settings { variables: vec![variable("AQ_DRM_DEVICES", "/dev/dri/card2")] };
        assert_eq!(settings.invalid(), vec![]);
        assert!(generate(&settings).contains("AQ_DRM_DEVICES"));
    }

    #[test]
    fn a_new_variable_is_enabled() {
        assert!(Variable::default().enabled);
    }

    #[test]
    fn an_empty_set_generates_nothing() {
        assert_eq!(generate(&Settings::default()), "");
    }
}
