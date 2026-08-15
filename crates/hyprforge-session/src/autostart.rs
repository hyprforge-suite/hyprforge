//! Programs started with the session.
//!
//! Hyprland runs these from an event handler, not at config load:
//!
//! ```lua
//! hl.on("hyprland.start", function()
//!     hl.exec_cmd("waybar")
//! end)
//! ```
//!
//! The handler is not decoration. `hl.exec_cmd` at the top level runs
//! whenever the config is evaluated, which includes every `hyprctl
//! reload` — so a bare list would relaunch every program each time
//! anything else in the app saved. Inside `hyprland.start` it runs once,
//! at login. `hyprland.shutdown` is the matching event for anything that
//! should run on the way out.

use hyprforge_core::lua::lua_string;
use serde::{Deserialize, Serialize};

/// When a program runs.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum When {
    #[default]
    Start,
    Shutdown,
}

impl When {
    pub const ALL: [When; 2] = [When::Start, When::Shutdown];

    /// The Hyprland event name.
    pub fn event(self) -> &'static str {
        match self {
            When::Start => "hyprland.start",
            When::Shutdown => "hyprland.shutdown",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            When::Start => "When the session starts",
            When::Shutdown => "When the session ends",
        }
    }
}

/// One program to run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Program {
    /// The shell command, stored and written verbatim. Hyprforge never
    /// runs it.
    pub command: String,
    /// Kept rather than deleted so a program can be turned off without
    /// losing the command — the same reason shortcuts have it.
    #[serde(default = "yes")]
    pub enabled: bool,
    #[serde(default)]
    pub when: When,
    /// Free-text note, for the "why is this here" that comments in a
    /// hand-written config carry.
    #[serde(default)]
    pub note: String,
}

fn yes() -> bool {
    true
}

/// Written by hand rather than derived, because `#[serde(default)]`
/// governs only deserialization — a derived `Default` would give
/// `enabled: false`, and a program added in the editor would be created
/// switched off.
impl Default for Program {
    fn default() -> Self {
        Program {
            command: String::new(),
            enabled: true,
            when: When::Start,
            note: String::new(),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Settings {
    #[serde(default, rename = "program")]
    pub programs: Vec<Program>,
}

impl Settings {
    pub fn is_empty(&self) -> bool {
        self.programs.is_empty()
    }

    pub fn invalid(&self) -> Vec<(usize, String)> {
        let mut out = Vec::new();
        for (i, program) in self.programs.iter().enumerate() {
            if program.command.trim().is_empty() {
                out.push((i, "needs a command".to_string()));
            }
        }
        // Two identical commands at the same event start the program
        // twice, which for a daemon means two copies fighting — exactly
        // what happened to hyprpaper's IPC socket here.
        let mut seen: Vec<(&str, When)> = Vec::new();
        for (i, program) in self.programs.iter().enumerate() {
            if !program.enabled {
                continue;
            }
            let key = (program.command.trim(), program.when);
            if seen.contains(&key) {
                out.push((i, "already started above — it would run twice".to_string()));
            }
            seen.push(key);
        }
        out
    }
}

/// Renders the `hl.on(...)` blocks.
pub fn generate(settings: &Settings) -> String {
    let bad: Vec<usize> = settings.invalid().into_iter().map(|(i, _)| i).collect();
    let mut out = String::new();
    for when in When::ALL {
        let programs: Vec<&Program> = settings
            .programs
            .iter()
            .enumerate()
            .filter(|(i, p)| p.when == when && p.enabled && !bad.contains(i))
            .map(|(_, p)| p)
            .collect();
        if programs.is_empty() {
            continue;
        }
        out.push_str(&format!("hl.on({}, function()\n", lua_string(when.event())));
        for program in programs {
            if !program.note.trim().is_empty() {
                out.push_str(&format!("    -- {}\n", program.note.trim()));
            }
            out.push_str(&format!("    hl.exec_cmd({})\n", lua_string(program.command.trim())));
        }
        out.push_str("end)\n");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn program(command: &str) -> Program {
        Program { command: command.into(), enabled: true, ..Program::default() }
    }

    /// The handler is what makes this run once at login rather than on
    /// every `hyprctl reload`.
    #[test]
    fn programs_are_wrapped_in_the_start_event() {
        let settings = Settings { programs: vec![program("waybar")] };
        let out = generate(&settings);
        assert!(out.starts_with("hl.on([[hyprland.start]], function()\n"), "{out}");
        assert!(out.contains("    hl.exec_cmd([[waybar]])\n"), "{out}");
        assert!(out.trim_end().ends_with("end)"), "{out}");
    }

    #[test]
    fn each_event_gets_its_own_block() {
        let settings = Settings {
            programs: vec![
                program("waybar"),
                Program { command: "save-state".into(), enabled: true, when: When::Shutdown, note: String::new() },
            ],
        };
        let out = generate(&settings);
        assert_eq!(out.matches("hl.on(").count(), 2, "{out}");
        assert!(out.contains("hyprland.shutdown"), "{out}");
        // Start first: it's the one people look at.
        assert!(out.find("hyprland.start") < out.find("hyprland.shutdown"), "{out}");
    }

    /// Disabled keeps the command rather than deleting it, so turning a
    /// program off doesn't lose what it was.
    #[test]
    fn a_disabled_program_is_kept_but_not_written() {
        let settings = Settings {
            programs: vec![Program { command: "waybar".into(), enabled: false, ..Program::default() }],
        };
        assert_eq!(generate(&settings), "");
        assert_eq!(settings.programs[0].command, "waybar");
    }

    /// A command is shell and is written exactly as typed — quoting it
    /// would change what runs.
    #[test]
    fn a_command_is_written_verbatim() {
        let settings = Settings { programs: vec![program("systemctl --user start hyprpolkitagent")] };
        assert!(generate(&settings).contains("hl.exec_cmd([[systemctl --user start hyprpolkitagent]])"));
    }

    /// A command containing `]]` would otherwise close the Lua long
    /// bracket early and break the file.
    #[test]
    fn a_command_containing_brackets_is_quoted_safely() {
        let settings = Settings { programs: vec![program("echo a]]b")] };
        let out = generate(&settings);
        assert!(out.contains("]=]") || out.contains("]==]"), "bracket level must escalate: {out}");
    }

    #[test]
    fn a_note_becomes_a_comment_above_its_command() {
        let settings = Settings {
            programs: vec![Program {
                command: "hyprlock".into(),
                enabled: true,
                when: When::Start,
                note: "lock immediately on autologin".into(),
            }],
        };
        let out = generate(&settings);
        assert!(out.contains("    -- lock immediately on autologin\n    hl.exec_cmd"), "{out}");
    }

    /// Starting a daemon twice means two copies fighting — which is
    /// exactly how hyprpaper's IPC socket got broken on this machine.
    #[test]
    fn the_same_command_twice_is_reported() {
        let settings = Settings { programs: vec![program("waybar"), program("waybar")] };
        let problems = settings.invalid();
        assert_eq!(problems.len(), 1);
        assert_eq!(problems[0].0, 1);
        assert!(problems[0].1.contains("twice"), "{}", problems[0].1);
    }

    /// The same command at different events is not a duplicate.
    #[test]
    fn the_same_command_at_different_events_is_fine() {
        let settings = Settings {
            programs: vec![
                program("sync-state"),
                Program { command: "sync-state".into(), enabled: true, when: When::Shutdown, note: String::new() },
            ],
        };
        assert_eq!(settings.invalid(), vec![]);
    }

    /// A disabled duplicate isn't a duplicate — it never runs.
    #[test]
    fn a_disabled_duplicate_is_not_reported() {
        let settings = Settings {
            programs: vec![
                program("waybar"),
                Program { command: "waybar".into(), enabled: false, ..Program::default() },
            ],
        };
        assert_eq!(settings.invalid(), vec![]);
    }

    #[test]
    fn a_program_without_a_command_is_reported_and_skipped() {
        let settings = Settings { programs: vec![program(""), program("waybar")] };
        assert_eq!(settings.invalid().len(), 1);
        let out = generate(&settings);
        assert_eq!(out.matches("exec_cmd").count(), 1, "{out}");
    }

    /// A derived `Default` gives `enabled: false`, so a program added in
    /// the editor would be created switched off and silently not run.
    #[test]
    fn a_new_program_is_enabled() {
        assert!(Program::default().enabled);
    }

    #[test]
    fn an_empty_set_generates_nothing() {
        assert_eq!(generate(&Settings::default()), "");
    }
}
