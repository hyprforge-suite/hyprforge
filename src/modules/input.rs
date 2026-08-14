//! Keyboard, pointer and touchpad settings.
//!
//! The screen is generated from [`hyprforge_input::catalog`] rather than
//! hand-laid-out: the catalog already knows every option's type, range and
//! accepted values, and a hand-written form would be a second, drifting
//! copy of all three. Adding an option means adding a catalog entry.
//!
//! The module's central idea shows up directly in the UI: a setting is
//! either **owned** — Hyprforge writes it, and it wins — or absent, in
//! which case Hyprland's default or the user's own config decides. Every
//! row can be handed back with its Reset button, which is not the same as
//! setting it to the default value: one stops mentioning the key, the other
//! writes it.

use hyprforge_core::lua_setup;
use hyprforge_core::theme::{spacing, FontScale};
use hyprforge_core::widgets::{
    danger_button, divider, meta_text, primary_button, scaled_text, secondary_button,
    section, setup_notice,
};
use hyprforge_core::SettingsModule;
use hyprforge_input::catalog::{self, Kind, Setting};
use hyprforge_input::import::{Discovered, Live};
use hyprforge_input::model::{Invalid, Settings, Value};
use hyprforge_input::setup::{HyprConfig, SetupPlan};
use iced::widget::{checkbox, column, container, pick_list, row, scrollable, text_input};
use iced::{Element, Length, Task};
use std::collections::BTreeMap;

#[derive(Debug, Clone)]
pub enum Message {
    /// A control whose value is complete the moment it changes — a
    /// checkbox or a dropdown. Applied immediately, the way every settings
    /// app behaves.
    Set(&'static str, Value),
    /// A character typed into a text or number field. Held as a draft, not
    /// applied: applying per keystroke would reload Hyprland once per
    /// character.
    DraftChanged(&'static str, String),
    /// Enter in a single field, or the Apply button for all of them.
    ApplyDrafts,
    DiscardDrafts,
    /// Stop writing this key at all, giving it back to Hyprland and the
    /// user's own config.
    Reset(&'static str),
    FilterChanged(String),
    ImportOpen,
    ImportLoaded(Result<Vec<Discovered>, String>),
    ImportToggle(usize),
    ImportConfirm,
    ImportCancel,
    /// The compositor's current value for every option, read once on open.
    LiveLoaded(Vec<Live>),
    Reloaded(Result<(), String>),
}

/// An import run, from click to review.
enum ImportState {
    /// The compositor is being asked what's set.
    Running,
    Ready(Vec<Candidate>),
}

struct Candidate {
    found: Discovered,
    selected: bool,
}

pub struct InputModule {
    settings: Settings,
    /// In-progress text for the fields that can't be committed per
    /// keystroke. Keyed by catalog key; absent means "showing the stored
    /// value".
    drafts: BTreeMap<&'static str, String>,
    /// Per-key reason the draft can't be applied, so a bad number is
    /// reported next to the field that holds it rather than as one vague
    /// banner at the top.
    draft_errors: BTreeMap<&'static str, String>,
    filter: String,
    config: HyprConfig,
    setup_plan: SetupPlan,
    error: Option<String>,
    status: Option<String>,
    /// Why the stored settings couldn't be read, if they couldn't. While
    /// set, the module refuses to write anything — an unreadable store is
    /// not an empty one, and saving over it would destroy what's there.
    store_unreadable: Option<String>,
    /// Stored values the catalog rejects, from a hand-edited file. Shown
    /// rather than dropped: they're skipped by codegen, so without this the
    /// user would see a setting in their TOML quietly doing nothing.
    invalid: Vec<Invalid>,
    import_review: Option<ImportState>,
    /// What Hyprland currently has for each option, and whether the user's
    /// own config is what put it there.
    ///
    /// Without this a row for an unowned key falls back to the *catalog*
    /// default, which is a different number from what's running whenever
    /// the user's config sets it — the screen would show `numlock` off
    /// while it's on. Empty until the read returns, and if it fails the
    /// rows fall back to the catalog default and say so.
    live: BTreeMap<&'static str, Live>,
}

impl InputModule {
    pub fn new() -> (Self, Task<Message>) {
        // A failure here must never look like "you've configured nothing":
        // that reading is what turns one bad parse into a wiped store on
        // the next save.
        let (settings, store_unreadable) =
            match hyprforge_input::storage::load(&hyprforge_core::paths::input_toml_path()) {
                Ok(settings) => (settings, None),
                Err(e) => (Settings::default(), Some(e.to_string())),
            };
        let invalid = settings.validate();
        let setup = lua_setup::bootstrap(
            &hyprforge_core::paths::hypr_config_dir(),
            &hyprforge_core::paths::hyprland_lua_path(),
            lua_setup::ModuleSetup {
                require_line: hyprforge_input::setup::REQUIRE_LINE,
                placement: hyprforge_input::setup::PLACEMENT,
                generated: (
                    hyprforge_core::paths::input_lua_path(),
                    hyprforge_input::codegen::generate(&Settings::default()),
                ),
            },
        );
        (
            InputModule {
                settings,
                drafts: BTreeMap::new(),
                draft_errors: BTreeMap::new(),
                filter: String::new(),
                config: setup.config,
                setup_plan: setup.plan,
                error: setup.error,
                status: None,
                store_unreadable,
                invalid,
                import_review: None,
                live: BTreeMap::new(),
            },
            // Read what's actually running, so unowned rows show the truth
            // rather than the catalog default. A failure is silent by
            // design: the rows still render, they just can't claim to know
            // what's live.
            Task::perform(read_live(), Message::LiveLoaded),
        )
    }

    /// Writes the canonical TOML. Nothing irreversible may run unless this
    /// returned `Ok` — the ordering rule that exists because doing it the
    /// other way round cost a real user 37 hand-written binds.
    fn persist(&mut self) -> Result<(), String> {
        if let Some(reason) = &self.store_unreadable {
            let message = format!(
                "Not saving — your input.toml couldn't be read, and overwriting \
                 it would lose whatever is in it. ({reason})"
            );
            self.error = Some(message.clone());
            return Err(message);
        }
        hyprforge_input::storage::save(
            &hyprforge_core::paths::input_toml_path(),
            &self.settings,
        )
        .map_err(|e| {
            self.error = Some(e.to_string());
            e.to_string()
        })
    }

    fn save_and_maybe_reload(&mut self) -> Task<Message> {
        if self.persist().is_err() {
            return Task::none();
        }
        self.invalid = self.settings.validate();
        // Without a Lua config there's nothing to source the generated file
        // from, so reloading would be a no-op dressed up as success. The
        // TOML is still saved, and takes effect once setup is resolved.
        if !matches!(self.config, HyprConfig::Lua(_)) {
            self.error = None;
            self.status = Some(
                "Saved. Settings take effect once Hyprland setup is finished — see above."
                    .to_string(),
            );
            return Task::none();
        }
        // The require line is installed automatically on open; this only
        // retries if that attempt failed.
        if self.setup_plan != SetupPlan::AlreadyPresent {
            match hyprforge_input::setup::install(&hyprforge_core::paths::hyprland_lua_path()) {
                Ok(plan) => self.setup_plan = plan,
                Err(e) => {
                    self.error = Some(e.to_string());
                    return Task::none();
                }
            }
        }
        Task::perform(regenerate_and_reload(self.settings.clone()), Message::Reloaded)
    }

    /// The value a row should display: the draft if one is being typed,
    /// otherwise what's stored, otherwise the catalog default.
    fn shown_text(&self, setting: &Setting) -> String {
        if let Some(draft) = self.drafts.get(setting.key) {
            return draft.clone();
        }
        render_for_edit(&self.effective(setting).0)
    }

    fn owns(&self, key: &str) -> bool {
        self.settings.get(key).is_some()
    }

    /// The value a control should display, and where it came from.
    ///
    /// Owned beats live beats the catalog default. The order matters in
    /// both directions: an owned value that hasn't reached the compositor
    /// yet must still show what the user chose, and an unowned one must
    /// show what's running rather than what Hyprland would do by default.
    fn effective(&self, setting: &Setting) -> (Value, Source) {
        if let Some(v) = self.settings.get(setting.key) {
            return (v.clone(), Source::Owned);
        }
        if let Some(live) = self.live.get(setting.key) {
            return (
                live.value.clone(),
                if live.set { Source::UserConfig } else { Source::Default },
            );
        }
        (Value::default_for(&setting.kind), Source::Default)
    }

    /// Parses every pending draft into the store. All-or-nothing per field:
    /// a field that doesn't parse keeps its draft and its error, and the
    /// ones that do parse are still applied, so one typo doesn't discard
    /// everything else the user typed.
    fn apply_drafts(&mut self) -> Task<Message> {
        self.draft_errors.clear();
        let pending: Vec<(&'static str, String)> =
            self.drafts.iter().map(|(k, v)| (*k, v.clone())).collect();
        let mut applied = false;
        for (key, raw) in pending {
            let Some(setting) = catalog::get(key) else {
                continue;
            };
            match parse_for(&setting.kind, &raw) {
                Ok(value) => {
                    let problems = Settings::from_one(key, value.clone()).validate();
                    if let Some(problem) = problems.first() {
                        self.draft_errors.insert(key, problem.problem.clone());
                    } else {
                        self.settings.set(key, value);
                        self.drafts.remove(key);
                        applied = true;
                    }
                }
                Err(problem) => {
                    self.draft_errors.insert(key, problem);
                }
            }
        }
        if !applied {
            return Task::none();
        }
        self.save_and_maybe_reload()
    }

    /// Rows matching the filter box, so a screen with fifty-odd options is
    /// still navigable. Matches the label, the key and the help text: a
    /// user looking for "natural scroll" and one looking for
    /// `kb_options` both find what they mean.
    fn matches_filter(&self, setting: &Setting) -> bool {
        let q = self.filter.trim().to_lowercase();
        if q.is_empty() {
            return true;
        }
        setting.label.to_lowercase().contains(&q)
            || setting.key.to_lowercase().contains(&q)
            || setting.help.to_lowercase().contains(&q)
    }
}

impl SettingsModule for InputModule {
    type Message = Message;

    fn title(&self) -> &str {
        "Input"
    }

    fn icon(&self) -> &'static str {
        "⌨"
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Set(key, value) => {
                self.settings.set(key, value);
                self.status = None;
                self.save_and_maybe_reload()
            }
            Message::DraftChanged(key, raw) => {
                self.drafts.insert(key, raw);
                self.draft_errors.remove(key);
                Task::none()
            }
            Message::ApplyDrafts => self.apply_drafts(),
            Message::DiscardDrafts => {
                self.drafts.clear();
                self.draft_errors.clear();
                Task::none()
            }
            Message::Reset(key) => {
                self.settings.clear(key);
                self.drafts.remove(key);
                self.draft_errors.remove(key);
                self.status = None;
                self.save_and_maybe_reload()
            }
            Message::FilterChanged(q) => {
                self.filter = q;
                Task::none()
            }
            Message::ImportOpen => {
                self.import_review = Some(ImportState::Running);
                let current = self.settings.clone();
                Task::perform(discover_settings(current), Message::ImportLoaded)
            }
            Message::ImportLoaded(Ok(found)) => {
                self.import_review = Some(ImportState::Ready(
                    found
                        .into_iter()
                        // Anything already owned starts unticked: the user
                        // came here to adopt what they haven't got, and
                        // re-importing would overwrite a value they set in
                        // this app with one from their config file.
                        .map(|found| Candidate {
                            selected: !found.already_owned,
                            found,
                        })
                        .collect(),
                ));
                Task::none()
            }
            Message::ImportLoaded(Err(e)) => {
                self.import_review = None;
                self.error = Some(e);
                Task::none()
            }
            Message::ImportToggle(i) => {
                if let Some(ImportState::Ready(candidates)) = &mut self.import_review {
                    if let Some(c) = candidates.get_mut(i) {
                        c.selected = !c.selected;
                    }
                }
                Task::none()
            }
            Message::ImportConfirm => {
                let chosen: Vec<Discovered> = match &self.import_review {
                    Some(ImportState::Ready(candidates)) => candidates
                        .iter()
                        .filter(|c| c.selected)
                        .map(|c| c.found.clone())
                        .collect(),
                    _ => Vec::new(),
                };
                self.import_review = None;
                if chosen.is_empty() {
                    return Task::none();
                }
                hyprforge_input::import::merge(&mut self.settings, &chosen);
                self.status = Some(format!("Imported {} setting(s).", chosen.len()));
                self.save_and_maybe_reload()
            }
            Message::ImportCancel => {
                self.import_review = None;
                Task::none()
            }
            Message::LiveLoaded(live) => {
                self.live = live.into_iter().map(|l| (l.key, l)).collect();
                Task::none()
            }
            Message::Reloaded(Ok(())) => {
                self.error = None;
                self.status = Some("Saved.".to_string());
                // The reload just changed what's running, so the cached
                // live values are stale. This matters most right after a
                // Reset: the row falls back to the live value, and without
                // re-reading it would show the value Hyprforge had been
                // setting rather than the one the user's config just took
                // back over.
                Task::perform(read_live(), Message::LiveLoaded)
            }
            Message::Reloaded(Err(e)) => {
                self.status = None;
                self.error = Some(e);
                Task::none()
            }
        }
    }

    fn view(&self, scale: FontScale) -> Element<'_, Message> {
        if let Some(review) = &self.import_review {
            return self.import_view(review, scale);
        }

        let mut content = column![].spacing(spacing::LG).width(Length::Fill);

        if let Some(notice) = setup_notice(&self.config, "input settings", scale) {
            content = content.push(notice);
        }
        if let Some(reason) = &self.store_unreadable {
            content = content.push(section(
                "Your settings file couldn't be read",
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
        if !self.invalid.is_empty() {
            let mut list = column![scaled_text(
                "These are in your input.toml but aren't being applied. Fix or \
                 remove them:",
                13.0,
                scale,
            )]
            .spacing(spacing::XS);
            for bad in &self.invalid {
                list = list.push(meta_text(
                    format!("{} — {}", bad.key, bad.problem),
                    12.0,
                    scale,
                ));
            }
            content = content.push(section("Settings being skipped", scale, list));
        }
        if let Some(e) = &self.error {
            content = content.push(section("Something went wrong", scale, scaled_text(e.clone(), 13.0, scale)));
        }
        if let Some(s) = &self.status {
            content = content.push(meta_text(s.clone(), 12.0, scale));
        }

        content = content.push(
            row![
                text_input("Filter settings…", &self.filter)
                    .on_input(Message::FilterChanged)
                    .padding(spacing::SM),
                secondary_button("Import from Hyprland").on_press(Message::ImportOpen),
            ]
            .spacing(spacing::MD)
            .align_y(iced::Alignment::Center),
        );

        if !self.drafts.is_empty() {
            content = content.push(
                row![
                    scaled_text(
                        format!("{} field(s) typed but not applied", self.drafts.len()),
                        13.0,
                        scale,
                    ),
                    primary_button("Apply").on_press(Message::ApplyDrafts),
                    secondary_button("Discard").on_press(Message::DiscardDrafts),
                ]
                .spacing(spacing::MD)
                .align_y(iced::Alignment::Center),
            );
        }

        let mut any_row = false;
        for category in catalog::CATEGORIES {
            let rows: Vec<&'static Setting> = catalog::in_category(category.key)
                .filter(|s| self.matches_filter(s))
                .collect();
            if rows.is_empty() {
                continue;
            }
            any_row = true;
            let mut body = column![meta_text(category.help, 12.0, scale)].spacing(spacing::SM);
            for setting in rows {
                body = body.push(divider());
                body = body.push(self.setting_row(setting, scale));
            }
            content = content.push(section(category.label, scale, body));
        }

        if !any_row {
            content = content.push(scaled_text(
                format!("Nothing matches “{}”.", self.filter.trim()),
                13.0,
                scale,
            ));
        }

        for (category, why) in hyprforge_input::model::UNSUPPORTED_CATEGORIES {
            if !self.filter.trim().is_empty() && !category.contains(&self.filter.trim().to_lowercase()) {
                continue;
            }
            content = content.push(meta_text(*why, 12.0, scale));
        }

        scrollable(container(content).padding(spacing::LG))
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    }
}

impl InputModule {
    fn setting_row(&self, setting: &'static Setting, scale: FontScale) -> Element<'_, Message> {
        let key = setting.key;
        let owned = self.owns(key);

        // Every control reads through `effective`, so an unowned row shows
        // what is actually running rather than what Hyprland would do if
        // nobody had configured anything.
        let (current, source) = self.effective(setting);

        let control: Element<'_, Message> = match setting.kind {
            Kind::Bool { default } => {
                let current = current.as_bool().unwrap_or(default);
                checkbox(current)
                    .on_toggle(move |b| Message::Set(key, Value::Bool(b)))
                    .into()
            }
            Kind::IntEnum { default, choices } => {
                let current = current.as_int().unwrap_or(default);
                let options: Vec<Choice> = choices
                    .iter()
                    .map(|(v, label)| Choice {
                        value: *v,
                        label,
                    })
                    .collect();
                let selected = options.iter().find(|c| c.value == current).copied();
                pick_list(options, selected, move |c: Choice| {
                    Message::Set(key, Value::Int(c.value))
                })
                .into()
            }
            Kind::TextEnum { default, choices } => {
                let current = current.as_text().unwrap_or(default).to_string();
                let options: Vec<TextChoice> =
                    choices.iter().map(|c| TextChoice { value: c }).collect();
                let selected = options
                    .iter()
                    .find(|c| c.value == current.as_str())
                    .copied();
                pick_list(options, selected, move |c: TextChoice| {
                    Message::Set(key, Value::Text(c.value.to_string()))
                })
                .into()
            }
            _ => text_input(&placeholder_for(&setting.kind), &self.shown_text(setting))
                .on_input(move |raw| Message::DraftChanged(key, raw))
                .on_submit(Message::ApplyDrafts)
                .padding(spacing::SM)
                .into(),
        };

        let mut label_side = column![scaled_text(setting.label, 14.0, scale)].spacing(2);
        label_side = label_side.push(meta_text(setting.help, 12.0, scale));
        if let Some(problem) = self.draft_errors.get(key) {
            label_side = label_side.push(scaled_text(problem.clone(), 12.0, scale));
        }
        // Where the displayed value comes from is the load-bearing
        // distinction on this screen. "Set by Hyprforge" means this app
        // writes it and it wins; the other two mean the app is only
        // reporting, and touching the control takes it over.
        label_side = label_side.push(meta_text(
            match source {
                Source::Owned => "Set by Hyprforge",
                Source::UserConfig => "From your Hyprland config",
                Source::Default => "Hyprland default",
            },
            12.0,
            scale,
        ));

        let control_side: Element<'_, Message> = if owned {
            row![
                container(control).width(Length::Fill),
                danger_button("Reset", Message::Reset(key)),
            ]
            .spacing(spacing::SM)
            .align_y(iced::Alignment::Center)
            .into()
        } else {
            control
        };

        // Laid out like `widgets::row_field`, but built here because the
        // label is a stack (name, help, ownership) rather than one string.
        row![
            container(label_side).width(Length::FillPortion(2)),
            container(control_side).width(Length::FillPortion(3)),
        ]
        .spacing(spacing::MD)
        .align_y(iced::Alignment::Center)
        .into()
    }

    fn import_view(&self, review: &ImportState, scale: FontScale) -> Element<'_, Message> {
        let body: Element<'_, Message> = match review {
            ImportState::Running => scaled_text("Reading your current settings…", 13.0, scale).into(),
            ImportState::Ready(candidates) if candidates.is_empty() => column![
                scaled_text(
                    "Hyprland reports no input options set beyond its own defaults, \
                     so there's nothing to import.",
                    13.0,
                    scale,
                ),
                secondary_button("Close").on_press(Message::ImportCancel),
            ]
            .spacing(spacing::MD)
            .into(),
            ImportState::Ready(candidates) => {
                let mut list = column![
                    scaled_text(
                        "These are the input options the running Hyprland reports as \
                         set, rather than left at its own default. Importing one hands \
                         it to Hyprforge, which from then on writes it and wins over \
                         your config file.",
                        13.0,
                        scale,
                    ),
                    // Hyprland reports "set", not "set *by your config*", and
                    // it can't tell the two apart. Anything that wrote an
                    // option since the last reload — hyprctl, a script, a
                    // tiling helper — looks identical here. Saying so is the
                    // difference between the user recognising a stray value
                    // and adopting it permanently without noticing.
                    meta_text(
                        "A value changed at runtime — by hyprctl, a script, or another \
                         tool — is reported as set too, and Hyprland can't tell it apart \
                         from one your config file wrote. Run `hyprctl reload` first if \
                         you want exactly what's in your config.",
                        12.0,
                        scale,
                    ),
                ]
                .spacing(spacing::SM);
                for (i, c) in candidates.iter().enumerate() {
                    let mut label = column![row![
                        checkbox(c.selected).on_toggle(move |_| Message::ImportToggle(i)),
                        scaled_text(
                            format!("{} — {}", c.found.label, render_for_edit(&c.found.value)),
                            13.0,
                            scale,
                        ),
                    ]
                    .spacing(spacing::SM)
                    .align_y(iced::Alignment::Center)]
                    .spacing(2);
                    if c.found.differs {
                        label = label.push(meta_text(
                            "Different from what Hyprforge has — importing replaces it.",
                            12.0,
                            scale,
                        ));
                    } else if c.found.already_owned {
                        label = label.push(meta_text("Already imported.", 12.0, scale));
                    }
                    list = list.push(label);
                }
                column![
                    list,
                    row![
                        primary_button("Import selected").on_press(Message::ImportConfirm),
                        secondary_button("Cancel").on_press(Message::ImportCancel),
                    ]
                    .spacing(spacing::MD),
                ]
                .spacing(spacing::LG)
                .into()
            }
        };
        scrollable(container(section("Import from Hyprland", scale, body)).padding(spacing::LG))
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    }
}

/// Where the value a row is showing came from. Not cosmetic: it's the
/// difference between a number this app writes into the config and one it
/// is merely reporting back from the compositor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Source {
    /// Hyprforge writes this key, and it wins over the user's config.
    Owned,
    /// The user's own config sets it; Hyprforge is only displaying it.
    UserConfig,
    /// Nobody set it, so this is what Hyprland does on its own.
    Default,
}

/// A dropdown entry for an [`Kind::IntEnum`]. Carries the number so the
/// message doesn't have to map a label back to a value — a mapping that
/// silently breaks the moment two choices share a label.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Choice {
    value: i64,
    label: &'static str,
}

impl std::fmt::Display for Choice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.label)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct TextChoice {
    value: &'static str,
}

impl std::fmt::Display for TextChoice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.value.is_empty() {
            f.write_str("Device default")
        } else {
            f.write_str(self.value)
        }
    }
}

/// How a value is written into a text field. Floats keep their decimal
/// point so a field showing `1` for a float setting can't be mistaken for
/// an integer one.
fn render_for_edit(value: &Value) -> String {
    match value {
        Value::Bool(b) => b.to_string(),
        Value::Int(i) => i.to_string(),
        Value::Float(f) => {
            let s = format!("{f:?}");
            if s.contains('.') {
                s
            } else {
                format!("{s}.0")
            }
        }
        Value::Text(s) => s.clone(),
    }
}

fn placeholder_for(kind: &Kind) -> String {
    match kind {
        Kind::Int { default, .. } => default.to_string(),
        Kind::Float { default, .. } => render_for_edit(&Value::Float(*default)),
        Kind::Text { default: "" } => "not set".to_string(),
        Kind::Text { default } => (*default).to_string(),
        _ => String::new(),
    }
}

/// Turns what was typed into the type the catalog expects. The error text
/// is what the user sees under the field, so it names the expectation
/// rather than echoing a parser's wording.
fn parse_for(kind: &Kind, raw: &str) -> Result<Value, String> {
    let trimmed = raw.trim();
    match kind {
        Kind::Int { .. } | Kind::IntEnum { .. } => trimmed
            .parse::<i64>()
            .map(Value::Int)
            .map_err(|_| "expected a whole number".to_string()),
        Kind::Float { .. } => trimmed
            .parse::<f64>()
            .map(Value::Float)
            .map_err(|_| "expected a number".to_string()),
        Kind::Bool { .. } => trimmed
            .parse::<bool>()
            .map(Value::Bool)
            .map_err(|_| "expected true or false".to_string()),
        // Text keeps its surrounding whitespace: a layout list like
        // "us, cz" is the user's to format, and trimming the interior would
        // be wrong anyway.
        Kind::Text { .. } | Kind::TextEnum { .. } => Ok(Value::Text(raw.to_string())),
    }
}

/// Reads every option's live value. A failure yields an empty list rather
/// than an error: the screen is fully usable without it, rows just fall
/// back to the catalog default, and a banner about a background read the
/// user never asked for would be noise.
async fn read_live() -> Vec<Live> {
    tokio::task::spawn_blocking(|| hyprforge_input::import::live().unwrap_or_default())
        .await
        .unwrap_or_default()
}

async fn discover_settings(current: Settings) -> Result<Vec<Discovered>, String> {
    tokio::task::spawn_blocking(move || {
        hyprforge_input::import::discover(&current).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

async fn regenerate_and_reload(settings: Settings) -> Result<(), String> {
    tokio::task::spawn_blocking(move || {
        hyprforge_input::apply::apply(&hyprforge_core::paths::input_lua_path(), &settings)
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Runs `f` against a module whose config lives in a throwaway
    /// directory. Without this, `InputModule::new()` reads — and several
    /// update arms then *save over* — the developer's real
    /// `~/.config/hyprforge/input.toml`.
    fn with_temp_config<T>(f: impl FnOnce(&mut InputModule) -> T) -> T {
        let _lock = crate::modules::CONFIG_ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().unwrap();
        let previous = std::env::var("XDG_CONFIG_HOME").ok();
        unsafe { std::env::set_var("XDG_CONFIG_HOME", dir.path()) };
        let (mut module, _) = InputModule::new();
        let out = f(&mut module);
        match previous {
            Some(p) => unsafe { std::env::set_var("XDG_CONFIG_HOME", p) },
            None => unsafe { std::env::remove_var("XDG_CONFIG_HOME") },
        }
        out
    }

    #[test]
    fn a_new_module_owns_nothing() {
        with_temp_config(|m| {
            assert!(m.settings.is_empty());
            assert!(!m.owns("input:kb_layout"));
        });
    }

    #[test]
    fn toggling_a_checkbox_takes_ownership_of_the_key() {
        with_temp_config(|m| {
            let _ = m.update(Message::Set("input:numlock_by_default", Value::Bool(true)));
            assert!(m.owns("input:numlock_by_default"));
            assert_eq!(
                m.settings.get("input:numlock_by_default"),
                Some(&Value::Bool(true))
            );
        });
    }

    /// Reset is not "set it to the default" — it stops writing the key, so
    /// Hyprland and the user's own config decide again.
    #[test]
    fn reset_gives_the_key_back_rather_than_writing_the_default() {
        with_temp_config(|m| {
            let _ = m.update(Message::Set("input:repeat_rate", Value::Int(30)));
            assert!(m.owns("input:repeat_rate"));
            let _ = m.update(Message::Reset("input:repeat_rate"));
            assert!(!m.owns("input:repeat_rate"));
            let lua = hyprforge_input::codegen::generate(&m.settings);
            assert!(!lua.contains("repeat_rate"), "{lua}");
        });
    }

    /// Typing must not write anything: a keystroke-per-save would reload
    /// Hyprland once per character.
    #[test]
    fn typing_does_not_commit_until_applied() {
        with_temp_config(|m| {
            let _ = m.update(Message::DraftChanged("input:repeat_rate", "45".into()));
            assert!(!m.owns("input:repeat_rate"), "still just a draft");
            let _ = m.update(Message::ApplyDrafts);
            assert_eq!(m.settings.get("input:repeat_rate"), Some(&Value::Int(45)));
            assert!(m.drafts.is_empty());
        });
    }

    #[test]
    fn a_bad_number_reports_next_to_its_own_field_and_keeps_what_was_typed() {
        with_temp_config(|m| {
            let _ = m.update(Message::DraftChanged("input:repeat_rate", "fast".into()));
            let _ = m.update(Message::ApplyDrafts);
            assert!(!m.owns("input:repeat_rate"));
            assert!(m.draft_errors.contains_key("input:repeat_rate"));
            assert_eq!(
                m.drafts.get("input:repeat_rate").map(String::as_str),
                Some("fast"),
                "what the user typed must survive the failed apply"
            );
        });
    }

    /// One bad field must not discard the others — the user typed those too.
    #[test]
    fn a_single_bad_field_does_not_block_the_good_ones() {
        with_temp_config(|m| {
            let _ = m.update(Message::DraftChanged("input:repeat_rate", "oops".into()));
            let _ = m.update(Message::DraftChanged("input:repeat_delay", "450".into()));
            let _ = m.update(Message::ApplyDrafts);
            assert_eq!(m.settings.get("input:repeat_delay"), Some(&Value::Int(450)));
            assert!(!m.owns("input:repeat_rate"));
        });
    }

    /// An out-of-range value is caught by the same validator the file uses,
    /// so the editor and a hand-edit can't disagree about what's allowed.
    #[test]
    fn an_out_of_range_value_is_refused_with_its_limit() {
        with_temp_config(|m| {
            let _ = m.update(Message::DraftChanged("input:sensitivity", "5".into()));
            let _ = m.update(Message::ApplyDrafts);
            assert!(!m.owns("input:sensitivity"));
            let problem = &m.draft_errors["input:sensitivity"];
            assert!(problem.contains("at most 1"), "{problem}");
        });
    }

    #[test]
    fn discarding_drafts_leaves_the_store_untouched() {
        with_temp_config(|m| {
            let _ = m.update(Message::Set("input:repeat_rate", Value::Int(30)));
            let _ = m.update(Message::DraftChanged("input:repeat_rate", "99".into()));
            let _ = m.update(Message::DiscardDrafts);
            assert_eq!(m.settings.get("input:repeat_rate"), Some(&Value::Int(30)));
            assert!(m.drafts.is_empty());
        });
    }

    /// The rule that keeps the data-loss bug from returning: an unreadable
    /// store blocks every write.
    #[test]
    fn an_unreadable_store_blocks_saving() {
        with_temp_config(|m| {
            m.store_unreadable = Some("bad toml".to_string());
            let _ = m.update(Message::Set("input:repeat_rate", Value::Int(30)));
            let saved =
                hyprforge_input::storage::load(&hyprforge_core::paths::input_toml_path()).unwrap();
            assert!(saved.is_empty(), "nothing may be written over a store we can't read");
            assert!(m.error.is_some(), "and the user has to be told why");
        });
    }

    #[test]
    fn a_readable_store_still_saves() {
        with_temp_config(|m| {
            let _ = m.update(Message::Set("input:repeat_rate", Value::Int(30)));
            let saved =
                hyprforge_input::storage::load(&hyprforge_core::paths::input_toml_path()).unwrap();
            assert_eq!(saved.get("input:repeat_rate"), Some(&Value::Int(30)));
        });
    }

    /// Values already owned start unticked, so a re-import can't silently
    /// replace a value the user set in the app with one from their file.
    #[test]
    fn an_already_owned_import_candidate_starts_unselected() {
        with_temp_config(|m| {
            let _ = m.update(Message::Set("input:kb_layout", Value::Text("de".into())));
            let found = vec![
                Discovered {
                    key: "input:kb_layout",
                    label: "Keyboard layout",
                    value: Value::Text("us".into()),
                    already_owned: true,
                    differs: true,
                },
                Discovered {
                    key: "input:repeat_rate",
                    label: "Key repeat rate",
                    value: Value::Int(25),
                    already_owned: false,
                    differs: false,
                },
            ];
            let _ = m.update(Message::ImportLoaded(Ok(found)));
            let Some(ImportState::Ready(candidates)) = &m.import_review else {
                panic!("expected a review");
            };
            assert!(!candidates[0].selected, "already owned, so not re-imported by default");
            assert!(candidates[1].selected);
        });
    }

    #[test]
    fn importing_writes_only_the_selected_candidates() {
        with_temp_config(|m| {
            let found = vec![
                Discovered {
                    key: "input:kb_layout",
                    label: "Keyboard layout",
                    value: Value::Text("us".into()),
                    already_owned: false,
                    differs: false,
                },
                Discovered {
                    key: "input:repeat_rate",
                    label: "Key repeat rate",
                    value: Value::Int(25),
                    already_owned: false,
                    differs: false,
                },
            ];
            let _ = m.update(Message::ImportLoaded(Ok(found)));
            let _ = m.update(Message::ImportToggle(1));
            let _ = m.update(Message::ImportConfirm);
            assert!(m.owns("input:kb_layout"));
            assert!(!m.owns("input:repeat_rate"), "it was unticked");
        });
    }

    #[test]
    fn cancelling_an_import_changes_nothing() {
        with_temp_config(|m| {
            let found = vec![Discovered {
                key: "input:kb_layout",
                label: "Keyboard layout",
                value: Value::Text("us".into()),
                already_owned: false,
                differs: false,
            }];
            let _ = m.update(Message::ImportLoaded(Ok(found)));
            let _ = m.update(Message::ImportCancel);
            assert!(m.settings.is_empty());
            assert!(m.import_review.is_none());
        });
    }

    /// A value the catalog rejects can only arrive by hand-editing, and
    /// codegen skips it. Without surfacing it the user would see a line in
    /// their TOML doing nothing at all.
    #[test]
    fn an_invalid_stored_value_is_surfaced_rather_than_silently_skipped() {
        with_temp_config(|m| {
            m.settings.set("input:sensitivity", Value::Float(9.0));
            m.invalid = m.settings.validate();
            assert_eq!(m.invalid.len(), 1);
            assert_eq!(m.invalid[0].key, "input:sensitivity");
        });
    }

    #[test]
    fn the_filter_matches_label_key_and_help() {
        with_temp_config(|m| {
            let tap = catalog::get("input:touchpad:tap_to_click").unwrap();
            m.filter = "tap".to_string();
            assert!(m.matches_filter(tap));
            m.filter = "touchpad".to_string();
            assert!(m.matches_filter(tap));
            m.filter = "three fingers".to_string();
            assert!(m.matches_filter(tap), "help text should match too");
            m.filter = "bluetooth".to_string();
            assert!(!m.matches_filter(tap));
        });
    }

    /// Every catalog entry has to render. A kind the view doesn't handle
    /// would otherwise only show up as a blank row in front of a user.
    #[test]
    fn every_catalogued_setting_produces_a_row() {
        with_temp_config(|m| {
            for setting in catalog::SETTINGS {
                let _ = m.setting_row(setting, FontScale::default());
            }
        });
    }

    /// Builds the whole screen in each state it can be in. The row test
    /// above covers the controls; this covers the banners, the import
    /// panel and the empty-filter case, none of which any other test
    /// constructs.
    #[test]
    fn the_screen_builds_in_every_state() {
        with_temp_config(|m| {
            let scale = FontScale::default();
            let _ = m.view(scale);

            m.store_unreadable = Some("bad toml".to_string());
            m.error = Some("something failed".to_string());
            m.status = Some("saved".to_string());
            m.settings.set("input:sensitivity", Value::Float(9.0));
            m.invalid = m.settings.validate();
            let _ = m.view(scale);

            m.store_unreadable = None;
            m.filter = "no such setting anywhere".to_string();
            let _ = m.view(scale);
            m.filter = String::new();

            m.import_review = Some(ImportState::Running);
            let _ = m.view(scale);
            m.import_review = Some(ImportState::Ready(Vec::new()));
            let _ = m.view(scale);
            m.import_review = Some(ImportState::Ready(vec![Candidate {
                found: Discovered {
                    key: "input:kb_layout",
                    label: "Keyboard layout",
                    value: Value::Text("us".into()),
                    already_owned: true,
                    differs: true,
                },
                selected: false,
            }]));
            let _ = m.view(scale);
        });
    }

    fn live_value(key: &'static str, value: Value, set: bool) -> Live {
        Live { key, value, set }
    }

    /// The defect this pass found: an unowned row fell back to the
    /// *catalog* default, so a machine whose config turns numlock on would
    /// see the checkbox unticked while numlock was actually on. The screen
    /// has to report what's running.
    #[test]
    fn an_unowned_row_shows_the_live_value_not_the_catalog_default() {
        with_temp_config(|m| {
            let setting = catalog::get("input:numlock_by_default").unwrap();
            assert_eq!(
                m.effective(setting),
                (Value::Bool(false), Source::Default),
                "catalog default before anything is known"
            );

            let _ = m.update(Message::LiveLoaded(vec![live_value(
                "input:numlock_by_default",
                Value::Bool(true),
                true,
            )]));
            assert_eq!(
                m.effective(setting),
                (Value::Bool(true), Source::UserConfig),
                "the user's config sets it, so that's what must show"
            );
        });
    }

    /// The three sources have to stay distinguishable, because they mean
    /// different things: only one of them is a value this app writes.
    #[test]
    fn the_value_source_is_reported_precisely() {
        with_temp_config(|m| {
            let _ = m.update(Message::LiveLoaded(vec![
                live_value("input:repeat_rate", Value::Int(25), false),
                live_value("input:kb_layout", Value::Text("de".into()), true),
            ]));
            assert_eq!(
                m.effective(catalog::get("input:repeat_rate").unwrap()).1,
                Source::Default
            );
            assert_eq!(
                m.effective(catalog::get("input:kb_layout").unwrap()).1,
                Source::UserConfig
            );

            let _ = m.update(Message::Set("input:kb_layout", Value::Text("us".into())));
            assert_eq!(
                m.effective(catalog::get("input:kb_layout").unwrap()),
                (Value::Text("us".into()), Source::Owned),
                "once owned, what the user chose wins over what's live"
            );
        });
    }

    /// An owned value must keep showing even when the live read disagrees
    /// — between saving and Hyprland reloading, they legitimately differ,
    /// and the control must not flicker back to the old value.
    #[test]
    fn an_owned_value_outranks_a_stale_live_read() {
        with_temp_config(|m| {
            let _ = m.update(Message::Set("input:repeat_rate", Value::Int(45)));
            let _ = m.update(Message::LiveLoaded(vec![live_value(
                "input:repeat_rate",
                Value::Int(25),
                false,
            )]));
            assert_eq!(
                m.effective(catalog::get("input:repeat_rate").unwrap()).0,
                Value::Int(45)
            );
        });
    }

    /// A failed live read must not break the screen — every row still
    /// renders, falling back to the catalog default.
    #[test]
    fn a_failed_live_read_leaves_the_screen_usable() {
        with_temp_config(|m| {
            let _ = m.update(Message::LiveLoaded(Vec::new()));
            let _ = m.view(FontScale::default());
            assert_eq!(
                m.effective(catalog::get("input:repeat_rate").unwrap()).0,
                Value::Int(25)
            );
        });
    }

    /// Text fields go through the same resolver, so a layout the user's
    /// config sets shows up in the box rather than the catalog's "us".
    #[test]
    fn a_text_field_shows_the_live_value_too() {
        with_temp_config(|m| {
            let _ = m.update(Message::LiveLoaded(vec![live_value(
                "input:kb_layout",
                Value::Text("de,fr".into()),
                true,
            )]));
            let setting = catalog::get("input:kb_layout").unwrap();
            assert_eq!(m.shown_text(setting), "de,fr");
        });
    }

    /// The draft bar only exists while something is typed, and the Apply
    /// button is the only way to reach it — a bar that never appears would
    /// strand every text field on the screen.
    #[test]
    fn the_apply_bar_appears_only_while_a_draft_is_pending() {
        with_temp_config(|m| {
            assert!(m.drafts.is_empty());
            let _ = m.update(Message::DraftChanged("input:repeat_rate", "45".into()));
            assert!(!m.drafts.is_empty(), "the bar's condition must now hold");
            let _ = m.update(Message::ApplyDrafts);
            assert!(m.drafts.is_empty(), "and stop holding once applied");
        });
    }
}
