//! Wallpaper, colour temperature and idle behaviour — the Hypr ecosystem
//! daemons.
//!
//! Three tabs rather than three screens: they are one family (separate
//! daemons, hyprlang configs, joined by a `source =` line) and a user
//! thinks of them as "how my desktop behaves when I'm not touching it".
//!
//! The one thing this screen must not smooth over is that a change
//! reaches each daemon differently. hyprpaper and hyprsunset take it
//! live; **hypridle has no IPC at all** and does nothing until it
//! restarts. Every save reports which of those happened rather than
//! saying "Saved." at all three.

use hyprforge_ui::theme::{spacing, FontScale};
use hyprforge_ui::widgets::{
    danger_button, divider, meta_text, primary_button, scaled_text, secondary_button, section,
};
use crate::module::SettingsModule;
use hyprforge_ecosystem::apply::{self, Applied};
use hyprforge_ecosystem::{idle, sunset, wallpaper};
use iced::widget::{checkbox, column, container, pick_list, row, scrollable, text_input};
use iced::{Element, Length, Task};
use std::collections::BTreeMap;
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Wallpaper,
    NightLight,
    Idle,
}

impl Tab {
    const ALL: [Tab; 3] = [Tab::Wallpaper, Tab::NightLight, Tab::Idle];

    fn label(self) -> &'static str {
        match self {
            Tab::Wallpaper => "Wallpaper",
            Tab::NightLight => "Night light",
            Tab::Idle => "Idle",
        }
    }
}

/// Which stored list a message is about. The three tabs each hold a list
/// of blocks and every list needs the same four operations, so they share
/// the messages rather than repeating them.
impl Field {
    /// Which tab's list this field belongs to.
    ///
    /// Needed because drafts from all three tabs share one map: without
    /// it, applying would save only the tab that happened to be open, and
    /// removing a row would drop the wrong rows' drafts.
    fn tab(self) -> Tab {
        match self {
            Field::Monitor
            | Field::Path
            | Field::FitMode
            | Field::Timeout
            | Field::RandomOrder
            | Field::Recursive => Tab::Wallpaper,
            Field::Time | Field::Temperature | Field::Gamma | Field::Identity => Tab::NightLight,
            Field::IdleTimeout
            | Field::OnTimeout
            | Field::OnResume
            | Field::IgnoreInhibit => Tab::Idle,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Field {
    Monitor,
    Path,
    FitMode,
    Timeout,
    RandomOrder,
    Recursive,
    Time,
    Temperature,
    Gamma,
    Identity,
    IdleTimeout,
    OnTimeout,
    OnResume,
    IgnoreInhibit,
}

#[derive(Debug, Clone)]
pub enum Message {
    TabSelected(Tab),
    Loaded(Loaded),
    // Lists
    WallpaperAdded,
    WallpaperRemoved(usize),
    WallpaperChanged(usize, Field, String),
    WallpaperToggled(usize, Field, bool),
    ProfileAdded,
    ProfileRemoved(usize),
    ProfileChanged(usize, Field, String),
    ProfileToggled(usize, Field, bool),
    ListenerAdded,
    ListenerRemoved(usize),
    ListenerChanged(usize, Field, String),
    ListenerToggled(usize, Field, bool),
    // General
    GeneralCommandChanged(&'static str, String),
    InhibitToggled(&'static str, bool),
    SplashToggled(bool),
    Commit,
    RestartIdle,
    Applied(Tab, Result<Applied, String>),
}

/// Everything read from the system when the screen opens.
#[derive(Debug, Clone, Default)]
pub struct Loaded {
    pub images: Vec<String>,
    pub monitors: Vec<String>,
}

pub struct DesktopModule {
    tab: Tab,
    wallpapers: wallpaper::Settings,
    sunset: sunset::Settings,
    idle: idle::Settings,
    /// Text mid-edit, keyed by `(tab, index, field)`. Held rather than
    /// committed per keystroke: every save rewrites a config file and
    /// pokes a daemon.
    drafts: BTreeMap<(usize, Field), String>,
    images: Vec<String>,
    monitors: Vec<String>,
    error: Option<String>,
    status: Option<String>,
    /// Set while the idle config on disk is ahead of the running daemon.
    idle_needs_restart: bool,
    store_unreadable: Option<String>,
}

impl DesktopModule {
    pub fn new() -> (Self, Task<Message>) {
        // A failure here must never look like "you've configured
        // nothing": that reading is what turns one bad parse into a wiped
        // store on the next save.
        let mut unreadable = Vec::new();
        let wallpapers = load_or_note(&wallpaper_toml(), &mut unreadable);
        let sunset = load_or_note(&sunset_toml(), &mut unreadable);
        let idle = load_or_note(&idle_toml(), &mut unreadable);
        (
            DesktopModule {
                tab: Tab::Wallpaper,
                wallpapers,
                sunset,
                idle,
                drafts: BTreeMap::new(),
                images: Vec::new(),
                monitors: Vec::new(),
                error: None,
                status: None,
                idle_needs_restart: false,
                store_unreadable: (!unreadable.is_empty()).then(|| unreadable.join("; ")),
            },
            Task::perform(load_system(), Message::Loaded),
        )
    }

    fn draft(&self, index: usize, field: Field, current: impl std::fmt::Display) -> String {
        self.drafts
            .get(&(index, field))
            .cloned()
            .unwrap_or_else(|| current.to_string())
    }

    /// Writes the tab's canonical TOML, then applies it.
    ///
    /// Nothing irreversible runs unless the write returned `Ok` — the
    /// ordering rule that exists because doing it the other way round
    /// cost a real user 37 hand-written binds.
    fn save(&mut self, tab: Tab) -> Task<Message> {
        if let Some(reason) = &self.store_unreadable {
            self.error = Some(format!(
                "Not saving — a settings file couldn't be read, and overwriting \
                 it would lose whatever is in it. ({reason})"
            ));
            return Task::none();
        }
        let hypr = hyprforge_core::paths::hypr_config_dir();
        let generated_dir = hyprforge_core::paths::hypr_hyprforge_dir();
        let result = match tab {
            Tab::Wallpaper => hyprforge_ecosystem::storage::save(&wallpaper_toml(), &self.wallpapers),
            Tab::NightLight => hyprforge_ecosystem::storage::save(&sunset_toml(), &self.sunset),
            Tab::Idle => hyprforge_ecosystem::storage::save(&idle_toml(), &self.idle),
        };
        // The wallpaper is also the auth screens' background, so saving
        // it has to reach them too. Doing this only in Appearance is how
        // the lock screen ends up showing last week's wallpaper.
        if result.is_ok() && tab == Tab::Wallpaper {
            crate::look::republish();
        }
        if let Err(e) = result {
            self.error = Some(e.to_string());
            return Task::none();
        }
        self.status = None;
        match tab {
            Tab::Wallpaper => {
                let settings = self.wallpapers.clone();
                Task::perform(
                    apply_wallpapers(generated_dir.join("wallpaper.conf"), hypr.join("hyprpaper.conf"), settings),
                    move |r| Message::Applied(tab, r),
                )
            }
            Tab::NightLight => {
                let settings = self.sunset.clone();
                Task::perform(
                    apply_sunset(generated_dir.join("sunset.conf"), hypr.join("hyprsunset.conf"), settings),
                    move |r| Message::Applied(tab, r),
                )
            }
            Tab::Idle => {
                let settings = self.idle.clone();
                Task::perform(
                    apply_idle(generated_dir.join("idle.conf"), hypr.join("hypridle.conf"), settings),
                    move |r| Message::Applied(tab, r),
                )
            }
        }
    }

    /// Parses every pending draft into the store, then saves.
    ///
    /// A field that doesn't parse keeps what was typed and reports it;
    /// the ones that do parse still apply, so one typo doesn't discard
    /// everything else.
    fn commit(&mut self) -> Task<Message> {
        let pending: Vec<((usize, Field), String)> =
            self.drafts.iter().map(|(k, v)| (*k, v.clone())).collect();
        let mut bad = Vec::new();
        let mut touched = Vec::new();
        for ((index, field), raw) in pending {
            if self.apply_draft(index, field, &raw) {
                self.drafts.remove(&(index, field));
                if !touched.contains(&field.tab()) {
                    touched.push(field.tab());
                }
            } else {
                bad.push(raw.trim().to_string());
            }
        }
        self.error = (!bad.is_empty())
            .then(|| format!("Couldn't read: {}. Everything else was saved.", bad.join(", ")));
        // Every tab that changed, not just the one on screen. Typing in
        // Wallpaper, switching to Idle and pressing Apply would otherwise
        // leave the wallpaper edit in memory and never written.
        Task::batch(touched.into_iter().map(|tab| self.save(tab)).collect::<Vec<_>>())
    }

    /// Drops the drafts belonging to a removed row, and shifts the ones
    /// after it down.
    ///
    /// Removing row 1 makes row 2 become row 1, so a draft keyed to index
    /// 2 would reappear against a different row's values. Only this tab's
    /// fields move — the three lists share one map but have separate
    /// indices.
    fn reindex_drafts(&mut self, tab: Tab, removed: usize) {
        let moved: Vec<((usize, Field), String)> = self
            .drafts
            .iter()
            .filter(|((index, field), _)| field.tab() == tab && *index > removed)
            .map(|((index, field), value)| ((*index - 1, *field), value.clone()))
            .collect();
        self.drafts
            .retain(|(index, field), _| field.tab() != tab || *index < removed);
        self.drafts.extend(moved);
    }

    /// `true` if the value was understood and stored.
    fn apply_draft(&mut self, index: usize, field: Field, raw: &str) -> bool {
        let text = raw.trim().to_string();
        match field {
            Field::Monitor | Field::Path => {
                let Some(e) = self.wallpapers.entries.get_mut(index) else {
                    return true;
                };
                if field == Field::Monitor {
                    e.monitor = text;
                } else {
                    e.path = text;
                }
                true
            }
            Field::Timeout => {
                let Some(e) = self.wallpapers.entries.get_mut(index) else {
                    return true;
                };
                if text.is_empty() {
                    e.timeout = None;
                    return true;
                }
                match text.parse::<u32>() {
                    Ok(v) if v > 0 => {
                        e.timeout = Some(v);
                        true
                    }
                    _ => false,
                }
            }
            Field::Time => {
                let Some(p) = self.sunset.profiles.get_mut(index) else {
                    return true;
                };
                if sunset::parse_time(&text).is_none() {
                    return false;
                }
                p.time = text;
                true
            }
            Field::Temperature => {
                let Some(p) = self.sunset.profiles.get_mut(index) else {
                    return true;
                };
                match text.parse::<i64>() {
                    Ok(v) if (sunset::MIN_TEMPERATURE..=sunset::MAX_TEMPERATURE).contains(&v) => {
                        p.temperature = v;
                        true
                    }
                    _ => false,
                }
            }
            Field::Gamma => {
                let Some(p) = self.sunset.profiles.get_mut(index) else {
                    return true;
                };
                match text.parse::<f64>() {
                    Ok(v) if v.is_finite() && v > 0.0 => {
                        p.gamma = v;
                        true
                    }
                    _ => false,
                }
            }
            Field::IdleTimeout => {
                let Some(l) = self.idle.listeners.get_mut(index) else {
                    return true;
                };
                match text.parse::<u32>() {
                    Ok(v) if v > 0 => {
                        l.timeout = v;
                        true
                    }
                    _ => false,
                }
            }
            Field::OnTimeout | Field::OnResume => {
                let Some(l) = self.idle.listeners.get_mut(index) else {
                    return true;
                };
                // Verbatim, not trimmed of interior anything: these are
                // shell commands and Hyprforge never runs them itself.
                if field == Field::OnTimeout {
                    l.on_timeout = raw.trim().to_string();
                } else {
                    l.on_resume = raw.trim().to_string();
                }
                true
            }
            _ => true,
        }
    }
}

impl SettingsModule for DesktopModule {
    type Message = Message;


    fn icon(&self) -> &'static str {
        "🖼"
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::TabSelected(tab) => {
                self.tab = tab;
                Task::none()
            }
            Message::Loaded(loaded) => {
                self.images = loaded.images;
                self.monitors = loaded.monitors;
                Task::none()
            }
            Message::WallpaperAdded => {
                // A new entry starts as the fallback with no image, which
                // `invalid()` reports until it's given one — better than
                // inventing a path the user didn't choose.
                self.wallpapers.entries.push(wallpaper::Entry::default());
                Task::none()
            }
            Message::WallpaperRemoved(i) => {
                if i < self.wallpapers.entries.len() {
                    self.wallpapers.entries.remove(i);
                    self.reindex_drafts(Tab::Wallpaper, i);
                }
                self.save(Tab::Wallpaper)
            }
            Message::WallpaperChanged(i, field, value) => {
                if field == Field::FitMode {
                    if let (Some(e), Some(mode)) = (
                        self.wallpapers.entries.get_mut(i),
                        wallpaper::FitMode::parse(&value),
                    ) {
                        e.fit_mode = mode;
                        return self.save(Tab::Wallpaper);
                    }
                    return Task::none();
                }
                // A picked path or monitor is a complete decision, so it
                // saves; typed text waits for Apply.
                if matches!(field, Field::Path | Field::Monitor) {
                    self.apply_draft(i, field, &value);
                    self.drafts.remove(&(i, field));
                    return self.save(Tab::Wallpaper);
                }
                self.drafts.insert((i, field), value);
                Task::none()
            }
            Message::WallpaperToggled(i, field, on) => {
                if let Some(e) = self.wallpapers.entries.get_mut(i) {
                    match field {
                        Field::RandomOrder => e.random_order = on,
                        Field::Recursive => e.recursive = on,
                        _ => {}
                    }
                }
                self.save(Tab::Wallpaper)
            }
            Message::SplashToggled(on) => {
                self.wallpapers.splash = Some(on);
                self.save(Tab::Wallpaper)
            }
            Message::ProfileAdded => {
                self.sunset.profiles.push(sunset::Profile::default());
                Task::none()
            }
            Message::ProfileRemoved(i) => {
                if i < self.sunset.profiles.len() {
                    self.sunset.profiles.remove(i);
                    self.reindex_drafts(Tab::NightLight, i);
                }
                self.save(Tab::NightLight)
            }
            Message::ProfileChanged(i, field, value) => {
                self.drafts.insert((i, field), value);
                Task::none()
            }
            Message::ProfileToggled(i, field, on) => {
                if let (Some(p), Field::Identity) = (self.sunset.profiles.get_mut(i), field) {
                    p.identity = on;
                }
                self.save(Tab::NightLight)
            }
            Message::ListenerAdded => {
                self.idle.listeners.push(idle::Listener {
                    timeout: 300,
                    ..idle::Listener::default()
                });
                Task::none()
            }
            Message::ListenerRemoved(i) => {
                if i < self.idle.listeners.len() {
                    self.idle.listeners.remove(i);
                    self.reindex_drafts(Tab::Idle, i);
                }
                self.save(Tab::Idle)
            }
            Message::ListenerChanged(i, field, value) => {
                self.drafts.insert((i, field), value);
                Task::none()
            }
            Message::ListenerToggled(i, field, on) => {
                if let (Some(l), Field::IgnoreInhibit) = (self.idle.listeners.get_mut(i), field) {
                    l.ignore_inhibit = on;
                }
                self.save(Tab::Idle)
            }
            Message::GeneralCommandChanged(field, value) => {
                self.idle.general.set_command(field, value);
                Task::none()
            }
            Message::InhibitToggled(field, on) => {
                match field {
                    "ignore_dbus_inhibit" => self.idle.general.ignore_dbus_inhibit = on,
                    "ignore_systemd_inhibit" => self.idle.general.ignore_systemd_inhibit = on,
                    "ignore_wayland_inhibit" => self.idle.general.ignore_wayland_inhibit = on,
                    _ => {}
                }
                self.save(Tab::Idle)
            }
            Message::Commit => self.commit(),
            Message::RestartIdle => {
                match apply::restart_idle() {
                    Ok(()) => {
                        self.idle_needs_restart = false;
                        self.status = Some("hypridle restarted — your idle settings are live.".into());
                        self.error = None;
                    }
                    Err(e) => self.error = Some(format!("Couldn't restart hypridle: {e}")),
                }
                Task::none()
            }
            Message::Applied(tab, Ok(applied)) => {
                self.error = None;
                self.idle_needs_restart =
                    tab == Tab::Idle && applied == Applied::NeedsRestart;
                match applied {
                    // Refusals are an error, not a status: the setting is
                    // saved but the screen did not change, and calling
                    // that "applied" is the failure this project keeps
                    // meeting.
                    Applied::PartlyRefused(refused) => {
                        self.status = None;
                        self.error = Some(format!(
                            "Saved, but the daemon wouldn't take: {}. \
                             Check the file still exists and is readable.",
                            refused.join(", ")
                        ));
                    }
                    Applied::Live => self.status = Some("Saved and applied.".into()),
                    Applied::NeedsRestart => {
                        self.status = Some(
                            "Saved. hypridle has no way to be told — restart it below to apply."
                                .into(),
                        )
                    }
                    Applied::DaemonNotRunning => {
                        self.status = Some(
                            "Saved. The daemon isn't running, so nothing changed on screen yet."
                                .into(),
                        )
                    }
                }
                Task::none()
            }
            Message::Applied(_, Err(e)) => {
                self.status = None;
                self.error = Some(e);
                Task::none()
            }
        }
    }

    fn view(&self, scale: FontScale) -> Element<'_, Message> {
        let mut content = column![].spacing(spacing::LG).width(Length::Fill);

        if let Some(reason) = &self.store_unreadable {
            content = content.push(section(
                "A settings file couldn't be read",
                scale,
                column![
                    scaled_text(
                        "Nothing will be saved until this is fixed — writing over it \
                         would lose whatever it contains.",
                        13.0,
                        scale,
                    ),
                    meta_text(reason.clone(), 12.0, scale),
                ]
                .spacing(spacing::SM),
            ));
        }
        if let Some(e) = &self.error {
            content = content.push(section(
                "Something went wrong",
                scale,
                scaled_text(e.clone(), 13.0, scale),
            ));
        }
        if let Some(s) = &self.status {
            content = content.push(meta_text(s.clone(), 12.0, scale));
        }

        let mut tabs = row![].spacing(spacing::SM);
        for tab in Tab::ALL {
            let button = if tab == self.tab {
                primary_button(tab.label())
            } else {
                secondary_button(tab.label())
            };
            tabs = tabs.push(button.on_press(Message::TabSelected(tab)));
        }
        content = content.push(tabs);

        if !self.drafts.is_empty() {
            content = content.push(
                row![
                    scaled_text(
                        format!("{} field(s) typed but not applied", self.drafts.len()),
                        13.0,
                        scale,
                    ),
                    primary_button("Apply").on_press(Message::Commit),
                ]
                .spacing(spacing::MD)
                .align_y(iced::Alignment::Center),
            );
        }

        content = match self.tab {
            Tab::Wallpaper => content.push(self.wallpaper_view(scale)),
            Tab::NightLight => content.push(self.sunset_view(scale)),
            Tab::Idle => self.idle_view(content, scale),
        };

        scrollable(container(content).padding(spacing::LG))
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    }
}

impl DesktopModule {
    fn wallpaper_view(&self, scale: FontScale) -> Element<'_, Message> {
        let problems = self.wallpapers.invalid();
        let mut body = column![meta_text(
            "A wallpaper with no monitor is the fallback: it covers every screen \
             that hasn't got one of its own, which is what makes a setup survive \
             plugging a different display in.",
            12.0,
            scale,
        )]
        .spacing(spacing::SM);

        for (i, entry) in self.wallpapers.entries.iter().enumerate() {
            body = body.push(divider());
            let mut fields = column![].spacing(spacing::SM);

            let monitors: Vec<String> = std::iter::once(ALL_MONITORS.to_string())
                .chain(self.monitors.iter().cloned())
                .collect();
            let selected = Some(if entry.is_fallback() {
                ALL_MONITORS.to_string()
            } else {
                entry.monitor.clone()
            });
            fields = fields.push(labelled(
                "Screen",
                pick_list(monitors, selected, move |choice: String| {
                    let value = if choice == ALL_MONITORS { String::new() } else { choice };
                    Message::WallpaperChanged(i, Field::Monitor, value)
                })
                .into(),
                scale,
            ));

            let images = options_including(&self.images, &entry.path);
            fields = fields.push(labelled(
                "Image or folder",
                pick_list(images, Some(entry.path.clone()).filter(|p| !p.is_empty()), move |choice: String| {
                    Message::WallpaperChanged(i, Field::Path, choice)
                })
                .into(),
                scale,
            ));

            let modes: Vec<String> = wallpaper::FitMode::ALL.iter().map(|m| m.to_string()).collect();
            fields = fields.push(labelled(
                "Fit",
                pick_list(modes, Some(entry.fit_mode.to_string()), move |choice: String| {
                    Message::WallpaperChanged(i, Field::FitMode, choice)
                })
                .into(),
                scale,
            ));

            // Only for a folder: these options mean nothing for one image,
            // and showing them would suggest a single image cycles.
            if entry.is_directory() {
                fields = fields.push(labelled(
                    "Change every (seconds)",
                    text_input("30", &self.draft(i, Field::Timeout, entry.timeout.map(|t| t.to_string()).unwrap_or_default()))
                        .on_input(move |v| Message::WallpaperChanged(i, Field::Timeout, v))
                        .on_submit(Message::Commit)
                        .padding(spacing::SM)
                        .width(Length::Fixed(90.0))
                        .into(),
                    scale,
                ));
                fields = fields.push(
                    row![
                        checkbox(entry.random_order)
                            .on_toggle(move |v| Message::WallpaperToggled(i, Field::RandomOrder, v)),
                        scaled_text("Shuffle", 13.0, scale),
                        checkbox(entry.recursive)
                            .on_toggle(move |v| Message::WallpaperToggled(i, Field::Recursive, v)),
                        scaled_text("Include subfolders", 13.0, scale),
                    ]
                    .spacing(spacing::SM)
                    .align_y(iced::Alignment::Center),
                );
            }

            if let Some((_, problem)) = problems.iter().find(|(index, _)| *index == i) {
                fields = fields.push(scaled_text(problem.clone(), 12.0, scale));
            }
            fields = fields.push(danger_button("Remove", Message::WallpaperRemoved(i)));
            body = body.push(fields);
        }

        body = body.push(divider());
        body = body.push(
            row![
                secondary_button("Add a wallpaper").on_press(Message::WallpaperAdded),
                checkbox(self.wallpapers.splash.unwrap_or(true))
                    .on_toggle(Message::SplashToggled),
                scaled_text("Show the Hyprland splash", 13.0, scale),
            ]
            .spacing(spacing::MD)
            .align_y(iced::Alignment::Center),
        );
        section("Wallpaper", scale, body)
    }

    fn sunset_view(&self, scale: FontScale) -> Element<'_, Message> {
        let problems = self.sunset.invalid();
        let mut body = column![meta_text(
            "Each entry holds from its time until the next one. Lower temperatures \
             are warmer; 6500K is neutral daylight.",
            12.0,
            scale,
        )]
        .spacing(spacing::SM);

        if self.sunset.has_midnight_gap() {
            body = body.push(scaled_text(
                "No entry starts at 00:00, so the last one of the day carries over \
                 into the morning. Add one at 00:00 if that isn't what you want.",
                13.0,
                scale,
            ));
        }

        for (i, profile) in self.sunset.profiles.iter().enumerate() {
            body = body.push(divider());
            let mut fields = column![].spacing(spacing::SM);
            fields = fields.push(labelled(
                "From",
                text_input("21:00", &self.draft(i, Field::Time, &profile.time))
                    .on_input(move |v| Message::ProfileChanged(i, Field::Time, v))
                    .on_submit(Message::Commit)
                    .padding(spacing::SM)
                    .width(Length::Fixed(90.0))
                    .into(),
                scale,
            ));
            fields = fields.push(labelled(
                "Temperature (K)",
                text_input("6500", &self.draft(i, Field::Temperature, profile.temperature))
                    .on_input(move |v| Message::ProfileChanged(i, Field::Temperature, v))
                    .on_submit(Message::Commit)
                    .padding(spacing::SM)
                    .width(Length::Fixed(90.0))
                    .into(),
                scale,
            ));
            fields = fields.push(labelled(
                "Brightness",
                text_input("1.0", &self.draft(i, Field::Gamma, profile.gamma))
                    .on_input(move |v| Message::ProfileChanged(i, Field::Gamma, v))
                    .on_submit(Message::Commit)
                    .padding(spacing::SM)
                    .width(Length::Fixed(90.0))
                    .into(),
                scale,
            ));
            fields = fields.push(
                row![
                    checkbox(profile.identity)
                        .on_toggle(move |v| Message::ProfileToggled(i, Field::Identity, v)),
                    scaled_text("Brightness only, no colour shift", 13.0, scale),
                ]
                .spacing(spacing::SM)
                .align_y(iced::Alignment::Center),
            );
            if let Some((_, problem)) = problems.iter().find(|(index, _)| *index == i) {
                fields = fields.push(scaled_text(problem.clone(), 12.0, scale));
            }
            fields = fields.push(danger_button("Remove", Message::ProfileRemoved(i)));
            body = body.push(fields);
        }

        body = body.push(divider());
        body = body.push(secondary_button("Add a time").on_press(Message::ProfileAdded));
        section("Night light", scale, body)
    }

    fn idle_view<'a>(
        &'a self,
        mut content: iced::widget::Column<'a, Message>,
        scale: FontScale,
    ) -> iced::widget::Column<'a, Message> {
        // The honest bit. hypridle has no IPC, so a save here genuinely
        // does nothing until it restarts.
        if self.idle_needs_restart {
            content = content.push(section(
                "Restart needed",
                scale,
                column![
                    scaled_text(
                        "hypridle has no way to be told about a config change, so your \
                         saved settings aren't running yet.",
                        13.0,
                        scale,
                    ),
                    meta_text(
                        "Restarting it briefly stops idle tracking — it won't lock or \
                         dim during the moment it takes.",
                        12.0,
                        scale,
                    ),
                    primary_button("Restart hypridle").on_press(Message::RestartIdle),
                ]
                .spacing(spacing::SM),
            ));
        }

        let problems = self.idle.invalid();
        let mut listeners = column![meta_text(
            "Each entry waits for the screen to be idle, then runs a command. \
             Hyprforge stores these and never runs them itself.",
            12.0,
            scale,
        )]
        .spacing(spacing::SM);

        for (i, listener) in self.idle.listeners.iter().enumerate() {
            listeners = listeners.push(divider());
            let mut fields = column![].spacing(spacing::SM);
            fields = fields.push(labelled(
                "After (seconds)",
                text_input("300", &self.draft(i, Field::IdleTimeout, listener.timeout))
                    .on_input(move |v| Message::ListenerChanged(i, Field::IdleTimeout, v))
                    .on_submit(Message::Commit)
                    .padding(spacing::SM)
                    .width(Length::Fixed(90.0))
                    .into(),
                scale,
            ));
            fields = fields.push(labelled(
                "Run",
                text_input("loginctl lock-session", &self.draft(i, Field::OnTimeout, &listener.on_timeout))
                    .on_input(move |v| Message::ListenerChanged(i, Field::OnTimeout, v))
                    .on_submit(Message::Commit)
                    .padding(spacing::SM)
                    .into(),
                scale,
            ));
            fields = fields.push(labelled(
                "On return",
                text_input("", &self.draft(i, Field::OnResume, &listener.on_resume))
                    .on_input(move |v| Message::ListenerChanged(i, Field::OnResume, v))
                    .on_submit(Message::Commit)
                    .padding(spacing::SM)
                    .into(),
                scale,
            ));
            fields = fields.push(
                row![
                    checkbox(listener.ignore_inhibit)
                        .on_toggle(move |v| Message::ListenerToggled(i, Field::IgnoreInhibit, v)),
                    scaled_text("Even while something is blocking idle", 13.0, scale),
                ]
                .spacing(spacing::SM)
                .align_y(iced::Alignment::Center),
            );
            if let Some((_, problem)) = problems.iter().find(|(index, _)| *index == i) {
                fields = fields.push(scaled_text(problem.clone(), 12.0, scale));
            }
            fields = fields.push(danger_button("Remove", Message::ListenerRemoved(i)));
            listeners = listeners.push(fields);
        }
        listeners = listeners.push(divider());
        listeners = listeners.push(secondary_button("Add a timeout").on_press(Message::ListenerAdded));
        content = content.push(section("Idle timeouts", scale, listeners));

        let mut general = column![].spacing(spacing::SM);
        for (field, label, value) in self.idle.general.commands() {
            general = general.push(labelled(
                label,
                text_input("", value)
                    .on_input(move |v| Message::GeneralCommandChanged(field, v))
                    .on_submit(Message::Commit)
                    .padding(spacing::SM)
                    .into(),
                scale,
            ));
        }
        for (field, label, on) in [
            ("ignore_dbus_inhibit", "Ignore app idle blocks (D-Bus)", self.idle.general.ignore_dbus_inhibit),
            ("ignore_systemd_inhibit", "Ignore systemd idle blocks", self.idle.general.ignore_systemd_inhibit),
            ("ignore_wayland_inhibit", "Ignore Wayland idle blocks", self.idle.general.ignore_wayland_inhibit),
        ] {
            general = general.push(
                row![
                    checkbox(on).on_toggle(move |v| Message::InhibitToggled(field, v)),
                    scaled_text(label, 13.0, scale),
                ]
                .spacing(spacing::SM)
                .align_y(iced::Alignment::Center),
            );
        }
        content.push(section("Session commands", scale, general))
    }
}

/// The label a `monitor =` of empty means, spelled out — an empty
/// dropdown entry would read as "not set" rather than "all of them".
const ALL_MONITORS: &str = "All screens";

fn labelled<'a>(
    label: &'a str,
    control: Element<'a, Message>,
    scale: FontScale,
) -> Element<'a, Message> {
    row![
        container(scaled_text(label, 13.0, scale)).width(Length::FillPortion(2)),
        container(control).width(Length::FillPortion(3)),
    ]
    .spacing(spacing::MD)
    .align_y(iced::Alignment::Center)
    .into()
}

/// Keeps a stored value that discovery didn't find — an image on another
/// disk, a folder that moved. Dropping it would show nothing selected,
/// and the next pick would replace a working wallpaper.
fn options_including(known: &[String], current: &str) -> Vec<String> {
    let mut options = known.to_vec();
    let current = current.trim();
    if !current.is_empty() && !options.iter().any(|o| o == current) {
        options.insert(0, current.to_string());
    }
    options
}

fn load_or_note<T: serde::de::DeserializeOwned + Default>(
    path: &std::path::Path,
    problems: &mut Vec<String>,
) -> T {
    match hyprforge_ecosystem::storage::load(path) {
        Ok(value) => value,
        Err(e) => {
            problems.push(e.to_string());
            T::default()
        }
    }
}

async fn load_system() -> Loaded {
    tokio::task::spawn_blocking(|| Loaded {
        images: wallpaper::discover_images(),
        monitors: hyprforge_core::monitors::connector_names(),
    })
    .await
    .unwrap_or_default()
}

async fn apply_wallpapers(
    generated: PathBuf,
    target: PathBuf,
    settings: wallpaper::Settings,
) -> Result<Applied, String> {
    tokio::task::spawn_blocking(move || {
        apply::wallpapers(&generated, &target, &settings).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

async fn apply_sunset(
    generated: PathBuf,
    target: PathBuf,
    settings: sunset::Settings,
) -> Result<Applied, String> {
    tokio::task::spawn_blocking(move || {
        // The schedule is applied against the clock now, so the profile
        // that should be holding is the one pushed.
        let now = minutes_now();
        apply::temperature(&generated, &target, &settings, now).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

async fn apply_idle(
    generated: PathBuf,
    target: PathBuf,
    settings: idle::Settings,
) -> Result<Applied, String> {
    tokio::task::spawn_blocking(move || {
        apply::idle(&generated, &target, &settings).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Local minutes past midnight.
///
/// Read from `date` rather than computed from the Unix epoch: the epoch
/// is UTC and the schedule is in the user's local time, and the timezone
/// offset isn't something to reimplement.
fn minutes_now() -> u32 {
    let queried = hyprforge_core::command::output(
        std::process::Command::new("date").arg("+%H:%M"),
        hyprforge_core::command::TIMEOUT,
    );
    let Ok(out) = queried else {
        return 0;
    };
    sunset::parse_time(String::from_utf8_lossy(&out.stdout).trim()).unwrap_or(0)
}

fn wallpaper_toml() -> PathBuf {
    hyprforge_core::paths::hyprforge_config_dir().join("wallpaper.toml")
}

fn sunset_toml() -> PathBuf {
    hyprforge_core::paths::hyprforge_config_dir().join("night-light.toml")
}

fn idle_toml() -> PathBuf {
    hyprforge_core::paths::hyprforge_config_dir().join("idle.toml")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Runs `f` against a module whose config lives in a throwaway
    /// directory. Without this, `DesktopModule::new()` reads — and
    /// several update arms then *save over* — the developer's real
    /// wallpaper, night-light and idle settings, and would poke the real
    /// daemons besides.
    fn with_temp_config<T>(f: impl FnOnce(&mut DesktopModule) -> T) -> T {
        let _lock = crate::modules::CONFIG_ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().unwrap();
        let previous = std::env::var("XDG_CONFIG_HOME").ok();
        unsafe { std::env::set_var("XDG_CONFIG_HOME", dir.path()) };
        let (mut module, _) = DesktopModule::new();
        let out = f(&mut module);
        match previous {
            Some(p) => unsafe { std::env::set_var("XDG_CONFIG_HOME", p) },
            None => unsafe { std::env::remove_var("XDG_CONFIG_HOME") },
        }
        out
    }

    #[test]
    fn a_new_module_has_nothing_configured() {
        with_temp_config(|m| {
            assert!(m.wallpapers.is_empty());
            assert!(m.sunset.is_empty());
            assert!(m.idle.is_empty());
        });
    }

    /// A new wallpaper starts as the fallback with no image, and
    /// `invalid()` says so until one is chosen — better than inventing a
    /// path nobody picked.
    #[test]
    fn a_new_wallpaper_starts_empty_and_is_reported_until_filled() {
        with_temp_config(|m| {
            let _ = m.update(Message::WallpaperAdded);
            assert_eq!(m.wallpapers.entries.len(), 1);
            assert!(m.wallpapers.entries[0].is_fallback());
            assert_eq!(m.wallpapers.invalid().len(), 1);

            let _ = m.update(Message::WallpaperChanged(0, Field::Path, "/w/a.png".into()));
            assert_eq!(m.wallpapers.entries[0].path, "/w/a.png");
            assert_eq!(m.wallpapers.invalid(), vec![]);
        });
    }

    /// "All screens" is the label for an empty `monitor =`, and it has to
    /// map back to empty — a literal "All screens" would be a monitor
    /// name hyprpaper never matches.
    #[test]
    fn choosing_all_screens_stores_an_empty_monitor() {
        with_temp_config(|m| {
            let _ = m.update(Message::WallpaperAdded);
            let _ = m.update(Message::WallpaperChanged(0, Field::Monitor, "eDP-2".into()));
            assert_eq!(m.wallpapers.entries[0].monitor, "eDP-2");
            let _ = m.update(Message::WallpaperChanged(0, Field::Monitor, String::new()));
            assert!(m.wallpapers.entries[0].is_fallback());
        });
    }

    /// Typed numbers wait for Apply; a picked value doesn't. Committing
    /// per keystroke would rewrite a config file and poke a daemon on
    /// every character.
    #[test]
    fn typed_fields_wait_for_apply_but_picked_ones_do_not() {
        with_temp_config(|m| {
            let _ = m.update(Message::ProfileAdded);
            let _ = m.update(Message::ProfileChanged(0, Field::Temperature, "4000".into()));
            assert_eq!(m.sunset.profiles[0].temperature, 6000, "still a draft");
            let _ = m.update(Message::Commit);
            assert_eq!(m.sunset.profiles[0].temperature, 4000);
            assert!(m.drafts.is_empty());
        });
    }

    /// One bad field must not discard the others — the user typed those
    /// too.
    #[test]
    fn a_bad_value_keeps_what_was_typed_and_lets_the_rest_through() {
        with_temp_config(|m| {
            let _ = m.update(Message::ProfileAdded);
            let _ = m.update(Message::ProfileChanged(0, Field::Time, "25:00".into()));
            let _ = m.update(Message::ProfileChanged(0, Field::Temperature, "4000".into()));
            let _ = m.update(Message::Commit);

            assert_eq!(m.sunset.profiles[0].temperature, 4000, "the good one applied");
            assert_eq!(m.sunset.profiles[0].time, "00:00", "the bad one didn't");
            assert_eq!(
                m.drafts.get(&(0, Field::Time)).map(String::as_str),
                Some("25:00"),
                "what was typed must survive"
            );
            assert!(m.error.is_some(), "and be reported");
        });
    }

    #[test]
    fn an_out_of_range_temperature_is_refused() {
        with_temp_config(|m| {
            let _ = m.update(Message::ProfileAdded);
            for bad in ["500", "99999", "warm"] {
                let _ = m.update(Message::ProfileChanged(0, Field::Temperature, bad.into()));
                let _ = m.update(Message::Commit);
                assert_ne!(m.sunset.profiles[0].temperature.to_string(), bad, "{bad}");
            }
        });
    }

    #[test]
    fn a_zero_idle_timeout_is_refused() {
        with_temp_config(|m| {
            let _ = m.update(Message::ListenerAdded);
            let _ = m.update(Message::ListenerChanged(0, Field::IdleTimeout, "0".into()));
            let _ = m.update(Message::Commit);
            assert_eq!(m.idle.listeners[0].timeout, 300, "the default is kept");
        });
    }

    /// Shell commands are stored exactly as written — Hyprforge never
    /// runs them, and quoting or escaping would change what does.
    #[test]
    fn a_shell_command_is_stored_verbatim() {
        with_temp_config(|m| {
            let _ = m.update(Message::ListenerAdded);
            let command = "pidof hyprlock || hyprlock";
            let _ = m.update(Message::ListenerChanged(0, Field::OnTimeout, command.into()));
            let _ = m.update(Message::Commit);
            assert_eq!(m.idle.listeners[0].on_timeout, command);
        });
    }

    /// Removing a row must take its half-typed drafts with it, or they'd
    /// reappear against whatever row slid into that index.
    #[test]
    fn removing_a_row_discards_its_drafts() {
        with_temp_config(|m| {
            let _ = m.update(Message::ProfileAdded);
            let _ = m.update(Message::ProfileChanged(0, Field::Time, "21:00".into()));
            let _ = m.update(Message::ProfileRemoved(0));
            assert!(m.sunset.profiles.is_empty());
            assert!(m.drafts.is_empty());
        });
    }

    /// The load-bearing honesty on this screen: hypridle has no IPC, so
    /// a save genuinely does nothing until it restarts, and the status
    /// must say which of the three things happened.
    #[test]
    fn each_apply_outcome_gets_its_own_message() {
        with_temp_config(|m| {
            let _ = m.update(Message::Applied(Tab::Idle, Ok(Applied::NeedsRestart)));
            assert!(m.idle_needs_restart);
            let status = m.status.clone().unwrap();
            assert!(status.contains("restart"), "{status}");

            let _ = m.update(Message::Applied(Tab::Wallpaper, Ok(Applied::Live)));
            assert!(!m.idle_needs_restart, "a live apply clears the restart notice");
            assert!(m.status.clone().unwrap().contains("applied"));

            let _ = m.update(Message::Applied(Tab::Wallpaper, Ok(Applied::DaemonNotRunning)));
            assert!(m.status.clone().unwrap().contains("isn't running"));

            let _ = m.update(Message::Applied(
                Tab::Wallpaper,
                Ok(Applied::PartlyRefused(vec!["/w/gone.png".into()])),
            ));
            assert!(m.error.is_some(), "a refusal is an error, not a status");
        });
    }

    /// The rule that keeps the data-loss bug from returning.
    #[test]
    fn an_unreadable_store_blocks_saving() {
        with_temp_config(|m| {
            m.store_unreadable = Some("bad toml".into());
            let _ = m.update(Message::WallpaperAdded);
            let _ = m.update(Message::WallpaperRemoved(0));
            assert!(m.error.is_some(), "the user has to be told why");
        });
    }

    /// An image on another disk, or a folder that moved, would otherwise
    /// vanish from its own picker — and the next pick would replace a
    /// working wallpaper.
    #[test]
    fn a_picker_keeps_a_path_discovery_did_not_find() {
        let known = vec!["/w/a.png".to_string()];
        let options = options_including(&known, "/elsewhere/b.png");
        assert_eq!(options.len(), 2);
        assert_eq!(options[0], "/elsewhere/b.png", "kept, and first so it's visible");
        assert_eq!(options_including(&known, "/w/a.png").len(), 1, "no duplicate");
        assert_eq!(options_including(&known, "").len(), 1);
    }

    /// Typing in one tab, switching, and pressing Apply must still save
    /// the first tab — otherwise the edit sits in memory and is never
    /// written.
    #[test]
    fn applying_saves_every_tab_that_was_edited_not_just_the_open_one() {
        with_temp_config(|m| {
            let _ = m.update(Message::WallpaperAdded);
            let _ = m.update(Message::ProfileAdded);
            let _ = m.update(Message::TabSelected(Tab::Wallpaper));
            let _ = m.update(Message::WallpaperChanged(0, Field::Timeout, "45".into()));
            let _ = m.update(Message::TabSelected(Tab::NightLight));
            let _ = m.update(Message::ProfileChanged(0, Field::Temperature, "4000".into()));

            let _ = m.update(Message::Commit);
            assert_eq!(m.wallpapers.entries[0].timeout, Some(45), "the other tab's edit applied");
            assert_eq!(m.sunset.profiles[0].temperature, 4000);
            assert!(m.drafts.is_empty());
        });
    }

    /// Removing row 1 makes row 2 become row 1, so a draft keyed to index
    /// 2 would reappear against a different row's values.
    #[test]
    fn removing_a_row_shifts_the_later_rows_drafts_down() {
        with_temp_config(|m| {
            for _ in 0..3 {
                let _ = m.update(Message::ProfileAdded);
            }
            let _ = m.update(Message::ProfileChanged(2, Field::Temperature, "4000".into()));
            let _ = m.update(Message::ProfileRemoved(0));

            assert_eq!(m.sunset.profiles.len(), 2);
            assert_eq!(
                m.drafts.get(&(1, Field::Temperature)).map(String::as_str),
                Some("4000"),
                "the draft must follow its row down"
            );
            assert!(!m.drafts.contains_key(&(2, Field::Temperature)));
        });
    }

    /// The three lists share one draft map but have separate indices, so
    /// removing a wallpaper must not disturb a profile's drafts.
    #[test]
    fn removing_a_row_leaves_other_tabs_drafts_alone() {
        with_temp_config(|m| {
            let _ = m.update(Message::WallpaperAdded);
            let _ = m.update(Message::ProfileAdded);
            let _ = m.update(Message::ProfileChanged(0, Field::Temperature, "4000".into()));
            let _ = m.update(Message::WallpaperRemoved(0));
            assert_eq!(
                m.drafts.get(&(0, Field::Temperature)).map(String::as_str),
                Some("4000")
            );
        });
    }

    /// A refused push means the setting is saved but the screen did not
    /// change — calling that "applied" is the failure this project keeps
    /// meeting.
    #[test]
    fn a_refused_push_is_reported_as_an_error_not_a_success() {
        with_temp_config(|m| {
            let _ = m.update(Message::Applied(
                Tab::Wallpaper,
                Ok(Applied::PartlyRefused(vec!["/w/gone.png".into()])),
            ));
            assert!(m.status.is_none(), "it did not succeed");
            let error = m.error.clone().unwrap();
            assert!(error.contains("/w/gone.png"), "{error}");
        });
    }

    /// Builds every tab in each state it can be in.
    #[test]
    fn the_screen_builds_in_every_state() {
        with_temp_config(|m| {
            let scale = FontScale::default();
            for tab in Tab::ALL {
                let _ = m.update(Message::TabSelected(tab));
                let _ = m.view(scale);
            }

            let _ = m.update(Message::Loaded(Loaded {
                images: vec!["/w/a.png".into()],
                monitors: vec!["eDP-2".into()],
            }));
            let _ = m.update(Message::WallpaperAdded);
            let _ = m.update(Message::ProfileAdded);
            let _ = m.update(Message::ListenerAdded);
            m.idle_needs_restart = true;
            m.error = Some("something failed".into());
            m.status = Some("saved".into());
            m.store_unreadable = Some("bad toml".into());
            for tab in Tab::ALL {
                let _ = m.update(Message::TabSelected(tab));
                let _ = m.view(scale);
            }
        });
    }
}
