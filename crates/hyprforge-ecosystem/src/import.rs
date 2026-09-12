//! Reading the daemons' own hand-written configs back into the models.
//!
//! Every screen here manages a *generated* file that the user's own config
//! sources. That arrangement keeps their comments and hand edits, but it
//! also means the app starts empty and shows them none of what they
//! already wrote — `night-light.toml` holding `profile = []` beside a
//! `hyprsunset.conf` that says `temperature = 6500`.
//!
//! **How wrong it goes differs per daemon, and that shapes everything
//! here.** hyprlang overwrites a repeated *variable* with the last one,
//! so a hand-written `temperature` is simply superseded by the generated
//! file: harmless. *Blocks* are categories and accumulate, so a
//! hand-written `listener` is not replaced by a generated one — both run.
//! This machine has five, which means saving the Idle tab without first
//! adopting them gives hypridle ten listeners, two of which lock the
//! screen.
//!
//! So adopting has to be able to retire the original lines, and retiring
//! a line is deleting a working setting unless the import understood all
//! of it. That is what every `dropped` field below is for, and why each
//! recovered item carries the lines it came from.

use crate::{idle, sunset, wallpaper};
use hyprforge_core::hyprlang::{Document, Item};

/// One recovered item, the lines it occupied, and what was not understood.
///
/// `dropped` is the whole point. These importers model a fixed set of
/// keys, the daemons accept more, and the caller *comments out the user's
/// original lines* once an item is adopted. Without a record of what was
/// not understood, a listener that dimmed the screen and restored the
/// keyboard backlight is adopted as one that only dims, and the lines
/// saying otherwise are retired — with a review screen showing a clean
/// import.
#[derive(Debug, Clone, PartialEq)]
pub struct Imported<T> {
    pub value: T,
    /// Keys present in the block that this importer does not model, in
    /// file order. Empty means it round-trips exactly.
    pub dropped: Vec<String>,
    /// 1-based, inclusive, into the file this came from.
    pub line: usize,
    pub end_line: usize,
}

impl<T> Imported<T> {
    /// Whether regenerating this reproduces what the user wrote.
    ///
    /// Only something that does may have its original lines retired — the
    /// same rule `windowrules::ImportedRule::is_faithful` applies, for the
    /// same reason.
    pub fn is_faithful(&self) -> bool {
        self.dropped.is_empty()
    }
}

/// Everything one config file yielded.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Review<T> {
    pub items: Vec<Imported<T>>,
    /// Lines the parser could not read at all. Never folded into "there
    /// was nothing here".
    pub problems: Vec<(usize, String)>,
}

impl<T> Review<T> {
    /// Whether every line of the file was understood, which is the only
    /// condition under which retiring the whole file is safe.
    pub fn is_complete(&self) -> bool {
        self.problems.is_empty() && self.items.iter().all(Imported::is_faithful)
    }
}

/// The assignments directly inside a block, as `(key, value, line)`.
fn fields(item: &Item) -> Vec<(&str, &str, usize)> {
    let Item::Block { items, .. } = item else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|i| match i {
            Item::Assignment { key, value, line } => Some((key.as_str(), value.as_str(), *line)),
            _ => None,
        })
        .collect()
}

/// Keys of a block that `known` does not list, in file order.
fn unknown(item: &Item, known: &[&str]) -> Vec<String> {
    fields(item)
        .into_iter()
        .filter(|(key, _, _)| !known.contains(key))
        .map(|(key, _, _)| key.to_string())
        .collect()
}

fn span(item: &Item) -> (usize, usize) {
    (item.line(), item.end_line())
}

/// `on-timeout` and `on-resume` are spelled with hyphens by hypridle,
/// while every key in `general` uses underscores. Both spellings are
/// accepted on the way in: a config written by hand may use either, and
/// refusing one would drop a listener that works.
const LISTENER_KEYS: &[&str] =
    &["timeout", "on-timeout", "on_timeout", "on-resume", "on_resume", "ignore_inhibit"];

const GENERAL_KEYS: &[&str] = &[
    "lock_cmd",
    "unlock_cmd",
    "before_sleep_cmd",
    "after_sleep_cmd",
    "on_lock_cmd",
    "on_unlock_cmd",
    "ignore_dbus_inhibit",
    "ignore_systemd_inhibit",
    "ignore_wayland_inhibit",
];

fn flag(value: &str) -> bool {
    matches!(value.trim(), "true" | "yes" | "1" | "on")
}

/// What a `hypridle.conf` says, as this app models it.
pub fn idle(doc: &Document) -> (Option<Imported<idle::General>>, Review<idle::Listener>) {
    // The last `general` block, not the first: it is a category hypridle
    // merges, so a later key overrides an earlier one and the last block
    // is the closest thing to "what is in effect".
    let general = doc.blocks("general").last().map(|item| {
        let mut general = idle::General::default();
        for (key, value, _) in fields(item) {
            match key {
                "ignore_dbus_inhibit" => general.ignore_dbus_inhibit = flag(value),
                "ignore_systemd_inhibit" => general.ignore_systemd_inhibit = flag(value),
                "ignore_wayland_inhibit" => general.ignore_wayland_inhibit = flag(value),
                _ => general.set_command(key, value.to_string()),
            }
        }
        let (line, end_line) = span(item);
        Imported { value: general, dropped: unknown(item, GENERAL_KEYS), line, end_line }
    });

    let mut review = Review { items: Vec::new(), problems: doc.problems.clone() };
    for item in doc.blocks("listener") {
        let mut listener = idle::Listener::default();
        let mut unreadable = Vec::new();
        for (key, value, line) in fields(item) {
            match key {
                "timeout" => match value.parse() {
                    Ok(seconds) => listener.timeout = seconds,
                    // Not a dropped *key* — the key is modelled and the
                    // value isn't a number. Recorded as a problem so the
                    // block is never retired on the strength of a timeout
                    // nobody can reproduce.
                    Err(_) => unreadable
                        .push((line, format!("`timeout = {value}` isn't a number of seconds"))),
                },
                "on-timeout" | "on_timeout" => listener.on_timeout = value.to_string(),
                "on-resume" | "on_resume" => listener.on_resume = value.to_string(),
                "ignore_inhibit" => listener.ignore_inhibit = flag(value),
                _ => {}
            }
        }
        let (line, end_line) = span(item);
        review.problems.extend(unreadable);
        review.items.push(Imported {
            value: listener,
            dropped: unknown(item, LISTENER_KEYS),
            line,
            end_line,
        });
    }
    (general, review)
}

const WALLPAPER_KEYS: &[&str] =
    &["monitor", "path", "fit_mode", "timeout", "order", "recursive"];

/// What a `hyprpaper.conf` says, as this app models it.
///
/// `preload` is read only to be ignored: this app deliberately doesn't
/// generate it (hyprpaper 0.8.4 no longer lists it among its IPC
/// requests), so it is not a key the model lost — it is one nothing needs.
/// It is still counted as *present*, because the line exists and retiring
/// it is a separate decision from adopting the block it supports.
pub fn wallpaper(doc: &Document) -> Review<wallpaper::Entry> {
    let mut review = Review { items: Vec::new(), problems: doc.problems.clone() };
    for item in doc.blocks("wallpaper") {
        let mut entry = wallpaper::Entry::default();
        for (key, value, line) in fields(item) {
            match key {
                "monitor" => entry.monitor = value.to_string(),
                "path" => entry.path = value.to_string(),
                "fit_mode" => match wallpaper::FitMode::parse(value) {
                    Some(mode) => entry.fit_mode = mode,
                    None => review
                        .problems
                        .push((line, format!("`fit_mode = {value}` isn't a fit mode"))),
                },
                "timeout" => match value.parse() {
                    Ok(seconds) => entry.timeout = Some(seconds),
                    Err(_) => review
                        .problems
                        .push((line, format!("`timeout = {value}` isn't a number of seconds"))),
                },
                "order" => entry.random_order = value.trim() == "random",
                "recursive" => entry.recursive = flag(value),
                _ => {}
            }
        }
        let (line, end_line) = span(item);
        review.items.push(Imported {
            value: entry,
            dropped: unknown(item, WALLPAPER_KEYS),
            line,
            end_line,
        });
    }
    review
}

const PROFILE_KEYS: &[&str] = &["time", "temperature", "gamma", "identity"];

/// What a `hyprsunset.conf` says, as this app models it.
///
/// A bare `temperature = 6500` with no `profile` block is hyprsunset's
/// startup value rather than a schedule. It becomes a profile at 00:00,
/// which is the same thing said in the model's vocabulary — the schedule
/// wraps, so one profile at midnight holds all day.
pub fn sunset(doc: &Document) -> (Option<i64>, Review<sunset::Profile>) {
    let max_gamma = doc.value("max-gamma").and_then(|v| v.parse().ok());
    let mut review = Review { items: Vec::new(), problems: doc.problems.clone() };

    for item in doc.blocks("profile") {
        let mut profile = sunset::Profile::default();
        for (key, value, line) in fields(item) {
            match key {
                "time" => profile.time = value.to_string(),
                "temperature" => match value.parse() {
                    Ok(kelvin) => profile.temperature = kelvin,
                    Err(_) => review
                        .problems
                        .push((line, format!("`temperature = {value}` isn't a number"))),
                },
                "gamma" => match value.parse() {
                    Ok(gamma) => profile.gamma = gamma,
                    Err(_) => {
                        review.problems.push((line, format!("`gamma = {value}` isn't a number")))
                    }
                },
                "identity" => profile.identity = flag(value),
                _ => {}
            }
        }
        let (line, end_line) = span(item);
        review.items.push(Imported {
            value: profile,
            dropped: unknown(item, PROFILE_KEYS),
            line,
            end_line,
        });
    }

    // Only when there is no schedule at all. A file with both is one
    // where the bare value is hyprsunset's starting point and the
    // profiles take over from it; adopting it as a sixth profile at
    // midnight would invent a schedule entry the user never wrote.
    if review.items.is_empty() {
        if let Some(item) = doc
            .items
            .iter()
            .rev()
            .find(|i| matches!(i, Item::Assignment { key, .. } if key == "temperature"))
        {
            let Item::Assignment { value, line, .. } = item else {
                unreachable!("filtered to assignments")
            };
            match value.parse() {
                Ok(temperature) => review.items.push(Imported {
                    value: sunset::Profile {
                        time: "00:00".to_string(),
                        temperature,
                        ..sunset::Profile::default()
                    },
                    dropped: Vec::new(),
                    line: *line,
                    end_line: *line,
                }),
                Err(_) => review
                    .problems
                    .push((*line, format!("`temperature = {value}` isn't a number"))),
            }
        }
    }
    (max_gamma, review)
}

#[cfg(test)]
mod tests {
    use super::*;
    use hyprforge_core::hyprlang::parse;

    /// The property that stops a reader and a writer drifting apart:
    /// anything this app generates must read back as what generated it.
    /// The same class of divergence as a generator and a matcher
    /// disagreeing, which has already been fixed twice in this crate.
    #[test]
    fn everything_the_idle_screen_writes_reads_back_as_itself() {
        let settings = idle::Settings {
            general: idle::General {
                lock_cmd: "hyprforge-lock".to_string(),
                before_sleep_cmd: "loginctl lock-session".to_string(),
                ignore_systemd_inhibit: true,
                ..idle::General::default()
            },
            listeners: vec![
                idle::Listener {
                    timeout: 150,
                    on_timeout: "brightnessctl -s set 10".to_string(),
                    on_resume: "brightnessctl -r".to_string(),
                    ignore_inhibit: false,
                },
                idle::Listener {
                    timeout: 1800,
                    on_timeout: "systemctl suspend".to_string(),
                    on_resume: String::new(),
                    ignore_inhibit: true,
                },
            ],
        };
        let (general, review) = idle(&parse(&idle::generate(&settings)));
        assert_eq!(review.problems, Vec::new());
        assert!(review.is_complete(), "{review:?}");
        assert_eq!(general.expect("a general block").value, settings.general);
        let listeners: Vec<_> = review.items.into_iter().map(|i| i.value).collect();
        assert_eq!(listeners, settings.listeners);
    }

    /// A real directory, because `render_one` only writes the cycling
    /// options for a path that *is* one — `is_directory` asks the
    /// filesystem. A made-up path would make this test pass while
    /// exercising none of `timeout`, `order` or `recursive`.
    #[test]
    fn everything_the_wallpaper_screen_writes_reads_back_as_itself() {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("rotation");
        std::fs::create_dir(&folder).unwrap();
        let settings = wallpaper::Settings {
            entries: vec![
                wallpaper::Entry {
                    monitor: String::new(),
                    path: "/w/a.png".to_string(),
                    fit_mode: wallpaper::FitMode::Contain,
                    ..wallpaper::Entry::default()
                },
                wallpaper::Entry {
                    monitor: "eDP-2".to_string(),
                    path: folder.display().to_string(),
                    timeout: Some(90),
                    random_order: true,
                    recursive: true,
                    ..wallpaper::Entry::default()
                },
            ],
            ..wallpaper::Settings::default()
        };
        let review = wallpaper(&parse(&wallpaper::generate(&settings)));
        assert!(review.is_complete(), "{review:?}");
        let entries: Vec<_> = review.items.into_iter().map(|i| i.value).collect();
        assert_eq!(entries, settings.entries);
    }

    #[test]
    fn everything_the_night_light_screen_writes_reads_back_as_itself() {
        let settings = sunset::Settings {
            max_gamma: Some(120),
            profiles: vec![
                sunset::Profile {
                    time: "06:00".to_string(),
                    temperature: 6500,
                    gamma: 1.0,
                    identity: false,
                },
                sunset::Profile {
                    time: "21:00".to_string(),
                    temperature: 4000,
                    gamma: 0.8,
                    identity: true,
                },
            ],
        };
        let (max_gamma, review) = sunset(&parse(&sunset::generate(&settings)));
        assert!(review.is_complete(), "{review:?}");
        assert_eq!(max_gamma, settings.max_gamma);
        let profiles: Vec<_> = review.items.into_iter().map(|i| i.value).collect();
        assert_eq!(profiles, settings.profiles);
    }

    /// The reason `dropped` exists. hypridle accepts keys this app does
    /// not model, and adopting a listener lets the caller retire the
    /// lines it came from — so a listener imported as "only dims" while
    /// its original said more must be refused, not quietly narrowed.
    #[test]
    fn a_listener_with_a_key_this_app_cannot_model_is_not_faithful() {
        let (_, review) = idle(&parse(
            "listener {\n    timeout = 5\n    on-timeout = x\n    something_new = 1\n}\n",
        ));
        let listener = &review.items[0];
        assert_eq!(listener.dropped, vec!["something_new"]);
        assert!(!listener.is_faithful());
        assert!(!review.is_complete());
    }

    /// A modelled key with an unreadable value is a *problem*, not a
    /// dropped key: the block must not be retired on the strength of a
    /// timeout nobody can reproduce.
    #[test]
    fn a_timeout_that_is_not_a_number_blocks_the_block_being_retired() {
        let (_, review) = idle(&parse("listener {\n    timeout = soon\n}\n"));
        assert_eq!(review.problems.len(), 1);
        assert!(review.problems[0].1.contains("isn't a number"), "{:?}", review.problems);
        assert!(!review.is_complete());
    }

    /// hypridle spells these with hyphens, but a hand-written config may
    /// use either. Refusing one would silently drop a listener that works.
    #[test]
    fn both_spellings_of_on_timeout_are_accepted() {
        for key in ["on-timeout", "on_timeout"] {
            let (_, review) = idle(&parse(&format!("listener {{\n  timeout = 5\n  {key} = x\n}}\n")));
            assert_eq!(review.items[0].value.on_timeout, "x", "{key}");
            assert!(review.items[0].is_faithful(), "{key} was treated as unknown");
        }
    }

    /// A bare `temperature` with no schedule is hyprsunset's startup
    /// value. It says the same thing as one profile at midnight, because
    /// the schedule wraps.
    #[test]
    fn a_bare_temperature_becomes_the_profile_that_holds_all_day() {
        let (_, review) = sunset(&parse("temperature = 6500\n"));
        assert_eq!(review.items.len(), 1);
        assert_eq!(review.items[0].value.time, "00:00");
        assert_eq!(review.items[0].value.temperature, 6500);
    }

    /// But only when there is no schedule. A file with both is one where
    /// the bare value is the starting point and the profiles take over;
    /// adopting it as an extra midnight profile would invent a schedule
    /// entry the user never wrote.
    #[test]
    fn a_bare_temperature_beside_a_schedule_is_not_invented_into_a_profile() {
        let (_, review) = sunset(&parse(
            "temperature = 6500\nprofile {\n    time = 21:00\n    temperature = 4000\n}\n",
        ));
        assert_eq!(review.items.len(), 1);
        assert_eq!(review.items[0].value.time, "21:00");
    }

    /// Blocks accumulate. Reading only the last would lose four of this
    /// machine's five listeners — and then retire all five lines.
    #[test]
    fn every_listener_is_recovered_with_the_lines_it_came_from() {
        let (_, review) = idle(&parse(
            "listener {\n  timeout = 1\n}\n\nlistener {\n  timeout = 2\n}\n",
        ));
        assert_eq!(review.items.len(), 2);
        assert_eq!((review.items[0].line, review.items[0].end_line), (1, 3));
        assert_eq!((review.items[1].line, review.items[1].end_line), (5, 7));
    }

    /// A file that will not parse must never read as a file with nothing
    /// in it, or adopting "nothing" and retiring the lines deletes a
    /// working config.
    #[test]
    fn a_malformed_file_is_never_complete() {
        let (_, review) = idle(&parse("listener {\n  timeout = 1\n"));
        assert!(!review.problems.is_empty());
        assert!(!review.is_complete());
    }
}
