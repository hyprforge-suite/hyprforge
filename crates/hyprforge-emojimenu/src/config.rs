//! The picker's own remembered state: the default skin tone, and which
//! emoji get picked most — what the "Frequently used" section is made
//! from.
//!
//! Stored at [`hyprforge_paths::emojimenu_toml_path`] — see that
//! function's own doc for why this crate's settings hang off the shared
//! `hyprforge` config directory rather than a directory of their own.
//!
//! # First-run is not an error, and an unreadable file is not first-run
//!
//! CLAUDE.md is explicit: "never collapse 'this file could not be read'
//! into 'there is nothing configured'." [`load`] follows
//! `hyprforge_core::hlconfig::storage`'s lead (and `look::resolve`'s) —
//! a file that does not exist yet is [`Stored::Fresh`] with nothing
//! printed (every user's very first run); a file that exists but will not
//! parse is [`Stored::Unreadable`], which `main.rs` warns about before
//! still opening with defaults.
//!
//! The other half of that rule matters more here than it did when this
//! file held one tone: the picker now *writes* after every pick, to count
//! it. Writing defaults over a file it could not read would quietly
//! replace whatever the user had with nothing — so a picker that started
//! from [`Stored::Unreadable`] never saves (see `main.rs`), and says so.

use hyprforge_emoji::Tone;
use serde::{Deserialize, Serialize};
use std::path::Path;

/// How many emoji the file remembers counts for. More than the section
/// shows (two rows), so an emoji that drops off the visible rows is not
/// forgotten the moment it does; bounded so the file cannot grow for
/// ever one pick at a time.
pub const REMEMBERED: usize = 36;

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

/// A tone's name in words, for the "default tone: medium" note.
pub fn tone_label(tone: Option<Tone>) -> &'static str {
    match tone {
        None => "none",
        Some(Tone::Light) => "light",
        Some(Tone::MediumLight) => "medium-light",
        Some(Tone::Medium) => "medium",
        Some(Tone::MediumDark) => "medium-dark",
        Some(Tone::Dark) => "dark",
    }
}

/// What the picker remembers between runs.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Prefs {
    /// The default tone, `None` for the plain neutral glyphs.
    pub tone: Option<Tone>,
    /// Emoji and how often each was picked, most-picked first. Keyed on
    /// the emoji's neutral glyph ([`hyprforge_emoji::Emoji::emoji`]), so
    /// picking 👍🏽 and 👍 counts towards the same thumbs-up.
    pub frequent: Vec<(String, u32)>,
}

impl Prefs {
    /// Counts one pick of `emoji` (its neutral glyph), keeping the list
    /// most-picked first and at most [`REMEMBERED`] long. Ties keep the
    /// most recent pick ahead, so a new favourite rises above an old one
    /// with the same count rather than sitting under it.
    pub fn count(&mut self, emoji: &str) {
        let count = match self.frequent.iter().position(|(e, _)| e == emoji) {
            Some(i) => self.frequent.remove(i).1.saturating_add(1),
            None => 1,
        };
        let at = self.frequent.iter().position(|(_, c)| *c <= count).unwrap_or(self.frequent.len());
        self.frequent.insert(at, (emoji.to_string(), count));
        self.frequent.truncate(REMEMBERED);
    }
}

/// The on-disk shape.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
struct StoredConfig {
    /// One of [`tone_name`]'s strings, or absent for neutral. An
    /// unrecognised string (a future tone, a hand edit) reads as absent
    /// rather than refusing to open the picker over one stale field.
    tone: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    frequent: Vec<StoredCount>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
struct StoredCount {
    emoji: String,
    count: u32,
}

/// What [`load`] found — see the module doc for why these three must
/// never read the same to a caller.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Stored {
    /// No file yet: first run.
    Fresh,
    Found(Prefs),
    /// The file exists but could not be read or parsed.
    Unreadable(String),
}

pub fn load() -> Stored {
    load_from(&hyprforge_paths::emojimenu_toml_path())
}

/// [`load`] against an explicit `path` — the seam the tests use.
pub fn load_from(path: &Path) -> Stored {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        // Telling "not found" from every other failure is the whole
        // point: a permissions error or a directory where the file should
        // be is not first-run.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Stored::Fresh,
        Err(e) => return Stored::Unreadable(e.to_string()),
    };
    let config: StoredConfig = match toml::from_str(&text) {
        Ok(config) => config,
        Err(e) => return Stored::Unreadable(e.to_string()),
    };
    let mut frequent: Vec<(String, u32)> =
        config.frequent.into_iter().filter(|c| !c.emoji.is_empty() && c.count > 0).map(|c| (c.emoji, c.count)).collect();
    // A hand-edited file need not be in order; everything reading this
    // assumes it is.
    frequent.sort_by_key(|entry| std::cmp::Reverse(entry.1));
    frequent.truncate(REMEMBERED);
    Stored::Found(Prefs { tone: config.tone.as_deref().and_then(tone_from_name), frequent })
}

/// Saves `prefs`. Failures come back as a `String` for stderr — there is
/// no UI left open to show them in by the time this runs.
pub fn save(prefs: &Prefs) -> Result<(), String> {
    save_to(&hyprforge_paths::emojimenu_toml_path(), prefs)
}

/// [`save`] against an explicit `path` — the seam the tests use.
pub fn save_to(path: &Path, prefs: &Prefs) -> Result<(), String> {
    let config = StoredConfig {
        tone: prefs.tone.map(tone_name).map(str::to_string),
        frequent: prefs.frequent.iter().map(|(emoji, count)| StoredCount { emoji: emoji.clone(), count: *count }).collect(),
    };
    let text = toml::to_string(&config).map_err(|e| e.to_string())?;
    hyprforge_paths::write_atomic(path, &text).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with_tone(tone: Option<Tone>) -> Prefs {
        Prefs { tone, ..Prefs::default() }
    }

    #[test]
    fn a_missing_file_is_first_run_not_an_error() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(load_from(&dir.path().join("does-not-exist.toml")), Stored::Fresh);
    }

    #[test]
    fn saving_and_loading_round_trips_the_tone_and_the_counts() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("emojimenu.toml");
        let prefs = Prefs { tone: Some(Tone::MediumDark), frequent: vec![("😂".into(), 5), ("👍".into(), 2)] };
        save_to(&path, &prefs).unwrap();
        assert_eq!(load_from(&path), Stored::Found(prefs));
    }

    #[test]
    fn saving_neutral_after_a_tone_was_chosen_clears_it() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("emojimenu.toml");
        save_to(&path, &with_tone(Some(Tone::Dark))).unwrap();
        save_to(&path, &with_tone(None)).unwrap();
        assert_eq!(load_from(&path), Stored::Found(with_tone(None)));
    }

    #[test]
    fn every_tone_round_trips_through_its_own_name() {
        for tone in hyprforge_emoji::TONES {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("emojimenu.toml");
            save_to(&path, &with_tone(Some(tone))).unwrap();
            assert_eq!(load_from(&path), Stored::Found(with_tone(Some(tone))));
        }
    }

    /// The property CLAUDE.md is explicit about: a file that exists but
    /// will not parse must not read the same as "nothing configured".
    #[test]
    fn a_file_that_will_not_parse_is_unreadable_not_first_run() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("emojimenu.toml");
        std::fs::write(&path, "this is not valid toml {{{").unwrap();
        assert!(matches!(load_from(&path), Stored::Unreadable(_)));
    }

    #[test]
    fn a_path_that_is_a_directory_is_unreadable_not_first_run() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("emojimenu.toml");
        std::fs::create_dir(&path).unwrap();
        assert!(matches!(load_from(&path), Stored::Unreadable(_)));
    }

    #[test]
    fn an_unrecognised_tone_name_reads_as_neutral() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("emojimenu.toml");
        std::fs::write(&path, "tone = \"chartreuse\"\n").unwrap();
        assert_eq!(load_from(&path), Stored::Found(with_tone(None)));
    }

    /// A file written before counts existed — `tone` alone — still loads.
    #[test]
    fn a_file_from_before_counts_existed_still_loads() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("emojimenu.toml");
        std::fs::write(&path, "tone = \"light\"\n").unwrap();
        assert_eq!(load_from(&path), Stored::Found(with_tone(Some(Tone::Light))));
    }

    #[test]
    fn a_hand_edited_file_is_put_back_in_order() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("emojimenu.toml");
        std::fs::write(&path, "[[frequent]]\nemoji = \"a\"\ncount = 1\n\n[[frequent]]\nemoji = \"b\"\ncount = 9\n").unwrap();
        let Stored::Found(prefs) = load_from(&path) else { panic!("should load") };
        assert_eq!(prefs.frequent[0].0, "b");
    }

    #[test]
    fn saving_creates_the_parent_directory() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hyprforge").join("emojimenu.toml");
        save_to(&path, &with_tone(Some(Tone::Light))).unwrap();
        assert_eq!(load_from(&path), Stored::Found(with_tone(Some(Tone::Light))));
    }

    // --- counting

    #[test]
    fn picking_something_again_moves_it_above_what_it_now_outnumbers() {
        let mut prefs = Prefs::default();
        prefs.count("a");
        prefs.count("b");
        prefs.count("b");
        assert_eq!(prefs.frequent, vec![("b".into(), 2), ("a".into(), 1)]);
    }

    #[test]
    fn a_tie_puts_the_latest_pick_first() {
        let mut prefs = Prefs::default();
        prefs.count("a");
        prefs.count("b");
        assert_eq!(prefs.frequent[0].0, "b");
    }

    #[test]
    fn the_list_never_grows_past_what_it_remembers() {
        let mut prefs = Prefs::default();
        for i in 0..REMEMBERED * 2 {
            prefs.count(&i.to_string());
        }
        assert_eq!(prefs.frequent.len(), REMEMBERED);
    }
}
