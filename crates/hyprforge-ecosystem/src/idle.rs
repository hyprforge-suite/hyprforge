//! Idle behaviour, via hypridle.
//!
//! A `general` block of session commands, plus a `listener` block per
//! idle timeout: wait N seconds, run something, and optionally run
//! something else when the user comes back.
//!
//! **hypridle has no IPC.** `hyprctl hypridle` answers "unknown request",
//! so unlike every other module here a saved change does not take effect
//! until the daemon restarts. The screen has to say so rather than
//! implying the change is live — this is exactly the "reports a save that
//! did nothing" failure, and the only defence is honesty about it.
//!
//! Every value here is a **shell command**. Hyprforge stores and writes
//! them verbatim and never runs them itself; the daemon does, via
//! `/bin/sh -c`.

use hyprforge_core::hyprlang;
use serde::{Deserialize, Serialize};

/// The canonical `idle.toml` behind the Idle & lock page.
///
/// Public, like [`crate::wallpaper::settings_path`], because two writers
/// share it: the Settings page and `hyprforge-setup`'s "lock when idle"
/// item. Two private copies of a path are two places for it to drift.
pub fn settings_path() -> std::path::PathBuf {
    hyprforge_core::paths::hyprforge_config_dir().join("idle.toml")
}

/// The generated `idle.conf` the settings are rendered into.
pub fn generated_path() -> std::path::PathBuf {
    hyprforge_core::paths::hypr_hyprforge_dir().join("idle.conf")
}

/// hypridle's own config, which sources [`generated_path`].
pub fn hypridle_conf_path() -> std::path::PathBuf {
    hyprforge_core::paths::hypr_config_dir().join("hypridle.conf")
}

/// Session-wide commands and inhibitor handling.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct General {
    /// Run when something asks the session to lock over D-Bus.
    #[serde(default)]
    pub lock_cmd: String,
    #[serde(default)]
    pub unlock_cmd: String,
    #[serde(default)]
    pub before_sleep_cmd: String,
    #[serde(default)]
    pub after_sleep_cmd: String,
    #[serde(default)]
    pub on_lock_cmd: String,
    #[serde(default)]
    pub on_unlock_cmd: String,
    #[serde(default)]
    pub ignore_dbus_inhibit: bool,
    #[serde(default)]
    pub ignore_systemd_inhibit: bool,
    #[serde(default)]
    pub ignore_wayland_inhibit: bool,
}

impl General {
    pub fn is_empty(&self) -> bool {
        *self == General::default()
    }

    /// `(field, label, value)` for every command, in editor order.
    pub fn commands(&self) -> [(&'static str, &'static str, &String); 6] {
        [
            ("lock_cmd", "On lock request", &self.lock_cmd),
            ("unlock_cmd", "On unlock request", &self.unlock_cmd),
            ("before_sleep_cmd", "Before sleep", &self.before_sleep_cmd),
            ("after_sleep_cmd", "After waking", &self.after_sleep_cmd),
            ("on_lock_cmd", "Once locked", &self.on_lock_cmd),
            ("on_unlock_cmd", "Once unlocked", &self.on_unlock_cmd),
        ]
    }

    pub fn set_command(&mut self, field: &str, value: String) {
        match field {
            "lock_cmd" => self.lock_cmd = value,
            "unlock_cmd" => self.unlock_cmd = value,
            "before_sleep_cmd" => self.before_sleep_cmd = value,
            "after_sleep_cmd" => self.after_sleep_cmd = value,
            "on_lock_cmd" => self.on_lock_cmd = value,
            "on_unlock_cmd" => self.on_unlock_cmd = value,
            _ => {}
        }
    }
}

/// One `listener { … }` block.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Listener {
    /// Seconds of inactivity before `on_timeout` runs.
    pub timeout: u32,
    #[serde(default)]
    pub on_timeout: String,
    #[serde(default)]
    pub on_resume: String,
    /// Fire even while something is inhibiting idle (a video playing,
    /// say).
    #[serde(default)]
    pub ignore_inhibit: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Settings {
    #[serde(default)]
    pub general: General,
    #[serde(default, rename = "listener")]
    pub listeners: Vec<Listener>,
}

impl Settings {
    pub fn is_empty(&self) -> bool {
        self.general.is_empty() && self.listeners.is_empty()
    }

    /// Listeners that can't be written, with the reason.
    pub fn invalid(&self) -> Vec<(usize, String)> {
        let mut out = Vec::new();
        for (i, l) in self.listeners.iter().enumerate() {
            if l.timeout == 0 {
                out.push((i, "a timeout of 0 seconds would fire immediately".to_string()));
            } else if l.on_timeout.trim().is_empty() {
                // A listener with nothing to run is a timer that does
                // nothing — hypridle accepts it and it silently wastes
                // the slot.
                out.push((i, "needs a command to run".to_string()));
            }
        }
        out
    }
}

/// A listener or `general` command rewritten from hyprlang-era
/// `hyprctl dispatch` syntax to the form a Lua Hyprland config accepts,
/// or `None` when it needs nothing.
///
/// Under a Lua config, `hyprctl dispatch` takes Lua: `hyprctl dispatch
/// dpms off` is evaluated as `hl.dispatch(dpms off)`, which is a syntax
/// error, and hyprctl exits 7 — inside a hypridle listener, where nobody
/// sees it. A `hypridle.conf` written for a hyprlang Hyprland and
/// imported unchanged left screen-off on idle silently doing nothing
/// (2026-10-10). The form this writes was checked against Hyprland
/// 0.56.2: `hyprctl dispatch 'hl.dsp.dpms({ action = "on" })'` answers
/// `ok`.
///
/// Only `dpms`, the one dispatcher an idle config uses in practice — the
/// catalogue in hyprforge-shortcuts has its Lua shape. Anything else is
/// left as written and reported by [`old_dispatches`], so a command this
/// does not understand is shown to the user rather than guessed at.
pub fn lua_dispatch(command: &str) -> Option<String> {
    const OLD: &str = "hyprctl dispatch dpms";
    if !command.contains(OLD) {
        return None;
    }
    let mut out = String::new();
    let mut rest = command;
    while let Some(at) = rest.find(OLD) {
        out.push_str(&rest[..at]);
        let after = &rest[at + OLD.len()..];
        // Words up to the next shell separator are the arguments.
        let end = after.find(['&', '|', ';', '\n']).unwrap_or(after.len());
        let mut words = after[..end].split_whitespace();
        let action = words.next().unwrap_or("toggle");
        if !matches!(action, "on" | "off" | "toggle") {
            // Not a form this knows; keep the whole command as written.
            return None;
        }
        let call = match words.next() {
            Some(monitor) => format!("{{ action = \"{action}\", monitor = \"{monitor}\" }}"),
            None => format!("{{ action = \"{action}\" }}"),
        };
        out.push_str(&format!("hyprctl dispatch 'hl.dsp.dpms({call})'"));
        // Keep the separator's spacing as the user wrote it.
        let trailing = &after[..end];
        let kept = &trailing[trailing.trim_end().len()..];
        out.push_str(kept);
        rest = &after[end..];
    }
    out.push_str(rest);
    Some(out)
}

/// Commands that still call `hyprctl dispatch` in hyprlang syntax after
/// [`lua_dispatch`] — something only a person can translate. Empty when
/// every dispatch is Lua.
pub fn old_dispatches(settings: &Settings) -> Vec<String> {
    let mut out = Vec::new();
    for command in settings.commands() {
        let fixed = lua_dispatch(command).unwrap_or_else(|| command.to_string());
        for (at, _) in fixed.match_indices("hyprctl dispatch ") {
            let arg = fixed[at + "hyprctl dispatch ".len()..].trim_start();
            if !(arg.starts_with('\'') || arg.starts_with('"') || arg.starts_with("hl.")) {
                out.push(command.to_string());
                break;
            }
        }
    }
    out
}

impl Settings {
    /// Every command string these settings hold: `general`'s and each
    /// listener's.
    pub fn commands(&self) -> Vec<&str> {
        let mut out: Vec<&str> = self.general.commands().into_iter().map(|(_, _, v)| v.as_str()).collect();
        for l in &self.listeners {
            out.push(&l.on_timeout);
            out.push(&l.on_resume);
        }
        out.retain(|c| !c.trim().is_empty());
        out
    }

    /// These settings with every command passed through [`lua_dispatch`],
    /// or `None` when none needed it.
    pub fn with_lua_dispatch(&self) -> Option<Settings> {
        let mut out = self.clone();
        let mut changed = false;
        let mut fix = |s: &mut String| {
            if let Some(new) = lua_dispatch(s) {
                *s = new;
                changed = true;
            }
        };
        for (field, _, value) in self.general.commands() {
            let mut v = value.clone();
            fix(&mut v);
            out.general.set_command(field, v);
        }
        for l in &mut out.listeners {
            fix(&mut l.on_timeout);
            fix(&mut l.on_resume);
        }
        changed.then_some(out)
    }
}

/// Renders the generated `idle.conf`.
///
/// Every command goes out in the Lua dispatch form ([`lua_dispatch`]):
/// whatever `idle.toml` still holds, the file hypridle reads is one a Lua
/// Hyprland accepts.
pub fn generate(settings: &Settings) -> String {
    let fixed = settings.with_lua_dispatch();
    let settings = fixed.as_ref().unwrap_or(settings);
    let bad: Vec<usize> = settings.invalid().into_iter().map(|(i, _)| i).collect();
    let mut out = hyprlang::header("idle settings");

    let mut general = Vec::new();
    for (field, _, value) in settings.general.commands() {
        if !value.trim().is_empty() {
            general.push((field, value.trim().to_string()));
        }
    }
    for (field, set) in [
        ("ignore_dbus_inhibit", settings.general.ignore_dbus_inhibit),
        ("ignore_systemd_inhibit", settings.general.ignore_systemd_inhibit),
        ("ignore_wayland_inhibit", settings.general.ignore_wayland_inhibit),
    ] {
        if set {
            general.push((field, "true".to_string()));
        }
    }
    if !general.is_empty() {
        out.push('\n');
        out.push_str(&hyprlang::block("general", &general));
    }

    for (i, listener) in settings.listeners.iter().enumerate() {
        if bad.contains(&i) {
            continue;
        }
        out.push('\n');
        out.push_str(&render_one(listener));
    }
    out
}

/// One `listener` block, exactly as [`generate`] writes it.
///
/// Note the hyphens: hypridle spells these `on-timeout` and `on-resume`,
/// not `on_timeout`, even though every key in `general` uses underscores.
pub fn render_one(listener: &Listener) -> String {
    let mut fields = vec![("timeout", listener.timeout.to_string())];
    if !listener.on_timeout.trim().is_empty() {
        fields.push(("on-timeout", listener.on_timeout.trim().to_string()));
    }
    if !listener.on_resume.trim().is_empty() {
        fields.push(("on-resume", listener.on_resume.trim().to_string()));
    }
    if listener.ignore_inhibit {
        fields.push(("ignore_inhibit", "true".to_string()));
    }
    hyprlang::block("listener", &fields)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_old_dpms_dispatch_becomes_the_lua_form() {
        assert_eq!(
            lua_dispatch("hyprctl dispatch dpms off").as_deref(),
            Some("hyprctl dispatch 'hl.dsp.dpms({ action = \"off\" })'")
        );
        assert_eq!(
            lua_dispatch("hyprctl dispatch dpms on eDP-1").as_deref(),
            Some("hyprctl dispatch 'hl.dsp.dpms({ action = \"on\", monitor = \"eDP-1\" })'")
        );
    }

    #[test]
    fn the_rest_of_a_compound_command_is_kept_exactly() {
        assert_eq!(
            lua_dispatch("hyprctl dispatch dpms on && brightnessctl -r").as_deref(),
            Some("hyprctl dispatch 'hl.dsp.dpms({ action = \"on\" })' && brightnessctl -r")
        );
    }

    #[test]
    fn a_command_already_in_lua_form_is_left_alone() {
        assert_eq!(lua_dispatch("hyprctl dispatch 'hl.dsp.dpms({ action = \"off\" })'"), None);
        assert_eq!(lua_dispatch("loginctl lock-session"), None);
    }

    #[test]
    fn a_dpms_form_this_does_not_know_is_not_guessed_at() {
        assert_eq!(lua_dispatch("hyprctl dispatch dpms sideways"), None);
    }

    #[test]
    fn generated_idle_conf_never_carries_the_old_syntax() {
        let settings = Settings {
            listeners: vec![Listener {
                timeout: 330,
                on_timeout: "hyprctl dispatch dpms off".into(),
                on_resume: "hyprctl dispatch dpms on".into(),
                ..Listener::default()
            }],
            ..Settings::default()
        };
        let text = generate(&settings);
        assert!(!text.contains("dispatch dpms"), "{text}");
        assert!(text.contains("hl.dsp.dpms({ action = \"off\" })"), "{text}");
        assert!(old_dispatches(&settings).is_empty());
    }

    #[test]
    fn a_dispatch_only_a_person_can_translate_is_reported() {
        let settings = Settings {
            listeners: vec![listener(60, "hyprctl dispatch exec kitty")],
            ..Settings::default()
        };
        assert_eq!(old_dispatches(&settings), vec!["hyprctl dispatch exec kitty".to_string()]);
    }

    fn listener(timeout: u32, on_timeout: &str) -> Listener {
        Listener {
            timeout,
            on_timeout: on_timeout.to_string(),
            ..Listener::default()
        }
    }

    /// hypridle spells these with hyphens while every `general` key uses
    /// underscores — getting it wrong is a listener that silently never
    /// runs anything.
    #[test]
    fn a_listener_uses_hyphenated_keys() {
        let mut l = listener(150, "brightnessctl -s set 10");
        l.on_resume = "brightnessctl -r".to_string();
        let out = render_one(&l);
        assert!(out.contains("on-timeout = brightnessctl -s set 10"), "{out}");
        assert!(out.contains("on-resume = brightnessctl -r"), "{out}");
        assert!(!out.contains("on_timeout"), "{out}");
    }

    #[test]
    fn general_uses_underscored_keys() {
        let mut s = Settings::default();
        s.general.before_sleep_cmd = "loginctl lock-session".to_string();
        let out = generate(&s);
        assert!(out.contains("before_sleep_cmd = loginctl lock-session"), "{out}");
    }

    /// A command is written verbatim: it is shell, and quoting or
    /// escaping it here would change what runs.
    #[test]
    fn a_command_is_written_verbatim() {
        let l = listener(300, "pidof hyprlock || hyprlock");
        assert!(render_one(&l).contains("on-timeout = pidof hyprlock || hyprlock"));
    }

    /// A listener with nothing to run is a timer that does nothing —
    /// hypridle accepts it and it silently wastes the slot.
    #[test]
    fn a_listener_without_a_command_is_reported_and_skipped() {
        let mut s = Settings::default();
        s.listeners.push(listener(150, ""));
        s.listeners.push(listener(300, "loginctl lock-session"));
        assert_eq!(s.invalid().len(), 1);
        let out = generate(&s);
        assert_eq!(out.matches("listener {").count(), 1, "{out}");
    }

    #[test]
    fn a_zero_timeout_is_refused() {
        let mut s = Settings::default();
        s.listeners.push(listener(0, "echo hi"));
        assert_eq!(s.invalid().len(), 1);
        assert!(s.invalid()[0].1.contains("immediately"));
    }

    #[test]
    fn unset_commands_and_flags_are_not_written() {
        let out = generate(&Settings::default());
        assert!(!out.contains("general {"), "{out}");
        assert!(!out.contains("ignore_"), "{out}");
    }

    #[test]
    fn inhibit_flags_are_written_only_when_set() {
        let mut s = Settings::default();
        s.general.ignore_dbus_inhibit = true;
        let out = generate(&s);
        assert!(out.contains("ignore_dbus_inhibit = true"), "{out}");
        assert!(!out.contains("ignore_systemd_inhibit"), "{out}");
    }

    #[test]
    fn every_command_field_round_trips_through_its_name() {
        let mut g = General::default();
        for (field, _, _) in General::default().commands() {
            g.set_command(field, format!("run-{field}"));
        }
        for (field, _, value) in g.commands() {
            assert_eq!(value, &format!("run-{field}"));
        }
    }

    #[test]
    fn an_empty_set_is_still_a_loadable_file() {
        let out = generate(&Settings::default());
        assert!(out.starts_with("# Generated by Hyprforge"));
        assert!(!out.contains("listener {"));
    }
}
