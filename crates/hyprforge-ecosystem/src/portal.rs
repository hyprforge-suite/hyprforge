//! Screen sharing, via `xdg-desktop-portal-hyprland`.
//!
//! The portal is what a browser or Discord talks to when it asks to
//! capture the screen, and `~/.config/hypr/xdph.conf` is the only place
//! its behaviour can be changed. It is hyprlang, like hypridle's and
//! hyprpaper's, so it uses the same generated-file-plus-`source =`
//! arrangement as the rest of this crate.
//!
//! **Only `screencopy` is modelled.** The binary also registers
//! `general:toplevel_dynamic_bind`, but the wiki documents neither its
//! type nor its default, and this project's rule is that a settings
//! screen states checked claims — a toggle whose effect cannot be
//! described accurately is worse than no toggle. It is still recognised
//! on import, so a user who has set it keeps it: see
//! [`crate::import::portal`].
//!
//! Applying is a restart, and a loud one. The portal reads this file at
//! startup only, and restarting it while a call is in progress drops the
//! screen share — so this is never done automatically, the way hypridle
//! isn't either.

use hyprforge_core::hyprlang;
use serde::{Deserialize, Serialize};

/// What the cursor does in a shared stream for clients that don't ask.
///
/// Browsers are the ones that don't ask, which is why this is worth a
/// control at all: it is the difference between a screen share where the
/// other side can follow your pointer and one where they can't.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CursorMode {
    /// Leave the protocol's own default, which is hidden. Written as `0`.
    #[default]
    Default,
    /// Explicitly hidden. Written as `1`.
    Hidden,
    /// Drawn into the stream. Written as `2`.
    Embedded,
}

impl CursorMode {
    pub const ALL: [CursorMode; 3] =
        [CursorMode::Default, CursorMode::Hidden, CursorMode::Embedded];

    /// The number xdph expects. Not a `serde` repr: the TOML stays
    /// readable as words, and the mapping to integers is a fact about
    /// xdph rather than about this app's storage.
    pub fn as_int(self) -> i64 {
        match self {
            CursorMode::Default => 0,
            CursorMode::Hidden => 1,
            CursorMode::Embedded => 2,
        }
    }

    pub fn from_int(value: i64) -> Option<CursorMode> {
        CursorMode::ALL.into_iter().find(|m| m.as_int() == value)
    }

    pub fn label(self) -> &'static str {
        match self {
            CursorMode::Default => "Let the app decide (hidden)",
            CursorMode::Hidden => "Never show the pointer",
            CursorMode::Embedded => "Show the pointer",
        }
    }
}

impl std::fmt::Display for CursorMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.label())
    }
}

/// xdph's own default frame rate, from the wiki's table. Held as a
/// constant so the screen can say what leaving the field empty means
/// rather than making the user guess.
pub const DEFAULT_MAX_FPS: i64 = 120;

/// The picker xdph runs when nothing else is named.
pub const DEFAULT_PICKER: &str = "hyprland-share-picker";

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Settings {
    /// `None` leaves xdph's own default of [`DEFAULT_MAX_FPS`]. `Some(0)`
    /// is *not* the same thing — it means no limit at all, which is a
    /// deliberate choice someone might make on a fast machine.
    #[serde(default)]
    pub max_fps: Option<i64>,
    /// Ticks "allow restore token" in the picker, so an app that shares
    /// repeatedly stops asking which window every time.
    #[serde(default)]
    pub allow_token_by_default: bool,
    /// Empty leaves [`DEFAULT_PICKER`].
    #[serde(default)]
    pub custom_picker_binary: String,
    /// Skips DMA-BUF in favour of SHM. Slower, but the documented way
    /// around allocation failures on multi-GPU machines — which is
    /// exactly the shape of "screen sharing is just black" that sends
    /// people looking for this file.
    #[serde(default)]
    pub force_shm: bool,
    #[serde(default)]
    pub cursor_mode: CursorMode,
}

impl Settings {
    pub fn is_empty(&self) -> bool {
        *self == Settings::default()
    }

    /// Settings that can't be written, with the reason.
    pub fn invalid(&self) -> Vec<(usize, String)> {
        let mut out = Vec::new();
        if let Some(fps) = self.max_fps {
            if fps < 0 {
                out.push((0, "frame rate can't be negative".to_string()));
            }
        }
        // A picker that isn't there means the share dialog never appears
        // and the request times out with nothing on screen — the silent
        // no-op this project keeps meeting, in the one place where the
        // user is staring at a call waiting for it.
        let picker = self.custom_picker_binary.trim();
        if !picker.is_empty() && picker.contains('/') && !std::path::Path::new(picker).is_file() {
            out.push((0, format!("there's no program at {picker}")));
        }
        out
    }
}

/// Renders the generated `xdph.conf` fragment.
///
/// Only what differs from xdph's own defaults is written. A file that
/// restates every default would bake today's defaults in permanently: a
/// later xdph could change one of its own and this file would keep the
/// old value forever, with nothing to say why.
pub fn generate(settings: &Settings) -> String {
    let bad = !settings.invalid().is_empty();
    let mut out = hyprlang::header("screen sharing settings");
    if bad {
        return out;
    }

    let mut fields: Vec<(&str, String)> = Vec::new();
    if let Some(fps) = settings.max_fps {
        fields.push(("max_fps", fps.to_string()));
    }
    if settings.allow_token_by_default {
        fields.push(("allow_token_by_default", "true".to_string()));
    }
    let picker = settings.custom_picker_binary.trim();
    if !picker.is_empty() {
        fields.push(("custom_picker_binary", picker.to_string()));
    }
    if settings.force_shm {
        fields.push(("force_shm", "true".to_string()));
    }
    if settings.cursor_mode != CursorMode::Default {
        fields.push(("cursor_mode", settings.cursor_mode.as_int().to_string()));
    }

    if !fields.is_empty() {
        out.push('\n');
        out.push_str(&hyprlang::block("screencopy", &fields));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_configured_writes_no_block_at_all() {
        let out = generate(&Settings::default());
        assert!(!out.contains("screencopy"), "{out}");
    }

    /// Restating a default would freeze it: xdph could change its own
    /// default in a later release and this file would silently keep the
    /// old one forever.
    #[test]
    fn only_what_differs_from_the_portals_own_default_is_written() {
        let settings = Settings { force_shm: true, ..Settings::default() };
        let out = generate(&settings);
        assert!(out.contains("force_shm = true"), "{out}");
        assert!(!out.contains("max_fps"), "{out}");
        assert!(!out.contains("cursor_mode"), "{out}");
        assert!(!out.contains("allow_token_by_default"), "{out}");
    }

    /// `None` means "leave xdph's default"; `Some(0)` means "no limit".
    /// Collapsing them would silently uncap someone's frame rate.
    #[test]
    fn an_unset_frame_rate_is_not_the_same_as_no_limit() {
        assert!(!generate(&Settings::default()).contains("max_fps"));
        let unlimited = Settings { max_fps: Some(0), ..Settings::default() };
        assert!(generate(&unlimited).contains("max_fps = 0"));
    }

    #[test]
    fn the_cursor_mode_is_written_as_the_number_xdph_expects() {
        let settings = Settings { cursor_mode: CursorMode::Embedded, ..Settings::default() };
        assert!(generate(&settings).contains("cursor_mode = 2"), "{}", generate(&settings));
        assert_eq!(CursorMode::from_int(1), Some(CursorMode::Hidden));
        assert_eq!(CursorMode::from_int(9), None);
    }

    /// A picker that isn't there means the share dialog never appears and
    /// the request times out with nothing on screen.
    #[test]
    fn a_picker_that_does_not_exist_is_refused_rather_than_written() {
        let settings = Settings {
            custom_picker_binary: "/nope/not-a-picker".to_string(),
            ..Settings::default()
        };
        assert_eq!(settings.invalid().len(), 1);
        assert!(!generate(&settings).contains("custom_picker_binary"));
    }

    /// A bare name is looked up on `PATH` by xdph, so it isn't this
    /// app's place to insist the file exists here.
    #[test]
    fn a_bare_program_name_is_left_for_the_portal_to_resolve() {
        let settings = Settings {
            custom_picker_binary: "my-picker".to_string(),
            ..Settings::default()
        };
        assert_eq!(settings.invalid(), vec![]);
        assert!(generate(&settings).contains("custom_picker_binary = my-picker"));
    }
}
