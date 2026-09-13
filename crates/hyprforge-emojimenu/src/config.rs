//! The picker's own persisted preference: the default skin tone a
//! long-press (or Tab, its keyboard equivalent — see `crate::popup_app`)
//! last chose.
//!
//! Stored at [`hyprforge_paths::emojimenu_toml_path`] — see that
//! function's own doc for why this crate's one setting hangs off the
//! shared `hyprforge` config directory rather than a directory of its
//! own.
//!
//! # First-run is not an error
//!
//! CLAUDE.md is explicit: "never collapse 'this file could not be read'
//! into 'there is nothing configured'." [`load`] follows
//! `hyprforge_core::hlconfig::storage`'s lead (and `look::resolve`'s) —
//! a file that does not exist yet is [`ToneSetting::Default`] with
//! nothing printed at all (every user's very first run of this popup);
//! a file that exists but will not parse is [`ToneSetting::Unreadable`],
//! which the caller (`main.rs`) warns about on stderr before still
//! showing the neutral grid rather than refusing to open. The two must
//! never look the same to a caller, which is why this returns an enum
//! instead of `Option<Tone>` — an `Option` alone cannot tell "nothing
//! saved yet" from "something is saved and broken" apart.

use hyprforge_emoji::Tone;
use serde::{Deserialize, Serialize};
use std::path::Path;

/// The five tones' on-disk names — kebab-case, matching the rest of this
/// suite's TOML (`hyprforge-appearance`, `hyprforge-tray`).
fn tone_name(tone: Tone) -> &'static str {
    match tone {
        Tone::Light => "light",
        Tone::MediumLight => "medium-light",
        Tone::Medium => "medium",
        Tone::MediumDark => "medium-dark",
        Tone::Dark => "dark",
    }
}

fn tone_from_name(name: &str) -> Option<Tone> {
    match name {
        "light" => Some(Tone::Light),
        "medium-light" => Some(Tone::MediumLight),
        "medium" => Some(Tone::Medium),
        "medium-dark" => Some(Tone::MediumDark),
        "dark" => Some(Tone::Dark),
        _ => None,
    }
}

/// The on-disk shape. `tone` is `None` in the file itself for the
/// neutral default (no tone chosen) — distinct from the file not
/// existing at all, which [`load`] tells apart via [`ToneSetting`].
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
struct StoredConfig {
    /// One of [`tone_name`]'s strings, or absent for "neutral". An
    /// unrecognised string (a future tone this build does not know
    /// about, or a hand edit) is treated the same as absent — see
    /// [`load`] — rather than refusing to open the picker over one
    /// stale field.
    tone: Option<String>,
}

/// What [`load`] found, telling "nothing configured yet" apart from
/// "something is configured, and it's broken" — see this module's doc
/// for why collapsing the two is the mistake CLAUDE.md warns about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToneSetting {
    /// No file yet, or a file that explicitly says "neutral" — either
    /// way, show the grid's plain, untoned glyphs.
    Neutral,
    /// A previously chosen default tone.
    Tone(Tone),
    /// The file exists but could not be read or parsed — `main.rs` warns
    /// about this on stderr; the caller still falls back to
    /// [`ToneSetting::Neutral`] rather than refusing to open, the same
    /// way `look::resolve` carries on past a broken `appearance.toml`.
    Unreadable(String),
}

/// Loads the saved default tone from [`hyprforge_paths::emojimenu_toml_path`].
pub fn load() -> ToneSetting {
    load_from(&hyprforge_paths::emojimenu_toml_path())
}

/// [`load`] against an explicit `path` — the seam the tests use.
pub fn load_from(path: &Path) -> ToneSetting {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        // Distinguishing "not found" from every other read failure is
        // the whole point: a permissions error or a directory where a
        // file should be is not the same as first-run, and a user who
        // hits one deserves to hear about it rather than have this
        // picker silently act as if nothing were ever saved.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return ToneSetting::Neutral,
        Err(e) => return ToneSetting::Unreadable(e.to_string()),
    };

    let config: StoredConfig = match toml::from_str(&text) {
        Ok(config) => config,
        Err(e) => return ToneSetting::Unreadable(e.to_string()),
    };

    match config.tone.as_deref() {
        None => ToneSetting::Neutral,
        Some(name) => match tone_from_name(name) {
            Some(tone) => ToneSetting::Tone(tone),
            // An unrecognised value reads as neutral rather than as
            // "unreadable": the file parsed fine as TOML, this crate
            // just does not know what to do with this one field's
            // value (a stale name from a future tone, or a hand edit) —
            // that is closer to "nothing usable configured" than to "the
            // file is broken", and is worth a quieter fallback than
            // `Unreadable`'s stderr warning.
            None => ToneSetting::Neutral,
        },
    }
}

/// Saves `tone` (`None` for neutral) as the new default. Failures are
/// reported to the caller as a `String` rather than swallowed — a
/// picker that just asked the user to choose a tone and then silently
/// failed to remember it would be exactly the kind of surprise CLAUDE.md
/// warns "the design belongs in the design, not in a user's surprise"
/// about, even though there is no UI left open by the time this runs to
/// show the message in (the popup has already closed by the time
/// `finish` calls this — see `main.rs`) — so it goes to stderr instead.
pub fn save(tone: Option<Tone>) -> Result<(), String> {
    save_to(&hyprforge_paths::emojimenu_toml_path(), tone)
}

/// [`save`] against an explicit `path` — the seam the tests use.
pub fn save_to(path: &Path, tone: Option<Tone>) -> Result<(), String> {
    let config = StoredConfig { tone: tone.map(tone_name).map(str::to_string) };
    let text = toml::to_string(&config).map_err(|e| e.to_string())?;
    hyprforge_paths::write_atomic(path, &text).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_file_is_first_run_not_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("does-not-exist.toml");
        assert_eq!(load_from(&path), ToneSetting::Neutral);
    }

    #[test]
    fn saving_and_loading_a_tone_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("emojimenu.toml");
        save_to(&path, Some(Tone::MediumDark)).unwrap();
        assert_eq!(load_from(&path), ToneSetting::Tone(Tone::MediumDark));
    }

    #[test]
    fn saving_neutral_after_a_tone_was_chosen_clears_it() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("emojimenu.toml");
        save_to(&path, Some(Tone::Dark)).unwrap();
        save_to(&path, None).unwrap();
        assert_eq!(load_from(&path), ToneSetting::Neutral);
    }

    #[test]
    fn every_tone_round_trips_through_its_own_name() {
        for tone in hyprforge_emoji::TONES {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("emojimenu.toml");
            save_to(&path, Some(tone)).unwrap();
            assert_eq!(load_from(&path), ToneSetting::Tone(tone));
        }
    }

    /// The property CLAUDE.md is explicit about: a file that exists but
    /// will not parse must not read the same as "nothing configured".
    #[test]
    fn a_file_that_will_not_parse_is_reported_as_unreadable_not_as_first_run() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("emojimenu.toml");
        std::fs::write(&path, "this is not valid toml {{{").unwrap();
        assert!(matches!(load_from(&path), ToneSetting::Unreadable(_)));
    }

    /// A directory where the file should be is also "unreadable", not
    /// "first-run" — the same distinction, from a different underlying
    /// I/O error than `NotFound`.
    #[test]
    fn a_path_that_is_a_directory_is_unreadable_not_first_run() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("emojimenu.toml");
        std::fs::create_dir(&path).unwrap();
        assert!(matches!(load_from(&path), ToneSetting::Unreadable(_)));
    }

    /// A stale or hand-edited tone name is treated as neutral rather
    /// than flagged as unreadable — the file itself is perfectly valid
    /// TOML, this crate just does not recognise the one value in it.
    #[test]
    fn an_unrecognised_tone_name_reads_as_neutral() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("emojimenu.toml");
        std::fs::write(&path, "tone = \"chartreuse\"\n").unwrap();
        assert_eq!(load_from(&path), ToneSetting::Neutral);
    }

    #[test]
    fn an_absent_tone_field_reads_as_neutral() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("emojimenu.toml");
        std::fs::write(&path, "").unwrap();
        assert_eq!(load_from(&path), ToneSetting::Neutral);
    }

    #[test]
    fn saving_creates_the_parent_directory() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hyprforge").join("emojimenu.toml");
        save_to(&path, Some(Tone::Light)).unwrap();
        assert_eq!(load_from(&path), ToneSetting::Tone(Tone::Light));
    }
}
