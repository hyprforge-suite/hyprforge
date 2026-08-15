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

/// Renders the generated `idle.conf`.
pub fn generate(settings: &Settings) -> String {
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
