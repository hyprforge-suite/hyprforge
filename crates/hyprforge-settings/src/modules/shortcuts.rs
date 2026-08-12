use hyprforge_core::lua_setup;
use hyprforge_core::theme::{spacing, FontScale};
use hyprforge_core::widgets::{
    danger_button, divider, meta_text, primary_button, row_field, scaled_text, secondary_button,
    section,
};
use hyprforge_core::SettingsModule;
use hyprforge_shortcuts::model::{description_for, generate_shortcut_name};
use hyprforge_shortcuts::setup::{HyprConfig, SetupPlan};
use hyprforge_shortcuts::{Action, KeyCombo, Modifier, Shortcut};
use iced::widget::{checkbox, column, container, row, scrollable, text_input};
use iced::{Element, Length, Task};

/// The edit form's state.
///
/// Kept as plain `String`/`Vec` fields, not the parsed `KeyCombo`/`Action`
/// themselves, so a half-typed value stays on screen rather than being
/// dropped. Every field here must round-trip through [`ShortcutDraft::from_shortcut`]:
/// a field that can be written but not read back gets silently erased the
/// next time the user edits the shortcut.
#[derive(Debug, Clone)]
struct ShortcutDraft {
    editing_index: Option<usize>,
    /// The description as it's currently live in the compositor (the
    /// prefixed form `binds::conflicts_for` expects), so editing a
    /// shortcut's own chord doesn't report a conflict with itself. `None`
    /// for a shortcut that's never been saved/applied yet.
    original_description: Option<String>,
    enabled: bool,
    mods: Vec<Modifier>,
    key: String,
    dispatcher: String,
    argument: String,
    description: String,
    /// Set after a conflict check finds the chord already bound elsewhere.
    /// Cleared on any further edit, since the edit may have resolved it.
    conflict: Option<String>,
    /// True while a conflict check is in flight, so Save can't be pressed
    /// twice for the same draft.
    checking: bool,
}

impl ShortcutDraft {
    fn from_shortcut(index: usize, shortcut: &Shortcut) -> Self {
        ShortcutDraft {
            editing_index: Some(index),
            original_description: Some(description_for(shortcut)),
            enabled: shortcut.enabled,
            mods: shortcut.combo.mods.clone(),
            key: shortcut.combo.key.clone(),
            dispatcher: shortcut.action.dispatcher.clone(),
            argument: shortcut.action.argument.clone(),
            description: shortcut.description.clone(),
            conflict: None,
            checking: false,
        }
    }

    fn combo(&self) -> KeyCombo {
        KeyCombo { mods: self.mods.clone(), key: self.key.clone() }
    }

    fn into_shortcut(self, name: String) -> Shortcut {
        Shortcut {
            name,
            enabled: self.enabled,
            combo: KeyCombo { mods: self.mods, key: self.key.trim().to_string() },
            action: Action {
                dispatcher: self.dispatcher.trim().to_string(),
                argument: self.argument.trim().to_string(),
            },
            description: self.description.trim().to_string(),
        }
    }
}

impl Default for ShortcutDraft {
    fn default() -> Self {
        ShortcutDraft {
            editing_index: None,
            original_description: None,
            enabled: true,
            mods: Vec::new(),
            key: String::new(),
            dispatcher: String::new(),
            argument: String::new(),
            description: String::new(),
            conflict: None,
            checking: false,
        }
    }
}

#[derive(Debug, Clone)]
pub enum Message {
    Add,
    Edit(usize),
    Delete(usize),
    ToggleEnabled(usize),
    DraftToggleMod(Modifier, bool),
    DraftKey(String),
    DraftDispatcher(String),
    DraftArgument(String),
    DraftDescription(String),
    DraftEnabled(bool),
    DraftSave,
    DraftCancel,
    ConflictChecked(Result<Vec<String>, String>),
    Reloaded(Result<(), String>),
    ImportFromConfig,
    ImportEvaluated(hyprforge_lua_import::ImportResult),
    ImportToggle(usize, bool),
    ImportConfirm,
    ImportCancel,
}

/// A shortcut found in the user's own `hyprland.lua`, pending review
/// before it's added to the store. No `name` yet — always freshly
/// generated on confirm via [`generate_shortcut_name`], never trusted
/// from the captured call.
struct ImportCandidate {
    combo: KeyCombo,
    action: Action,
    description: String,
    checked: bool,
    /// Where the original call came from — kept so a checked-and-added
    /// candidate's hand-written source line can be removed after a
    /// successful import.
    source_path: std::path::PathBuf,
    line: Option<usize>,
}

/// The result of the last "Import from config" run, awaiting review.
#[derive(Default)]
struct ImportReview {
    shortcuts: Vec<ImportCandidate>,
    /// `(file, reason)` — files that didn't evaluate cleanly. Shown as-is
    /// rather than dropped (vision pillar #3: no dead ends).
    failures: Vec<(std::path::PathBuf, String)>,
}

pub struct ShortcutsModule {
    shortcuts: Vec<Shortcut>,
    draft: Option<ShortcutDraft>,
    /// Which Hyprland config the user has. Only [`HyprConfig::Lua`] can be
    /// served by inserting a require line, so this gates the whole setup
    /// flow rather than being a detail inside it.
    config: HyprConfig,
    /// Only meaningful when `config` is [`HyprConfig::Lua`].
    setup_plan: SetupPlan,
    error: Option<String>,
    status: Option<String>,
    /// `None` when no import is in progress or under review. `Some(None)`
    /// while the evaluator is running; `Some(Some(_))` once results are
    /// ready for the user to check through.
    import_review: Option<Option<ImportReview>>,
}

impl ShortcutsModule {
    pub fn new() -> (Self, Task<Message>) {
        let shortcuts =
            hyprforge_shortcuts::storage::load(&hyprforge_core::paths::shortcuts_toml_path())
                .unwrap_or_default();
        let mut config = hyprforge_shortcuts::setup::discover(&hyprforge_core::paths::hypr_config_dir());
        // No confirm dialog: the require line only ever points at
        // Hyprforge's own generated file, never touches the user's own
        // content, so there's nothing to ask permission for beyond what
        // installing this module already implies. A real write failure
        // still surfaces as an error rather than being silently swallowed.
        let mut error = None;
        match &config {
            HyprConfig::Missing => {
                let path = hyprforge_core::paths::hyprland_lua_path();
                match hyprforge_shortcuts::setup::create_lua_config(&path) {
                    Ok(()) => config = HyprConfig::Lua(path),
                    Err(e) => error = Some(e.to_string()),
                }
            }
            HyprConfig::Lua(_) => {
                if let Err(e) =
                    hyprforge_shortcuts::setup::install(&hyprforge_core::paths::hyprland_lua_path())
                {
                    error = Some(e.to_string());
                }
            }
            HyprConfig::ConfOnly(_) => {}
        }
        let setup_plan = match &config {
            HyprConfig::Lua(path) => {
                let contents = std::fs::read_to_string(path).unwrap_or_default();
                hyprforge_shortcuts::setup::detect(&contents)
            }
            _ => SetupPlan::NeedsInsert { insert_before_line: 1 },
        };
        // The require line can exist (just installed above, or from a
        // prior session) before this module has ever written its own
        // generated file — e.g. before the first shortcut is ever saved.
        // A require() pointing at a file that doesn't exist yet errors on
        // the user's next `hyprctl reload`, so this ensures it's at least
        // present and empty until a real save writes real content.
        if matches!(config, HyprConfig::Lua(_)) {
            let lua_path = hyprforge_core::paths::keybinds_lua_path();
            if !lua_path.exists() {
                let _ = hyprforge_core::paths::write_atomic(
                    &lua_path,
                    &hyprforge_shortcuts::codegen::generate(&[]),
                );
            }
        }
        (
            ShortcutsModule {
                shortcuts,
                draft: None,
                config,
                setup_plan,
                error,
                status: None,
                import_review: None,
            },
            Task::none(),
        )
    }

    fn edit_draft(&mut self, f: impl FnOnce(&mut ShortcutDraft)) -> Task<Message> {
        if let Some(d) = &mut self.draft {
            f(d);
            // Any edit can change whether the chord conflicts, so a stale
            // warning from a previous combo must not linger.
            d.conflict = None;
        }
        Task::none()
    }

    fn commit_draft(&mut self) {
        let Some(draft) = self.draft.take() else {
            return;
        };
        let editing_index = draft.editing_index;
        let label = if draft.description.trim().is_empty() {
            draft.key.clone()
        } else {
            draft.description.clone()
        };
        match editing_index {
            Some(i) => {
                let name = self.shortcuts[i].name.clone();
                self.shortcuts[i] = draft.into_shortcut(name);
            }
            None => {
                let existing: Vec<String> = self.shortcuts.iter().map(|s| s.name.clone()).collect();
                let name = generate_shortcut_name(&label, &existing);
                self.shortcuts.push(draft.into_shortcut(name));
            }
        }
    }

    fn save_and_maybe_reload(&mut self) -> Task<Message> {
        if let Err(e) = hyprforge_shortcuts::storage::save(
            &hyprforge_core::paths::shortcuts_toml_path(),
            &self.shortcuts,
        ) {
            self.error = Some(e.to_string());
            return Task::none();
        }
        // Without a Lua config there is nothing to source the generated file
        // from, so reloading would be a no-op dressed up as success. The
        // TOML is still saved — shortcuts authored now take effect as soon
        // as this is resolved (informational notice only, nothing to click).
        if !matches!(self.config, HyprConfig::Lua(_)) {
            self.error = None;
            self.status =
                Some("Saved. Shortcuts take effect once Hyprland setup is finished — see above.".to_string());
            return Task::none();
        }
        // The require line is installed automatically on open; this only
        // retries if that attempt failed (e.g. a transient permission
        // issue) rather than asking the user to confirm anything.
        if self.setup_plan != SetupPlan::AlreadyPresent {
            match hyprforge_shortcuts::setup::install(&hyprforge_core::paths::hyprland_lua_path()) {
                Ok(plan) => self.setup_plan = plan,
                Err(e) => {
                    self.error = Some(e.to_string());
                    return Task::none();
                }
            }
        }
        Task::perform(regenerate_and_reload(self.shortcuts.clone()), Message::Reloaded)
    }

    /// The recovery banner for the two config shapes the require-line flow
    /// can't serve. Never blocks editing — shortcuts still save to TOML and
    /// activate once setup is done (vision pillar #3: no dead ends).
    /// `Missing` normally never reaches here — `new()` creates a minimal
    /// `hyprland.lua` automatically — so seeing it means that attempt
    /// failed; the reason is already in `self.error`, shown generically
    /// above this.
    fn setup_notice(&self, scale: FontScale) -> Option<Element<'_, Message>> {
        let body = match &self.config {
            HyprConfig::Lua(_) => return None,
            HyprConfig::Missing => column![scaled_text(
                "No Hyprland config found, and Hyprforge couldn't create one \
                 automatically — see the error above.",
                13.0,
                scale,
            )],
            HyprConfig::ConfOnly(path) => column![
                scaled_text(
                    "You're using Hyprland's hyprland.conf format. Hyprforge \
                     generates Lua binds and sources them with require(), which \
                     only the Lua config supports — so it won't modify your .conf.",
                    13.0,
                    scale,
                ),
                meta_text(path.display().to_string(), 12.0, scale),
                scaled_text(
                    "Hyprland switched its config language from hyprlang to Lua \
                     in 0.55. To use this module, port your settings into a \
                     hyprland.lua; Hyprforge will pick it up automatically on \
                     next launch. Your .conf is left untouched either way.",
                    13.0,
                    scale,
                ),
            ],
        };
        Some(section("Setup required", scale, body.spacing(spacing::SM)))
    }
}

impl SettingsModule for ShortcutsModule {
    type Message = Message;

    fn title(&self) -> &str {
        "Shortcuts"
    }

    fn icon(&self) -> &'static str {
        "\u{2328}"
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Add => {
                self.draft = Some(ShortcutDraft::default());
                Task::none()
            }
            Message::Edit(i) => {
                if let Some(shortcut) = self.shortcuts.get(i) {
                    self.draft = Some(ShortcutDraft::from_shortcut(i, shortcut));
                }
                Task::none()
            }
            Message::Delete(i) => {
                if i < self.shortcuts.len() {
                    self.shortcuts.remove(i);
                }
                self.save_and_maybe_reload()
            }
            Message::ToggleEnabled(i) => {
                if let Some(shortcut) = self.shortcuts.get_mut(i) {
                    shortcut.enabled = !shortcut.enabled;
                }
                self.save_and_maybe_reload()
            }
            Message::DraftToggleMod(m, on) => self.edit_draft(|d| {
                if on {
                    if !d.mods.contains(&m) {
                        d.mods.push(m);
                    }
                } else {
                    d.mods.retain(|existing| *existing != m);
                }
            }),
            Message::DraftKey(v) => self.edit_draft(|d| d.key = v),
            Message::DraftDispatcher(v) => self.edit_draft(|d| d.dispatcher = v),
            Message::DraftArgument(v) => self.edit_draft(|d| d.argument = v),
            Message::DraftDescription(v) => self.edit_draft(|d| d.description = v),
            Message::DraftEnabled(v) => self.edit_draft(|d| d.enabled = v),
            Message::DraftSave => {
                let Some(draft) = &self.draft else {
                    return Task::none();
                };
                if draft.checking {
                    return Task::none();
                }
                let combo = draft.combo();
                let own_description = draft.original_description.clone();
                if let Some(d) = &mut self.draft {
                    d.checking = true;
                }
                Task::perform(check_conflicts(combo, own_description), Message::ConflictChecked)
            }
            Message::DraftCancel => {
                self.draft = None;
                Task::none()
            }
            Message::ConflictChecked(result) => {
                let Some(draft) = &mut self.draft else {
                    return Task::none();
                };
                draft.checking = false;
                match result {
                    Ok(conflicts) if !conflicts.is_empty() => {
                        draft.conflict = Some(format!(
                            "Already bound to: {}",
                            conflicts.join(", ")
                        ));
                        Task::none()
                    }
                    // Either genuinely free, or Hyprland isn't reachable to
                    // check against — in the latter case there's nothing to
                    // conflict with, so saving proceeds rather than
                    // stalling behind a check that can never complete
                    // (vision pillar #3: no dead ends).
                    Ok(_) | Err(_) => {
                        self.commit_draft();
                        self.save_and_maybe_reload()
                    }
                }
            }
            Message::Reloaded(Ok(())) => {
                self.status = Some("Saved and reloaded.".to_string());
                self.error = None;
                Task::none()
            }
            Message::Reloaded(Err(e)) => {
                self.error = Some(e);
                Task::none()
            }
            Message::ImportFromConfig => {
                self.import_review = Some(None);
                Task::perform(import_from_config(), Message::ImportEvaluated)
            }
            Message::ImportEvaluated(result) => {
                let hyprforge_dir = hyprforge_core::paths::hypr_hyprforge_dir();
                let mut shortcuts = Vec::new();
                for call in &result.calls {
                    // Hyprforge's own generated file is `require()`d from
                    // hyprland.lua too, so it gets evaluated right along
                    // with the user's own — excluded here, or every
                    // existing shortcut would show up as importable again.
                    if call.source_path.starts_with(&hyprforge_dir) {
                        continue;
                    }
                    if let Some((combo, action, description)) =
                        hyprforge_shortcuts::import::shortcut_from_call(&call.kind, &call.args)
                    {
                        shortcuts.push(ImportCandidate {
                            combo,
                            action,
                            description,
                            checked: true,
                            source_path: call.source_path.clone(),
                            line: call.line,
                        });
                    }
                }
                self.import_review = Some(Some(ImportReview { shortcuts, failures: result.failures }));
                Task::none()
            }
            Message::ImportToggle(i, checked) => {
                if let Some(Some(review)) = &mut self.import_review {
                    if let Some(candidate) = review.shortcuts.get_mut(i) {
                        candidate.checked = checked;
                    }
                }
                Task::none()
            }
            Message::ImportCancel => {
                self.import_review = None;
                Task::none()
            }
            Message::ImportConfirm => {
                // Collected as we go, so removal only ever targets a line
                // that actually became a stored shortcut.
                let mut to_remove: Vec<(std::path::PathBuf, usize)> = Vec::new();
                if let Some(Some(review)) = self.import_review.take() {
                    for candidate in review.shortcuts.into_iter().filter(|c| c.checked) {
                        let existing: Vec<String> = self.shortcuts.iter().map(|s| s.name.clone()).collect();
                        let label = if candidate.description.trim().is_empty() {
                            candidate.combo.key.clone()
                        } else {
                            candidate.description.clone()
                        };
                        let name = generate_shortcut_name(&label, &existing);
                        if let Some(line) = candidate.line {
                            to_remove.push((candidate.source_path.clone(), line));
                        }
                        self.shortcuts.push(Shortcut {
                            name,
                            enabled: true,
                            combo: candidate.combo,
                            action: candidate.action,
                            description: candidate.description,
                        });
                    }
                }
                // Best-effort and silent on failure: the entries are
                // already safely in the store either way, and
                // `remove_matched_lines` only ever removes a line that
                // still verifiably looks like the exact call it recorded.
                let _ = lua_setup::remove_matched_lines(&to_remove);
                self.save_and_maybe_reload()
            }
        }
    }

    fn view(&self, scale: FontScale) -> Element<'_, Message> {
        if let Some(draft) = &self.draft {
            return self.draft_view(draft, scale);
        }

        if let Some(review) = &self.import_review {
            return self.import_review_view(review.as_ref(), scale);
        }

        let mut content = column![scaled_text("Shortcuts", 22.0, scale)].spacing(spacing::LG);

        if let Some(notice) = self.setup_notice(scale) {
            content = content.push(notice);
        }

        if let Some(err) = &self.error {
            content = content.push(scaled_text(format!("Error: {err}"), 13.0, scale));
        }
        if let Some(status) = &self.status {
            content = content.push(meta_text(status.clone(), 13.0, scale));
        }

        let mut list = column![].spacing(spacing::SM);
        if self.shortcuts.is_empty() {
            list = list.push(meta_text("No shortcuts yet.", 14.0, scale));
        }
        for (i, shortcut) in self.shortcuts.iter().enumerate() {
            if i > 0 {
                list = list.push(divider());
            }
            let label = if shortcut.description.trim().is_empty() {
                shortcut.combo.to_bind_string()
            } else {
                shortcut.description.clone()
            };
            let info = column![
                scaled_text(label, 14.0, scale),
                meta_text(
                    format!("{}  →  {}", shortcut.combo.to_bind_string(), shortcut.action.dispatcher),
                    12.0,
                    scale,
                ),
            ]
            .spacing(spacing::XS)
            .width(Length::Fill);
            list = list.push(
                container(
                    row![
                        checkbox(shortcut.enabled).on_toggle(move |_| Message::ToggleEnabled(i)),
                        info,
                        secondary_button("Edit").on_press(Message::Edit(i)),
                        danger_button("Delete", Message::Delete(i)),
                    ]
                    .spacing(spacing::SM)
                    .align_y(iced::Alignment::Center),
                )
                .padding([spacing::SM, 0.0]),
            );
        }

        content = content.push(section(
            "Shortcuts",
            scale,
            container(scrollable(list).width(Length::Fill).height(Length::Shrink))
                .max_height(420.0),
        ));
        content = content.push(
            container(
                row![
                    secondary_button("Import from config").on_press(Message::ImportFromConfig),
                    primary_button("Add shortcut").on_press(Message::Add),
                ]
                .spacing(spacing::SM),
            )
            .width(Length::Fill)
            .align_x(iced::alignment::Horizontal::Right),
        );

        container(content).padding(spacing::LG).into()
    }
}

impl ShortcutsModule {
    /// The "Import from config" flow's loading state and review list.
    /// Reachable any time, not just on first run — opt-in and re-runnable
    /// whenever the user wants to pick up hand-written binds added since
    /// the last import.
    fn import_review_view(&self, review: Option<&ImportReview>, scale: FontScale) -> Element<'_, Message> {
        let Some(review) = review else {
            return container(scaled_text("Reading your hyprland.lua…", 14.0, scale))
                .padding(spacing::LG)
                .into();
        };

        let mut body = column![scaled_text("Import from config", 22.0, scale)].spacing(spacing::LG);

        if !review.failures.is_empty() {
            let mut failures = column![meta_text(
                format!(
                    "{} file{} couldn't be imported:",
                    review.failures.len(),
                    if review.failures.len() == 1 { "" } else { "s" }
                ),
                13.0,
                scale,
            )]
            .spacing(spacing::XS);
            for (path, reason) in &review.failures {
                failures = failures.push(meta_text(format!("{} — {reason}", path.display()), 12.0, scale));
            }
            body = body.push(section("Couldn't import", scale, failures.spacing(spacing::XS)));
        }

        if review.shortcuts.is_empty() {
            body = body.push(meta_text("No importable shortcuts found.", 14.0, scale));
        } else {
            let mut list = column![].spacing(spacing::SM);
            for (i, candidate) in review.shortcuts.iter().enumerate() {
                let summary = if candidate.description.trim().is_empty() {
                    candidate.combo.to_bind_string()
                } else {
                    candidate.description.clone()
                };
                let detail = if candidate.action.dispatcher.trim().is_empty() {
                    format!(
                        "{}  →  (action not recoverable — bound to something other than hl.dsp.*)",
                        candidate.combo.to_bind_string()
                    )
                } else {
                    format!("{}  →  {}", candidate.combo.to_bind_string(), candidate.action.dispatcher)
                };
                list = list.push(
                    row![
                        checkbox(candidate.checked).on_toggle(move |v| Message::ImportToggle(i, v)),
                        column![
                            scaled_text(summary, 14.0, scale),
                            meta_text(detail, 12.0, scale),
                        ]
                        .spacing(spacing::XS)
                        .width(Length::Fill),
                    ]
                    .spacing(spacing::SM)
                    .align_y(iced::Alignment::Center),
                );
            }
            body = body.push(section("Shortcuts", scale, list));
        }

        body = body.push(meta_text(
            "Checked entries are added here and their original line is removed \
             from your config — only when it still matches exactly what was \
             imported, and only after it's safely added. A multi-line bind, or \
             one edited since this list was generated, is left in place instead \
             of guessed at.",
            12.0,
            scale,
        ));

        body = body.push(
            container(
                row![
                    secondary_button("Cancel").on_press(Message::ImportCancel),
                    primary_button("Add checked").on_press(Message::ImportConfirm),
                ]
                .spacing(spacing::SM),
            )
            .width(Length::Fill)
            .align_x(iced::alignment::Horizontal::Right),
        );

        container(body).padding(spacing::LG).into()
    }

    fn draft_view(&self, draft: &ShortcutDraft, scale: FontScale) -> Element<'_, Message> {
        let title = if draft.editing_index.is_some() {
            "Edit shortcut"
        } else {
            "New shortcut"
        };

        let mut mod_row = row![].spacing(spacing::SM);
        for m in Modifier::ALL {
            let on = draft.mods.contains(&m);
            mod_row = mod_row.push(
                checkbox(on)
                    .label(m.keyword())
                    .on_toggle(move |v| Message::DraftToggleMod(m, v)),
            );
        }

        let mut form = column![
            row_field("Modifiers", mod_row),
            row_field(
                "Key",
                text_input("e.g. Q, Return, XF86AudioRaiseVolume", &draft.key)
                    .on_input(Message::DraftKey),
            ),
            row_field(
                "Dispatcher",
                text_input("e.g. window.close, exec_cmd", &draft.dispatcher)
                    .on_input(Message::DraftDispatcher),
            ),
            row_field(
                "Argument",
                text_input("Lua expression, e.g. [[ghostty]]", &draft.argument)
                    .on_input(Message::DraftArgument),
            ),
            row_field(
                "Description",
                text_input("Shown in hyprctl binds", &draft.description)
                    .on_input(Message::DraftDescription),
            ),
            checkbox(draft.enabled).label("Enabled").on_toggle(Message::DraftEnabled),
        ]
        .spacing(spacing::MD);

        if let Some(conflict) = &draft.conflict {
            form = form.push(meta_text(conflict.clone(), 12.0, scale));
        }

        let mut body = column![scaled_text(title, 22.0, scale), section("Shortcut", scale, form)]
            .spacing(spacing::LG)
            .max_width(520.0);

        body = body.push(
            container(
                row![
                    secondary_button("Cancel").on_press(Message::DraftCancel),
                    primary_button(if draft.checking { "Checking…" } else { "Save" })
                        .on_press(Message::DraftSave),
                ]
                .spacing(spacing::SM),
            )
            .width(Length::Fill)
            .align_x(iced::alignment::Horizontal::Right),
        );

        // No scrollable here — the app shell already wraps every screen in
        // one. A second, content-sized scrollable nested inside it puts a
        // stray scrollbar partway across the window.
        container(body).padding(spacing::LG).into()
    }
}

/// Checks whether `combo` is already bound in the live compositor, excluding
/// the shortcut's own bind (if any). Blocking work (it shells out to
/// `hyprctl`), so it goes on the blocking pool rather than stalling the UI
/// thread — same treatment `regenerate_and_reload` gets below.
async fn check_conflicts(
    combo: KeyCombo,
    own_description: Option<String>,
) -> Result<Vec<String>, String> {
    tokio::task::spawn_blocking(move || {
        let binds = hyprforge_shortcuts::binds::list_binds().map_err(|e| e.to_string())?;
        Ok(hyprforge_shortcuts::binds::conflicts_for(&binds, &combo, own_description.as_deref())
            .into_iter()
            .map(|b| {
                let label = b.label();
                if label.trim().is_empty() {
                    "an existing bind".to_string()
                } else {
                    label.to_string()
                }
            })
            .collect())
    })
    .await
    .map_err(|e| e.to_string())?
}

async fn regenerate_and_reload(shortcuts: Vec<Shortcut>) -> Result<(), String> {
    tokio::task::spawn_blocking(move || {
        hyprforge_shortcuts::apply::apply(&hyprforge_core::paths::keybinds_lua_path(), &shortcuts)
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Evaluates the user's own `hyprland.lua` for importable shortcuts.
/// Blocking work (a synchronous Lua VM run), so it goes on the blocking
/// pool rather than stalling the UI thread — same treatment the conflict
/// check gets.
async fn import_from_config() -> hyprforge_lua_import::ImportResult {
    let hypr_dir = hyprforge_core::paths::hypr_config_dir();
    tokio::task::spawn_blocking(move || hyprforge_lua_import::evaluate(&hypr_dir))
        .await
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fully_populated_shortcut() -> Shortcut {
        Shortcut {
            name: "hyprforge-test-1".to_string(),
            enabled: true,
            combo: KeyCombo { mods: vec![Modifier::Super, Modifier::Shift], key: "Q".to_string() },
            action: Action { dispatcher: "window.close".to_string(), argument: "1".to_string() },
            description: "Close window".to_string(),
        }
    }

    /// The invariant that keeps editing non-destructive: anything the form
    /// can hold must survive load → save unchanged.
    #[test]
    fn draft_round_trips_every_field() {
        let shortcut = fully_populated_shortcut();
        let draft = ShortcutDraft::from_shortcut(0, &shortcut);
        let name = shortcut.name.clone();
        let rebuilt = draft.into_shortcut(name);
        assert_eq!(rebuilt, shortcut);
    }

    #[test]
    fn blank_draft_produces_an_empty_shortcut() {
        let draft = ShortcutDraft::default();
        let shortcut = draft.into_shortcut("hyprforge-x-1".to_string());
        assert!(shortcut.combo.is_empty());
        assert!(shortcut.action.is_empty());
    }

    #[test]
    fn toggling_a_modifier_off_removes_only_that_one() {
        let mut draft = ShortcutDraft {
            mods: vec![Modifier::Super, Modifier::Shift],
            ..Default::default()
        };
        draft.mods.retain(|m| *m != Modifier::Shift);
        assert_eq!(draft.mods, vec![Modifier::Super]);
    }

    #[test]
    fn original_description_is_carried_for_conflict_exclusion() {
        let shortcut = fully_populated_shortcut();
        let draft = ShortcutDraft::from_shortcut(0, &shortcut);
        assert_eq!(draft.original_description, Some(description_for(&shortcut)));
    }

    #[test]
    fn a_new_draft_has_no_original_description() {
        assert_eq!(ShortcutDraft::default().original_description, None);
    }
}
