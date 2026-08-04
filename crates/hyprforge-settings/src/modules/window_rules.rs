use hyprforge_core::theme::{spacing, FontScale};
use hyprforge_core::widgets::{confirm_dialog, danger_button, row_field, scaled_text, section};
use hyprforge_core::SettingsModule;
use hyprforge_windowrules::model::{generate_rule_name, Effects, Matcher};
use hyprforge_windowrules::setup::SetupPlan;
use hyprforge_windowrules::Rule;
use iced::widget::{button, checkbox, column, container, row, scrollable, text_input};
use iced::{Element, Length, Task};

#[derive(Debug, Clone, Default)]
struct RuleDraft {
    editing_index: Option<usize>,
    class: String,
    title: String,
    float: bool,
    no_blur: bool,
    rounding: String,
    border_color: String,
}

impl RuleDraft {
    fn from_rule(index: usize, rule: &Rule) -> Self {
        RuleDraft {
            editing_index: Some(index),
            class: rule.matcher.class.clone().unwrap_or_default(),
            title: rule.matcher.title.clone().unwrap_or_default(),
            float: rule.effects.float.unwrap_or(false),
            no_blur: rule.effects.no_blur.unwrap_or(false),
            rounding: rule
                .effects
                .rounding
                .map(|r| r.to_string())
                .unwrap_or_default(),
            border_color: rule.effects.border_color.clone().unwrap_or_default(),
        }
    }

    fn into_matcher_effects(self) -> (Matcher, Effects) {
        let matcher = Matcher {
            class: non_empty(self.class),
            title: non_empty(self.title),
            ..Default::default()
        };
        let effects = Effects {
            float: self.float.then_some(true),
            no_blur: self.no_blur.then_some(true),
            rounding: self.rounding.trim().parse().ok(),
            border_color: non_empty(self.border_color),
            ..Default::default()
        };
        (matcher, effects)
    }
}

fn non_empty(s: String) -> Option<String> {
    let s = s.trim();
    if s.is_empty() {
        None
    } else {
        Some(s.to_string())
    }
}

#[derive(Debug, Clone)]
pub enum Message {
    Add,
    Edit(usize),
    Delete(usize),
    MoveUp(usize),
    MoveDown(usize),
    ToggleEnabled(usize),
    DraftClass(String),
    DraftTitle(String),
    DraftFloat(bool),
    DraftNoBlur(bool),
    DraftRounding(String),
    DraftBorderColor(String),
    DraftSave,
    DraftCancel,
    ConfirmSetup,
    CancelSetup,
    Reloaded(Result<(), String>),
}

pub struct WindowRulesModule {
    rules: Vec<Rule>,
    draft: Option<RuleDraft>,
    setup_plan: SetupPlan,
    show_setup_confirm: bool,
    setup_diff_text: String,
    error: Option<String>,
    status: Option<String>,
}

impl WindowRulesModule {
    pub fn new() -> (Self, Task<Message>) {
        let rules = hyprforge_windowrules::storage::load(&hyprforge_core::paths::window_rules_toml_path())
            .unwrap_or_default();
        let hyprland_lua = std::fs::read_to_string(hyprforge_core::paths::hyprland_lua_path())
            .unwrap_or_default();
        let setup_plan = hyprforge_windowrules::setup::detect(&hyprland_lua);
        let setup_diff_text = format!(
            "hyprland.lua:\n  {}   <- inserted before your existing require() calls",
            hyprforge_windowrules::setup::preview_line()
        );
        (
            WindowRulesModule {
                rules,
                draft: None,
                setup_plan,
                show_setup_confirm: false,
                setup_diff_text,
                error: None,
                status: None,
            },
            Task::none(),
        )
    }

    fn commit_draft(&mut self) {
        let Some(draft) = self.draft.take() else {
            return;
        };
        let editing_index = draft.editing_index;
        let (matcher, effects) = draft.into_matcher_effects();
        match editing_index {
            Some(i) => {
                self.rules[i].matcher = matcher;
                self.rules[i].effects = effects;
            }
            None => {
                let existing: Vec<String> = self.rules.iter().map(|r| r.name.clone()).collect();
                let label = matcher
                    .class
                    .clone()
                    .or_else(|| matcher.title.clone())
                    .unwrap_or_else(|| "rule".to_string());
                let name = generate_rule_name(&label, &existing);
                self.rules.push(Rule {
                    name,
                    enabled: true,
                    matcher,
                    effects,
                });
            }
        }
    }

    fn save_and_maybe_reload(&mut self) -> Task<Message> {
        if let Err(e) =
            hyprforge_windowrules::storage::save(&hyprforge_core::paths::window_rules_toml_path(), &self.rules)
        {
            self.error = Some(e.to_string());
            return Task::none();
        }
        if self.setup_plan == SetupPlan::AlreadyPresent {
            Task::perform(regenerate_and_reload(self.rules.clone()), Message::Reloaded)
        } else {
            self.show_setup_confirm = true;
            Task::none()
        }
    }
}

impl SettingsModule for WindowRulesModule {
    type Message = Message;

    fn title(&self) -> &str {
        "Window Rules"
    }

    fn icon(&self) -> &'static str {
        "\u{1FA9F}"
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Add => {
                self.draft = Some(RuleDraft::default());
                Task::none()
            }
            Message::Edit(i) => {
                if let Some(rule) = self.rules.get(i) {
                    self.draft = Some(RuleDraft::from_rule(i, rule));
                }
                Task::none()
            }
            Message::Delete(i) => {
                if i < self.rules.len() {
                    self.rules.remove(i);
                }
                self.save_and_maybe_reload()
            }
            Message::MoveUp(i) => {
                if i > 0 && i < self.rules.len() {
                    self.rules.swap(i - 1, i);
                }
                self.save_and_maybe_reload()
            }
            Message::MoveDown(i) => {
                if i + 1 < self.rules.len() {
                    self.rules.swap(i, i + 1);
                }
                self.save_and_maybe_reload()
            }
            Message::ToggleEnabled(i) => {
                if let Some(rule) = self.rules.get_mut(i) {
                    rule.enabled = !rule.enabled;
                }
                self.save_and_maybe_reload()
            }
            Message::DraftClass(v) => {
                if let Some(d) = &mut self.draft {
                    d.class = v;
                }
                Task::none()
            }
            Message::DraftTitle(v) => {
                if let Some(d) = &mut self.draft {
                    d.title = v;
                }
                Task::none()
            }
            Message::DraftFloat(v) => {
                if let Some(d) = &mut self.draft {
                    d.float = v;
                }
                Task::none()
            }
            Message::DraftNoBlur(v) => {
                if let Some(d) = &mut self.draft {
                    d.no_blur = v;
                }
                Task::none()
            }
            Message::DraftRounding(v) => {
                if let Some(d) = &mut self.draft {
                    d.rounding = v;
                }
                Task::none()
            }
            Message::DraftBorderColor(v) => {
                if let Some(d) = &mut self.draft {
                    d.border_color = v;
                }
                Task::none()
            }
            Message::DraftSave => {
                self.commit_draft();
                self.save_and_maybe_reload()
            }
            Message::DraftCancel => {
                self.draft = None;
                Task::none()
            }
            Message::ConfirmSetup => {
                self.show_setup_confirm = false;
                match hyprforge_windowrules::setup::install(&hyprforge_core::paths::hyprland_lua_path())
                {
                    Ok(plan) => {
                        self.setup_plan = plan;
                        Task::perform(regenerate_and_reload(self.rules.clone()), Message::Reloaded)
                    }
                    Err(e) => {
                        self.error = Some(e.to_string());
                        Task::none()
                    }
                }
            }
            Message::CancelSetup => {
                self.show_setup_confirm = false;
                Task::none()
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
        }
    }

    fn view(&self, scale: FontScale) -> Element<'_, Message> {
        if self.show_setup_confirm {
            return container(confirm_dialog(
                "Enable Hyprforge window rules",
                "This is a one-time change to your hyprland.lua: Hyprforge needs to \
                 source its generated window-rules file. Placing it first means your \
                 own rules — named or anonymous — still override Hyprforge's by \
                 default, since Hyprland evaluates named rules first and the last \
                 match wins. A backup is saved as hyprland.lua.hyprforge.bak.",
                &self.setup_diff_text,
                Message::ConfirmSetup,
                Message::CancelSetup,
            ))
            .center(Length::Fill)
            .into();
        }

        if let Some(draft) = &self.draft {
            return self.draft_view(draft, scale);
        }

        let mut content = column![scaled_text("Window Rules", 20.0, scale)].spacing(spacing::MD);

        if let Some(err) = &self.error {
            content = content.push(scaled_text(format!("Error: {err}"), 13.0, scale));
        }
        if let Some(status) = &self.status {
            content = content.push(scaled_text(status.clone(), 13.0, scale));
        }

        let mut list = column![].spacing(spacing::SM);
        if self.rules.is_empty() {
            list = list.push(scaled_text("No rules yet.", 14.0, scale));
        }
        for (i, rule) in self.rules.iter().enumerate() {
            let summary = rule
                .matcher
                .class
                .clone()
                .or_else(|| rule.matcher.title.clone())
                .unwrap_or_else(|| "(no match)".to_string());
            list = list.push(
                row![
                    checkbox(rule.enabled).on_toggle(move |_| Message::ToggleEnabled(i)),
                    scaled_text(summary, 14.0, scale).width(Length::FillPortion(2)),
                    scaled_text(rule.name.clone(), 12.0, scale).width(Length::FillPortion(2)),
                    button("Up").on_press(Message::MoveUp(i)),
                    button("Down").on_press(Message::MoveDown(i)),
                    button("Edit").on_press(Message::Edit(i)),
                    danger_button("Delete", Message::Delete(i)),
                ]
                .spacing(spacing::SM)
                .align_y(iced::Alignment::Center),
            );
        }

        content = content.push(section(
            "Rules",
            scale,
            scrollable(list).height(Length::Fill),
        ));
        content = content.push(button("Add rule").on_press(Message::Add));

        container(content).padding(spacing::LG).into()
    }
}

impl WindowRulesModule {
    fn draft_view(&self, draft: &RuleDraft, scale: FontScale) -> Element<'_, Message> {
        let title = if draft.editing_index.is_some() {
            "Edit rule"
        } else {
            "New rule"
        };
        container(
            column![
                scaled_text(title, 18.0, scale),
                row_field(
                    "Class",
                    text_input("Window class (regex)", &draft.class)
                        .on_input(Message::DraftClass),
                ),
                row_field(
                    "Title",
                    text_input("Window title (regex)", &draft.title)
                        .on_input(Message::DraftTitle),
                ),
                checkbox(draft.float).label("Float").on_toggle(Message::DraftFloat),
                checkbox(draft.no_blur)
                    .label("Disable blur")
                    .on_toggle(Message::DraftNoBlur),
                row_field(
                    "Rounding (px)",
                    text_input("e.g. 8", &draft.rounding).on_input(Message::DraftRounding),
                ),
                row_field(
                    "Border color",
                    text_input("e.g. rgb(FF0000)", &draft.border_color)
                        .on_input(Message::DraftBorderColor),
                ),
                row![
                    danger_button("Cancel", Message::DraftCancel),
                    button("Save").on_press(Message::DraftSave),
                ]
                .spacing(spacing::SM),
            ]
            .spacing(spacing::MD)
            .max_width(480.0),
        )
        .padding(spacing::LG)
        .into()
    }
}

async fn regenerate_and_reload(rules: Vec<Rule>) -> Result<(), String> {
    tokio::task::spawn_blocking(move || {
        hyprforge_windowrules::apply::apply(&hyprforge_core::paths::window_rules_lua_path(), &rules)
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}
