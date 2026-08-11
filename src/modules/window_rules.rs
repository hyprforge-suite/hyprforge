use hyprforge_core::theme::{spacing, FontScale};
use hyprforge_core::widgets::{
    confirm_dialog, danger_button, divider, meta_text, primary_button, row_field, scaled_text,
    secondary_button, section, tri_state,
};
use hyprforge_core::SettingsModule;
use hyprforge_windowrules::clients::Client;
use hyprforge_windowrules::model::{
    generate_rule_name, Effects, Matcher, Opacity, Workspace, WorkspaceRule,
};
use hyprforge_windowrules::monitors::Monitor;
use hyprforge_windowrules::setup::{HyprConfig, SetupPlan};
use hyprforge_windowrules::Rule;
use iced::widget::{checkbox, column, container, row, scrollable, text_input};
use iced::{Element, Length, Task};

/// The edit form's state.
///
/// Everything is kept as a `String` while editing — including the numeric
/// and expression fields — so a half-typed value stays on screen instead of
/// being silently dropped by a failed parse. Conversion happens once, in
/// [`RuleDraft::into_matcher_effects`].
///
/// Every field here must round-trip through [`RuleDraft::from_rule`]: a
/// field that can be written but not read back gets silently erased the next
/// time the user edits the rule.
#[derive(Debug, Clone, Default)]
struct RuleDraft {
    editing_index: Option<usize>,
    class: String,
    title: String,
    initial_class: String,
    initial_title: String,
    /// Tri-state: `None` means "don't match on this at all", which is
    /// distinct from `Some(false)` — `floating = false` is a real matcher
    /// that selects tiled windows.
    fullscreen: Option<bool>,
    floating: Option<bool>,
    xwayland: Option<bool>,
    match_tag: String,
    content: String,
    workspace: String,
    workspace_silent: bool,
    tag: String,
    float: bool,
    no_blur: bool,
    rounding: String,
    border_color: String,
    move_x: String,
    move_y: String,
    size_w: String,
    size_h: String,
    opacity_active: String,
    opacity_inactive: String,
    opacity_fullscreen: String,
    opacity_override: bool,
}

impl RuleDraft {
    fn from_rule(index: usize, rule: &Rule) -> Self {
        let m = &rule.matcher;
        let e = &rule.effects;
        let pair = |v: &Option<[String; 2]>| match v {
            Some([a, b]) => (a.clone(), b.clone()),
            None => (String::new(), String::new()),
        };
        let (move_x, move_y) = pair(&e.r#move);
        let (size_w, size_h) = pair(&e.size);
        let opacity = |v: Option<f32>| v.map(|f| f.to_string()).unwrap_or_default();

        RuleDraft {
            editing_index: Some(index),
            class: m.class.clone().unwrap_or_default(),
            title: m.title.clone().unwrap_or_default(),
            initial_class: m.initial_class.clone().unwrap_or_default(),
            initial_title: m.initial_title.clone().unwrap_or_default(),
            fullscreen: m.fullscreen,
            floating: m.floating,
            xwayland: m.xwayland,
            match_tag: m.tag.clone().unwrap_or_default(),
            content: m.content.clone().unwrap_or_default(),
            workspace: e.workspace.name.clone(),
            workspace_silent: e.workspace.silent,
            tag: e.tag.clone().unwrap_or_default(),
            float: e.float.unwrap_or(false),
            no_blur: e.no_blur.unwrap_or(false),
            rounding: e.rounding.map(|r| r.to_string()).unwrap_or_default(),
            border_color: e.border_color.clone().unwrap_or_default(),
            move_x,
            move_y,
            size_w,
            size_h,
            opacity_active: opacity(e.opacity.active),
            opacity_inactive: opacity(e.opacity.inactive),
            opacity_fullscreen: opacity(e.opacity.fullscreen),
            opacity_override: e.opacity.is_override,
        }
    }

    fn into_matcher_effects(self) -> (Matcher, Effects) {
        let matcher = Matcher {
            class: non_empty(self.class),
            title: non_empty(self.title),
            initial_class: non_empty(self.initial_class),
            initial_title: non_empty(self.initial_title),
            fullscreen: self.fullscreen,
            floating: self.floating,
            xwayland: self.xwayland,
            tag: non_empty(self.match_tag),
            content: non_empty(self.content),
        };
        let effects = Effects {
            // The silent flag is meaningless without a workspace, so it's
            // dropped along with a blank one rather than persisting as a
            // setting with nothing to apply to.
            workspace: Workspace {
                name: non_empty(self.workspace).unwrap_or_default(),
                silent: self.workspace_silent,
            },
            tag: non_empty(self.tag),
            float: self.float.then_some(true),
            no_blur: self.no_blur.then_some(true),
            rounding: self.rounding.trim().parse().ok(),
            border_color: non_empty(self.border_color),
            // move/size are two-part; a half-filled pair isn't expressible
            // in Hyprland's `{ x, y }` form, so it's dropped rather than
            // guessed at. Values pass through as typed — the codegen decides
            // literal-vs-expression quoting.
            r#move: pair_or_none(self.move_x, self.move_y),
            size: pair_or_none(self.size_w, self.size_h),
            opacity: Opacity {
                active: parse_opacity(&self.opacity_active),
                inactive: parse_opacity(&self.opacity_inactive),
                fullscreen: parse_opacity(&self.opacity_fullscreen),
                is_override: self.opacity_override,
            },
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

fn pair_or_none(a: String, b: String) -> Option<[String; 2]> {
    match (non_empty(a), non_empty(b)) {
        (Some(a), Some(b)) => Some([a, b]),
        _ => None,
    }
}

fn parse_opacity(s: &str) -> Option<f32> {
    s.trim().parse().ok()
}

/// One entry in a pin's monitor dropdown.
///
/// Carries the `desc:`-style selector that gets stored alongside the label
/// that gets shown, so the widget never has to reconstruct one from the
/// other — reconstructing is where a description that Hyprland won't
/// recognise would creep in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MonitorChoice {
    label: String,
    selector: String,
}

impl std::fmt::Display for MonitorChoice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.label)
    }
}

impl MonitorChoice {
    fn from_monitor(m: &Monitor) -> Self {
        MonitorChoice {
            label: m.label(),
            selector: m.rule_selector(),
        }
    }

    /// A selector already stored in a pin, for a monitor that isn't connected
    /// right now. Shown as-is so unplugging a display doesn't make its pin
    /// look empty — or worse, let an edit silently clear it.
    fn from_stored(selector: &str) -> Self {
        MonitorChoice {
            label: format!("{selector} (not connected)"),
            selector: selector.to_string(),
        }
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
    DraftInitialClass(String),
    DraftInitialTitle(String),
    DraftFullscreen(Option<bool>),
    DraftFloating(Option<bool>),
    DraftXwayland(Option<bool>),
    DraftMatchTag(String),
    DraftContent(String),
    DraftWorkspace(String),
    DraftWorkspaceSilent(bool),
    DraftTag(String),
    DraftFloat(bool),
    DraftNoBlur(bool),
    DraftRounding(String),
    DraftBorderColor(String),
    DraftMoveX(String),
    DraftMoveY(String),
    DraftSizeW(String),
    DraftSizeH(String),
    DraftOpacityActive(String),
    DraftOpacityInactive(String),
    DraftOpacityFullscreen(String),
    DraftOpacityOverride(bool),
    ToggleAdvanced,
    OpenPicker,
    ClosePicker,
    ClientsLoaded(Result<Vec<Client>, String>),
    /// Index into `picker`'s loaded list — the class is taken from there
    /// rather than carried in the message, so a stale click can't inject a
    /// class that isn't on screen.
    PickClass(usize),
    PickTitle(usize),
    AddWorkspacePin,
    DeleteWorkspacePin(usize),
    PinWorkspace(usize, String),
    PinMonitor(usize, MonitorChoice),
    PinDefault(usize, bool),
    PinPersistent(usize, bool),
    MonitorsLoaded(Vec<Monitor>),
    DraftSave,
    DraftCancel,
    ConfirmSetup,
    CancelSetup,
    ConfirmCreateLuaConfig,
    Reloaded(Result<(), String>),
}

pub struct WindowRulesModule {
    rules: Vec<Rule>,
    /// Workspace→monitor pins. Held beside `rules` rather than inside a
    /// `storage::Rules` so the list operations below can keep indexing one
    /// flat Vec; the two are recombined on save.
    workspace_rules: Vec<WorkspaceRule>,
    draft: Option<RuleDraft>,
    /// Which Hyprland config the user has. `None` of the non-`Lua` variants
    /// can be served by inserting a require line, so this gates the whole
    /// setup flow rather than being a detail inside it.
    config: HyprConfig,
    /// Only meaningful when `config` is [`HyprConfig::Lua`].
    setup_plan: SetupPlan,
    show_setup_confirm: bool,
    /// Collapsed by default — the raw Hyprland vocabulary (move/size
    /// expressions, per-state opacity, xwayland matching) is the power-user
    /// surface, kept out of the way of "float Discord" (vision pillar #1).
    show_advanced: bool,
    /// Monitors to choose from when pinning a workspace. Empty until the
    /// `hyprctl` read returns, or if it fails — the pin's monitor is then
    /// simply not selectable, which beats blocking the whole section.
    monitors: Vec<Monitor>,
    /// The open-window picker, when it's showing. `None` means closed, which
    /// is distinct from `Some(Loading)` — the panel has to be on screen while
    /// the `hyprctl` call is in flight or the button looks dead.
    picker: Option<PickerState>,
    setup_diff_text: String,
    error: Option<String>,
    status: Option<String>,
}

/// Where the window picker is in its one-shot load.
enum PickerState {
    Loading,
    Loaded(Vec<Client>),
    /// Hyprland isn't running, or `hyprctl` isn't installed. Not an error
    /// dialog: the class and title fields still work by hand, so this is a
    /// note in the panel and nothing more (vision pillar #3 — the dead end
    /// would be a modal you can't get past to type the class yourself).
    Unavailable(String),
}

impl WindowRulesModule {
    pub fn new() -> (Self, Task<Message>) {
        let stored = hyprforge_windowrules::storage::load(&hyprforge_core::paths::window_rules_toml_path())
            .unwrap_or_default();
        let (rules, workspace_rules) = (stored.rules, stored.workspace_rules);
        let config = hyprforge_windowrules::setup::discover(&hyprforge_core::paths::hypr_config_dir());
        // Only a real hyprland.lua can be inspected for the require line;
        // for the other variants this stays at the "not installed" default
        // and the view routes to a recovery screen instead of the dialog.
        let setup_plan = match &config {
            HyprConfig::Lua(path) => {
                let contents = std::fs::read_to_string(path).unwrap_or_default();
                hyprforge_windowrules::setup::detect(&contents)
            }
            _ => SetupPlan::NeedsInsert {
                insert_before_line: 1,
            },
        };
        let setup_diff_text = format!(
            "hyprland.lua:\n  {}   <- inserted before your existing require() calls",
            hyprforge_windowrules::setup::preview_line()
        );
        (
            WindowRulesModule {
                rules,
                workspace_rules,
                draft: None,
                config,
                setup_plan,
                show_setup_confirm: false,
                show_advanced: false,
                monitors: Vec::new(),
                picker: None,
                setup_diff_text,
                error: None,
                status: None,
            },
            // Read the monitors up front: the workspace-pin dropdown needs
            // them, and a failure here is silent by design — the section
            // still lists existing pins, it just can't offer new monitors.
            Task::perform(load_monitors(), Message::MonitorsLoaded),
        )
    }

    /// Applies `f` to the open draft, if there is one. Keeps the ~19 field
    /// -edit message arms to one line each.
    /// Applies `f` to the pin at `index`, then persists.
    ///
    /// A blank pin is legitimate mid-edit — you add a row before you've typed
    /// a workspace into it — so an incomplete one is saved as-is and simply
    /// skipped by the codegen rather than blocking the save.
    fn edit_pin(&mut self, index: usize, f: impl FnOnce(&mut WorkspaceRule)) -> Task<Message> {
        let Some(pin) = self.workspace_rules.get_mut(index) else {
            return Task::none();
        };
        f(pin);
        self.save_and_maybe_reload()
    }

    /// Both rule kinds as storage and codegen want them. The module keeps
    /// them apart while editing because only window rules are reorderable.
    fn stored_rules(&self) -> hyprforge_windowrules::storage::Rules {
        hyprforge_windowrules::storage::Rules {
            rules: self.rules.clone(),
            workspace_rules: self.workspace_rules.clone(),
        }
    }

    /// The window at `index` in the picker's loaded list, if the picker is
    /// still showing that list. Indices are only meaningful against the
    /// snapshot they were rendered from.
    fn picked_client(&self, index: usize) -> Option<&Client> {
        match &self.picker {
            Some(PickerState::Loaded(clients)) => clients.get(index),
            _ => None,
        }
    }

    fn edit_draft(&mut self, f: impl FnOnce(&mut RuleDraft)) -> Task<Message> {
        if let Some(d) = &mut self.draft {
            f(d);
        }
        Task::none()
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
            hyprforge_windowrules::storage::save(
                &hyprforge_core::paths::window_rules_toml_path(),
                &self.stored_rules(),
            )
        {
            self.error = Some(e.to_string());
            return Task::none();
        }
        // Without a Lua config there is nothing to source the generated file
        // from, so reloading would be a no-op dressed up as success. The TOML
        // is still saved — rules authored now take effect as soon as the
        // banner's setup step is done.
        if !matches!(self.config, HyprConfig::Lua(_)) {
            self.error = None;
            self.status = Some("Saved. Rules take effect once Hyprland setup is finished — see above.".to_string());
            return Task::none();
        }
        if self.setup_plan == SetupPlan::AlreadyPresent {
            Task::perform(regenerate_and_reload(self.stored_rules()), Message::Reloaded)
        } else {
            self.show_setup_confirm = true;
            Task::none()
        }
    }

    /// The recovery banner for the two config shapes the require-line flow
    /// can't serve. Never blocks rule editing — rules still save to TOML and
    /// activate once setup is done (vision pillar #3: no dead ends).
    fn setup_notice(&self, scale: FontScale) -> Option<Element<'_, Message>> {
        let body = match &self.config {
            HyprConfig::Lua(_) => return None,
            HyprConfig::Missing => column![
                scaled_text(
                    "No Hyprland config found. Hyprforge can create a minimal \
                     hyprland.lua that sources your window rules; everything else \
                     stays at Hyprland's defaults for you to fill in.",
                    13.0,
                    scale,
                ),
                meta_text(
                    hyprforge_core::paths::hyprland_lua_path().display().to_string(),
                    12.0,
                    scale,
                ),
                container(primary_button("Create hyprland.lua").on_press(Message::ConfirmCreateLuaConfig))
                    .width(Length::Fill)
                    .align_x(iced::alignment::Horizontal::Right),
            ],
            HyprConfig::ConfOnly(path) => column![
                scaled_text(
                    "You're using Hyprland's hyprland.conf format. Hyprforge \
                     generates Lua rules and sources them with require(), which \
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
            // Every draft field edit is the same shape: mutate the draft if
            // one is open, never re-render anything else.
            Message::DraftClass(v) => self.edit_draft(|d| d.class = v),
            Message::DraftTitle(v) => self.edit_draft(|d| d.title = v),
            Message::DraftInitialClass(v) => self.edit_draft(|d| d.initial_class = v),
            Message::DraftInitialTitle(v) => self.edit_draft(|d| d.initial_title = v),
            Message::DraftFullscreen(v) => self.edit_draft(|d| d.fullscreen = v),
            Message::DraftFloating(v) => self.edit_draft(|d| d.floating = v),
            Message::DraftXwayland(v) => self.edit_draft(|d| d.xwayland = v),
            Message::DraftMatchTag(v) => self.edit_draft(|d| d.match_tag = v),
            Message::DraftContent(v) => self.edit_draft(|d| d.content = v),
            Message::DraftWorkspace(v) => self.edit_draft(|d| d.workspace = v),
            Message::DraftWorkspaceSilent(v) => self.edit_draft(|d| d.workspace_silent = v),
            Message::DraftTag(v) => self.edit_draft(|d| d.tag = v),
            Message::DraftFloat(v) => self.edit_draft(|d| d.float = v),
            Message::DraftNoBlur(v) => self.edit_draft(|d| d.no_blur = v),
            Message::DraftRounding(v) => self.edit_draft(|d| d.rounding = v),
            Message::DraftBorderColor(v) => self.edit_draft(|d| d.border_color = v),
            Message::DraftMoveX(v) => self.edit_draft(|d| d.move_x = v),
            Message::DraftMoveY(v) => self.edit_draft(|d| d.move_y = v),
            Message::DraftSizeW(v) => self.edit_draft(|d| d.size_w = v),
            Message::DraftSizeH(v) => self.edit_draft(|d| d.size_h = v),
            Message::DraftOpacityActive(v) => self.edit_draft(|d| d.opacity_active = v),
            Message::DraftOpacityInactive(v) => self.edit_draft(|d| d.opacity_inactive = v),
            Message::DraftOpacityFullscreen(v) => self.edit_draft(|d| d.opacity_fullscreen = v),
            Message::DraftOpacityOverride(v) => self.edit_draft(|d| d.opacity_override = v),
            Message::OpenPicker => {
                self.picker = Some(PickerState::Loading);
                Task::perform(load_clients(), Message::ClientsLoaded)
            }
            Message::ClosePicker => {
                self.picker = None;
                Task::none()
            }
            Message::ClientsLoaded(result) => {
                // Dropped entirely if the picker was closed while the call
                // was in flight — reopening it starts a fresh load, and
                // letting this land would put the old list under the new
                // spinner.
                if self.picker.is_some() {
                    self.picker = Some(match result {
                        Ok(clients) => PickerState::Loaded(clients),
                        Err(e) => PickerState::Unavailable(e),
                    });
                }
                Task::none()
            }
            Message::PickClass(i) => {
                let class = self.picked_client(i).map(|c| c.class.clone());
                match class {
                    Some(class) => self.edit_draft(|d| d.class = class),
                    None => Task::none(),
                }
            }
            Message::PickTitle(i) => {
                let title = self.picked_client(i).map(|c| c.title.clone());
                match title {
                    Some(title) => self.edit_draft(|d| d.title = title),
                    None => Task::none(),
                }
            }
            Message::MonitorsLoaded(monitors) => {
                self.monitors = monitors;
                Task::none()
            }
            Message::AddWorkspacePin => {
                self.workspace_rules.push(WorkspaceRule::default());
                Task::none()
            }
            Message::DeleteWorkspacePin(i) => {
                if i < self.workspace_rules.len() {
                    self.workspace_rules.remove(i);
                    return self.save_and_maybe_reload();
                }
                Task::none()
            }
            Message::PinWorkspace(i, v) => self.edit_pin(i, |p| p.workspace = v),
            Message::PinMonitor(i, choice) => self.edit_pin(i, |p| p.monitor = choice.selector),
            Message::PinDefault(i, v) => self.edit_pin(i, |p| p.default = v),
            Message::PinPersistent(i, v) => self.edit_pin(i, |p| p.persistent = v),
            Message::ToggleAdvanced => {
                self.show_advanced = !self.show_advanced;
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
                        Task::perform(regenerate_and_reload(self.stored_rules()), Message::Reloaded)
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
            Message::ConfirmCreateLuaConfig => {
                let path = hyprforge_core::paths::hyprland_lua_path();
                match hyprforge_windowrules::setup::create_lua_config(&path) {
                    Ok(()) => {
                        // The file we just wrote already contains the require
                        // line, so setup is complete — no insertion step.
                        self.config = HyprConfig::Lua(path);
                        self.setup_plan = SetupPlan::AlreadyPresent;
                        self.error = None;
                        self.status = Some("Created hyprland.lua.".to_string());
                        Task::perform(regenerate_and_reload(self.stored_rules()), Message::Reloaded)
                    }
                    Err(e) => {
                        self.error = Some(e.to_string());
                        Task::none()
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

        let mut content = column![scaled_text("Window Rules", 22.0, scale)].spacing(spacing::LG);

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
        if self.rules.is_empty() {
            list = list.push(meta_text("No rules yet.", 14.0, scale));
        }
        for (i, rule) in self.rules.iter().enumerate() {
            if i > 0 {
                list = list.push(divider());
            }
            let summary = rule
                .matcher
                .class
                .clone()
                .or_else(|| rule.matcher.title.clone())
                .unwrap_or_else(|| "(no match)".to_string());
            let info = column![
                scaled_text(summary, 14.0, scale),
                meta_text(rule.name.clone(), 12.0, scale),
            ]
            .spacing(spacing::XS)
            .width(Length::Fill);
            list = list.push(
                container(
                    row![
                        checkbox(rule.enabled).on_toggle(move |_| Message::ToggleEnabled(i)),
                        info,
                        secondary_button("Up").on_press(Message::MoveUp(i)),
                        secondary_button("Down").on_press(Message::MoveDown(i)),
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
            "Rules",
            scale,
            container(scrollable(list).width(Length::Fill).height(Length::Shrink))
                .max_height(360.0),
        ));
        content = content.push(
            container(primary_button("Add rule").on_press(Message::Add))
                .width(Length::Fill)
                .align_x(iced::alignment::Horizontal::Right),
        );

        content = content.push(self.workspace_pins_view(scale));

        container(content).padding(spacing::LG).into()
    }
}

impl WindowRulesModule {
    /// Workspace→monitor pins.
    ///
    /// A separate section from the rules list because it's a different kind
    /// of statement: a rule says what a *window* does, a pin says where a
    /// *workspace* lives. Together they're how "Steam on the external
    /// display" gets expressed — the rule sends Steam to a workspace, the pin
    /// puts that workspace on the monitor.
    fn workspace_pins_view(&self, scale: FontScale) -> Element<'_, Message> {
        let mut choices: Vec<MonitorChoice> =
            self.monitors.iter().map(MonitorChoice::from_monitor).collect();

        let mut list = column![].spacing(spacing::MD);
        if self.workspace_rules.is_empty() {
            list = list.push(meta_text(
                "No workspaces pinned. Pin one to a monitor, then send windows \
                 to that workspace with a rule's Workspace field.",
                13.0,
                scale,
            ));
        }

        for (i, pin) in self.workspace_rules.iter().enumerate() {
            // A pin for a monitor that isn't plugged in right now still has
            // to show what it points at, or editing anything else on the row
            // would look like the monitor was never set.
            let selected = if pin.monitor.trim().is_empty() {
                None
            } else {
                let known = choices.iter().find(|c| c.selector == pin.monitor).cloned();
                Some(known.unwrap_or_else(|| {
                    let stored = MonitorChoice::from_stored(&pin.monitor);
                    if !choices.contains(&stored) {
                        choices.push(stored.clone());
                    }
                    stored
                }))
            };

            list = list.push(
                column![
                    row![
                        text_input("workspace, e.g. 3 or name:gaming", &pin.workspace)
                            .on_input(move |v| Message::PinWorkspace(i, v)),
                        iced::widget::pick_list(choices.clone(), selected, move |c| {
                            Message::PinMonitor(i, c)
                        })
                        .placeholder("on monitor…"),
                        danger_button("Remove", Message::DeleteWorkspacePin(i)),
                    ]
                    .spacing(spacing::SM)
                    .align_y(iced::Alignment::Center),
                    row![
                        checkbox(pin.default)
                            .label("Default workspace for that monitor")
                            .on_toggle(move |v| Message::PinDefault(i, v)),
                        checkbox(pin.persistent)
                            .label("Keep alive when empty")
                            .on_toggle(move |v| Message::PinPersistent(i, v)),
                    ]
                    .spacing(spacing::MD),
                ]
                .spacing(spacing::XS),
            );
        }

        let note = if self.monitors.is_empty() {
            Some(meta_text(
                "Couldn't read the connected monitors, so there's nothing to \
                 choose from — existing pins are still listed and kept.",
                12.0,
                scale,
            ))
        } else {
            // Worth saying out loud: this is why a pin survives a replug.
            Some(meta_text(
                "Monitors are matched by their description rather than their \
                 connector, so a pin still applies after replugging.",
                12.0,
                scale,
            ))
        };

        let mut body = column![list].spacing(spacing::MD);
        if let Some(note) = note {
            body = body.push(note);
        }
        body = body.push(
            container(secondary_button("Pin a workspace").on_press(Message::AddWorkspacePin))
                .width(Length::Fill),
        );

        section("Workspaces on monitors", scale, body)
    }

    /// The open-window list, inline under the form rather than as a modal —
    /// you're choosing a value *for* a field, and the field should stay
    /// visible while you do it.
    fn picker_view<'a>(
        &'a self,
        picker: &'a PickerState,
        scale: FontScale,
    ) -> Element<'a, Message> {
        let inner: Element<'_, Message> = match picker {
            PickerState::Loading => meta_text("Reading open windows…", 13.0, scale).into(),
            PickerState::Unavailable(why) => column![
                meta_text(
                    "Couldn't read the open windows — is Hyprland running? \
                     You can still type the class in by hand.",
                    13.0,
                    scale,
                ),
                meta_text(why.as_str(), 12.0, scale),
            ]
            .spacing(spacing::SM)
            .into(),
            PickerState::Loaded(clients) if clients.is_empty() => {
                meta_text("No open windows to pick from.", 13.0, scale).into()
            }
            PickerState::Loaded(clients) => {
                let mut list = column![].spacing(spacing::SM);
                for (i, client) in clients.iter().enumerate() {
                    // Class is the whole row: it's the choice you almost
                    // always want. The title is a separate, smaller action
                    // because a title-matched rule quietly stops working when
                    // the app renames its window — a browser tab, a file path
                    // — and that should be deliberate.
                    let mut actions = row![secondary_button(client.label()).on_press(
                        Message::PickClass(i)
                    )]
                    .spacing(spacing::SM);
                    if !client.title.trim().is_empty() {
                        actions = actions
                            .push(secondary_button("+ title").on_press(Message::PickTitle(i)));
                    }
                    list = list.push(
                        column![
                            actions,
                            meta_text(
                                format!(
                                    "workspace {}{}",
                                    client.workspace.name,
                                    if client.xwayland { " · XWayland" } else { "" }
                                ),
                                11.0,
                                scale,
                            ),
                        ]
                        .spacing(2),
                    );
                }
                column![
                    meta_text("Fills the class. Add the title only if you want the rule to apply to this one window.", 12.0, scale),
                    container(scrollable(list).width(Length::Fill)).max_height(240.0),
                ]
                .spacing(spacing::SM)
                .into()
            }
        };

        section(
            "Open windows",
            scale,
            column![
                inner,
                container(secondary_button("Close").on_press(Message::ClosePicker))
                    .width(Length::Fill),
            ]
            .spacing(spacing::MD),
        )
    }

    fn draft_view(&self, draft: &RuleDraft, scale: FontScale) -> Element<'_, Message> {
        let title = if draft.editing_index.is_some() {
            "Edit rule"
        } else {
            "New rule"
        };
        let form = column![
            row_field(
                "Class",
                row![
                    text_input("Window class (regex)", &draft.class)
                        .on_input(Message::DraftClass),
                    secondary_button("Pick a window…").on_press(Message::OpenPicker),
                ]
                .spacing(spacing::SM),
            ),
            row_field(
                "Title",
                text_input("Window title (regex)", &draft.title).on_input(Message::DraftTitle),
            ),
            // Workspace assignment sits in the primary form rather than
            // behind "advanced": it's the rule most people are here to
            // write, and burying it would make the common case the hidden
            // one.
            row_field(
                "Workspace",
                text_input("e.g. 3, name:coding, special:scratchpad", &draft.workspace)
                    .on_input(Message::DraftWorkspace),
            ),
            checkbox(draft.workspace_silent)
                .label("Open there without switching to it")
                .on_toggle(Message::DraftWorkspaceSilent),
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
        ]
        .spacing(spacing::MD);

        let mut body = column![
            scaled_text(title, 22.0, scale),
            section("Rule", scale, form),
        ]
        .spacing(spacing::LG)
        .max_width(520.0);

        if let Some(picker) = &self.picker {
            body = body.push(self.picker_view(picker, scale));
        }

        body = body.push(
            container(
                secondary_button(if self.show_advanced {
                    "Hide advanced"
                } else {
                    "Show advanced"
                })
                .on_press(Message::ToggleAdvanced),
            )
            .width(Length::Fill),
        );

        if self.show_advanced {
            body = body.push(section(
                "Match on more",
                scale,
                column![
                    // initial_* match the class/title the window had when it
                    // opened, which is what you need for apps that rename
                    // themselves after startup.
                    row_field(
                        "Initial class",
                        text_input("Class at open time (regex)", &draft.initial_class)
                            .on_input(Message::DraftInitialClass),
                    ),
                    row_field(
                        "Initial title",
                        text_input("Title at open time (regex)", &draft.initial_title)
                            .on_input(Message::DraftInitialTitle),
                    ),
                    row_field("Fullscreen", tri_state(draft.fullscreen, Message::DraftFullscreen)),
                    row_field("Floating", tri_state(draft.floating, Message::DraftFloating)),
                    row_field("XWayland", tri_state(draft.xwayland, Message::DraftXwayland)),
                    row_field(
                        "Tag",
                        text_input("e.g. term — also matches term*", &draft.match_tag)
                            .on_input(Message::DraftMatchTag),
                    ),
                    row_field(
                        "Content type",
                        text_input("e.g. game, video", &draft.content)
                            .on_input(Message::DraftContent),
                    ),
                ]
                .spacing(spacing::MD),
            ));

            body = body.push(section(
                "Tag",
                scale,
                column![
                    row_field(
                        "Apply tag",
                        text_input("e.g. +code", &draft.tag).on_input(Message::DraftTag),
                    ),
                    meta_text(
                        "Prefix + to add and - to remove; no prefix toggles. Tagged \
                         windows can then be matched by other rules.",
                        12.0,
                        scale,
                    ),
                ]
                .spacing(spacing::SM),
            ));

            body = body.push(section(
                "Position & size",
                scale,
                column![
                    meta_text(
                        "Plain numbers are pixels. Anything else is passed to Hyprland \
                         as an expression, e.g. cursor_x-(window_w*0.5) or 60%.",
                        12.0,
                        scale,
                    ),
                    row![
                        text_input("x", &draft.move_x).on_input(Message::DraftMoveX),
                        text_input("y", &draft.move_y).on_input(Message::DraftMoveY),
                    ]
                    .spacing(spacing::SM),
                    row![
                        text_input("width", &draft.size_w).on_input(Message::DraftSizeW),
                        text_input("height", &draft.size_h).on_input(Message::DraftSizeH),
                    ]
                    .spacing(spacing::SM),
                    meta_text("Both halves of a pair are needed for it to apply.", 12.0, scale),
                ]
                .spacing(spacing::SM),
            ));

            body = body.push(section(
                "Opacity",
                scale,
                column![
                    row_field(
                        "Active",
                        text_input("1.0", &draft.opacity_active)
                            .on_input(Message::DraftOpacityActive),
                    ),
                    row_field(
                        "Inactive",
                        text_input("1.0", &draft.opacity_inactive)
                            .on_input(Message::DraftOpacityInactive),
                    ),
                    row_field(
                        "Fullscreen",
                        text_input("1.0", &draft.opacity_fullscreen)
                            .on_input(Message::DraftOpacityFullscreen),
                    ),
                    checkbox(draft.opacity_override)
                        .label("Absolute (override) rather than multiplied")
                        .on_toggle(Message::DraftOpacityOverride),
                ]
                .spacing(spacing::MD),
            ));
        }

        body = body.push(
            container(
                row![
                    secondary_button("Cancel").on_press(Message::DraftCancel),
                    primary_button("Save").on_press(Message::DraftSave),
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

/// Reads the connected monitors. A failure is flattened to an empty list
/// rather than surfaced: the pin section still works without it, and the
/// module already reports a missing `hyprctl` when a save actually needs one.
async fn load_monitors() -> Vec<Monitor> {
    tokio::task::spawn_blocking(hyprforge_windowrules::monitors::list_monitors)
        .await
        .ok()
        .and_then(Result::ok)
        .unwrap_or_default()
}

/// Reads the open windows off the compositor. Blocking work (it shells out to
/// `hyprctl`), so it goes on the blocking pool rather than stalling the UI
/// thread — same treatment `regenerate_and_reload` gets below.
async fn load_clients() -> Result<Vec<Client>, String> {
    tokio::task::spawn_blocking(hyprforge_windowrules::clients::list_clients)
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())
}

async fn regenerate_and_reload(rules: hyprforge_windowrules::storage::Rules) -> Result<(), String> {
    tokio::task::spawn_blocking(move || {
        hyprforge_windowrules::apply::apply(&hyprforge_core::paths::window_rules_lua_path(), &rules)
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Runs `f` against a module whose config lives in a throwaway directory.
    ///
    /// `WindowRulesModule::new()` reads the canonical TOML and several update
    /// arms write it straight back, so a test that skips this loads — and
    /// *saves over* — the real `~/.config/hyprforge/window-rules.toml`. That
    /// happened: a run wrote its fixture pins into the developer's own
    /// config, and the tests then failed because each one loaded what the
    /// last had left behind.
    ///
    /// A fresh directory per test, because sharing one just moves the
    /// contamination from the real config into a shared temp one. The lock
    /// is what makes that safe: `$XDG_CONFIG_HOME` is process-global and
    /// these tests run on parallel threads, so only one may own it at a time.
    /// Same reasoning as the `ENV_LOCK` in `hyprforge-core`'s `paths` tests.
    fn with_isolated_module(f: impl FnOnce(&mut WindowRulesModule)) {
        use std::sync::Mutex;
        static ENV_LOCK: Mutex<()> = Mutex::new(());
        // A test that panics while holding the lock poisons it; the next
        // test still needs to run, and there's no shared state to corrupt.
        let _lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());

        let dir = tempfile::tempdir().expect("could not make a temp config dir");
        let previous = std::env::var_os("XDG_CONFIG_HOME");
        std::env::set_var("XDG_CONFIG_HOME", dir.path());

        let mut module = WindowRulesModule::new().0;
        f(&mut module);

        match previous {
            Some(v) => std::env::set_var("XDG_CONFIG_HOME", v),
            None => std::env::remove_var("XDG_CONFIG_HOME"),
        }
    }

    fn fully_populated_rule() -> Rule {
        Rule {
            name: "hyprforge-test-1".to_string(),
            enabled: true,
            matcher: Matcher {
                class: Some("discord".to_string()),
                title: Some("^Discord$".to_string()),
                initial_class: Some("Discord".to_string()),
                initial_title: Some("Starting".to_string()),
                fullscreen: Some(true),
                // Explicitly false, not unset — the case a plain checkbox
                // can't represent.
                floating: Some(false),
                xwayland: Some(true),
                tag: Some("term".to_string()),
                content: Some("game".to_string()),
            },
            effects: Effects {
                workspace: Workspace { name: "name:coding".to_string(), silent: true },
                tag: Some("+code".to_string()),
                float: Some(true),
                r#move: Some(["cursor_x-(window_w*0.5)".to_string(), "40".to_string()]),
                size: Some(["60%".to_string(), "480".to_string()]),
                opacity: Opacity {
                    active: Some(0.9),
                    inactive: Some(0.7),
                    fullscreen: Some(1.0),
                    is_override: true,
                },
                border_color: Some("rgb(FF0000)".to_string()),
                no_blur: Some(true),
                rounding: Some(8),
            },
        }
    }

    /// The invariant that keeps editing non-destructive: anything the form
    /// can hold must survive load → save unchanged. A field that's writable
    /// but not readable silently erases itself on the user's next edit.
    #[test]
    fn draft_round_trips_every_field() {
        let rule = fully_populated_rule();
        let (matcher, effects) = RuleDraft::from_rule(0, &rule).into_matcher_effects();
        assert_eq!(matcher, rule.matcher);
        assert_eq!(effects, rule.effects);
    }

    #[test]
    fn draft_preserves_explicitly_false_matchers() {
        let mut rule = fully_populated_rule();
        rule.matcher.floating = Some(false);
        rule.matcher.fullscreen = None;

        let (matcher, _) = RuleDraft::from_rule(0, &rule).into_matcher_effects();
        assert_eq!(
            matcher.floating,
            Some(false),
            "floating = false selects tiled windows; collapsing it to None would \
             widen the rule to match everything"
        );
        assert_eq!(matcher.fullscreen, None);
    }

    #[test]
    fn blank_fields_become_none_rather_than_empty_strings() {
        let (matcher, effects) = RuleDraft::default().into_matcher_effects();
        assert!(matcher.is_empty());
        assert_eq!(effects, Effects::default());
    }

    #[test]
    fn whitespace_only_input_counts_as_blank() {
        let draft = RuleDraft {
            class: "   ".to_string(),
            rounding: "  ".to_string(),
            ..Default::default()
        };
        let (matcher, effects) = draft.into_matcher_effects();
        assert_eq!(matcher.class, None);
        assert_eq!(effects.rounding, None);
    }

    #[test]
    fn a_half_filled_move_pair_is_dropped() {
        // Hyprland's move takes { x, y }; there is no way to express "x only",
        // so a partial pair must not be emitted as a bogus rule.
        let draft = RuleDraft {
            move_x: "100".to_string(),
            size_h: "480".to_string(),
            ..Default::default()
        };
        let (_, effects) = draft.into_matcher_effects();
        assert_eq!(effects.r#move, None);
        assert_eq!(effects.size, None);
    }

    #[test]
    fn unparseable_numbers_are_dropped_not_defaulted() {
        // Mid-typing garbage must not silently become 0 — that would apply a
        // real rounding of 0 the user never asked for.
        let draft = RuleDraft {
            rounding: "abc".to_string(),
            opacity_active: "not-a-number".to_string(),
            ..Default::default()
        };
        let (_, effects) = draft.into_matcher_effects();
        assert_eq!(effects.rounding, None);
        assert_eq!(effects.opacity.active, None);
    }

    /// The whole chain the GUI actually exercises: form state -> model ->
    /// generated Lua. Guards the seam between this module and the codegen,
    /// which the per-layer tests on either side don't cover.
    #[test]
    fn an_advanced_draft_reaches_the_generated_lua() {
        let draft = RuleDraft {
            class: "discord".to_string(),
            floating: Some(false),
            initial_class: "Discord".to_string(),
            move_x: "cursor_x-(window_w*0.5)".to_string(),
            move_y: "40".to_string(),
            size_w: "60%".to_string(),
            size_h: "480".to_string(),
            opacity_active: "0.9".to_string(),
            opacity_inactive: "0.7".to_string(),
            opacity_override: true,
            ..Default::default()
        };
        let (matcher, effects) = draft.into_matcher_effects();
        let lua = hyprforge_windowrules::codegen::generate(&[Rule {
            name: "hyprforge-discord-1".to_string(),
            enabled: true,
            matcher,
            effects,
        }], &[]);

        assert!(lua.contains("initial_class = [[Discord]]"), "{lua}");
        // Emitted as `float`, which is what Hyprland calls the matcher —
        // the draft field and TOML key stay `floating`.
        assert!(lua.contains("float = false"), "{lua}");
        // A literal is emitted bare; an expression is quoted.
        assert!(lua.contains("move = { [[cursor_x-(window_w*0.5)]], 40 }"), "{lua}");
        assert!(lua.contains("size = { [[60%]], 480 }"), "{lua}");
        assert!(lua.contains("opacity = [[0.9 override 0.7 override]]"), "{lua}");
    }

    /// The headline case, end to end: what a user types into the form
    /// becomes the Lua that puts Discord on workspace 3 without yanking
    /// them to it.
    #[test]
    fn a_typed_workspace_reaches_the_generated_lua() {
        let draft = RuleDraft {
            class: "discord".to_string(),
            workspace: "3".to_string(),
            workspace_silent: true,
            ..Default::default()
        };
        let (matcher, effects) = draft.into_matcher_effects();
        let lua = hyprforge_windowrules::codegen::generate(&[Rule {
            name: "hyprforge-discord-1".to_string(),
            enabled: true,
            matcher,
            effects,
        }], &[]);
        assert!(lua.contains("workspace = [[3 silent]]"), "{lua}");
    }

    /// The checkbox can be left on with the field cleared. "Silently open on
    /// no workspace at all" isn't a rule, so it must not be written.
    #[test]
    fn silent_without_a_workspace_applies_nothing() {
        let draft = RuleDraft {
            class: "discord".to_string(),
            workspace: "   ".to_string(),
            workspace_silent: true,
            ..Default::default()
        };
        let (matcher, effects) = draft.into_matcher_effects();
        assert!(effects.workspace.is_empty());
        let lua = hyprforge_windowrules::codegen::generate(&[Rule {
            name: "hyprforge-discord-1".to_string(),
            enabled: true,
            matcher,
            effects,
        }], &[]);
        assert!(!lua.contains("workspace"), "{lua}");
    }

    fn client(class: &str, title: &str) -> Client {
        Client {
            class: class.to_string(),
            title: title.to_string(),
            mapped: true,
            ..Default::default()
        }
    }

    /// Opens a draft and shows the picker with two windows, one of which
    /// shares nothing with the other so a mixed-up index is visible.
    fn with_picker(f: impl FnOnce(&mut WindowRulesModule)) {
        with_isolated_module(|m| {
            m.draft = Some(RuleDraft::default());
            m.picker = Some(PickerState::Loaded(vec![
                client("com.mitchellh.ghostty", "hyprforge"),
                client("dev.zed.Zed", "hyprland.lua"),
            ]));
            f(m);
        });
    }

    #[test]
    fn picking_a_window_fills_the_class_and_leaves_the_title_alone() {
        with_picker(|m| {
            let _ = m.update(Message::PickClass(1));
            let draft = m.draft.as_ref().unwrap();
            assert_eq!(draft.class, "dev.zed.Zed");
            assert_eq!(draft.title, "", "the title is a separate, deliberate choice");
        });
    }

    #[test]
    fn adding_the_title_is_a_second_explicit_action() {
        with_picker(|m| {
            let _ = m.update(Message::PickClass(0));
            let _ = m.update(Message::PickTitle(0));
            let draft = m.draft.as_ref().unwrap();
            assert_eq!(draft.class, "com.mitchellh.ghostty");
            assert_eq!(draft.title, "hyprforge");
        });
    }

    /// An index only means anything against the list it was rendered from.
    /// Out of range must be dropped, not panic — indexing a Vec here would
    /// take the whole app down.
    #[test]
    fn a_stale_index_is_ignored_rather_than_panicking() {
        with_picker(|m| {
            let _ = m.update(Message::PickClass(99));
            assert_eq!(m.draft.as_ref().unwrap().class, "");
        });
    }

    #[test]
    fn picking_while_the_picker_is_closed_does_nothing() {
        with_isolated_module(|m| {
            m.draft = Some(RuleDraft::default());
            m.picker = None;
            let _ = m.update(Message::PickClass(0));
            assert_eq!(m.draft.as_ref().unwrap().class, "");
        });
    }

    /// Closing the picker while the hyprctl call is still running must not
    /// have the result reopen it underneath the user.
    #[test]
    fn a_response_arriving_after_close_is_discarded() {
        with_picker(|m| {
            let _ = m.update(Message::ClosePicker);
            let _ = m.update(Message::ClientsLoaded(Ok(vec![client("late", "arrival")])));
            assert!(m.picker.is_none(), "a closed picker must stay closed");
        });
    }

    /// No Hyprland is not an error dialog — the fields still work by hand.
    #[test]
    fn an_unavailable_compositor_leaves_the_form_usable() {
        with_isolated_module(|m| {
            m.draft = Some(RuleDraft::default());
            let _ = m.update(Message::OpenPicker);
            let _ = m.update(Message::ClientsLoaded(Err("could not run hyprctl".into())));
            assert!(matches!(m.picker, Some(PickerState::Unavailable(_))));
            assert!(m.error.is_none(), "this must not surface as a module-level error");
        });
    }

    fn monitor(name: &str, desc: &str) -> Monitor {
        Monitor { name: name.to_string(), description: desc.to_string() }
    }

    /// A pin must store the EDID description, not the connector — that's the
    /// whole reason it survives a replug.
    #[test]
    fn choosing_a_monitor_stores_its_description_not_its_connector() {
        with_isolated_module(|m| {
            m.monitors = vec![monitor("DP-3", "GWD ARZOPA")];
            let _ = m.update(Message::AddWorkspacePin);
            let choice = MonitorChoice::from_monitor(&m.monitors[0]);
            let _ = m.update(Message::PinMonitor(0, choice));
            assert_eq!(m.workspace_rules[0].monitor, "desc:GWD ARZOPA");
        });
    }

    /// A monitor that reports no description can only be named by connector;
    /// a bare `desc:` would match nothing at all.
    #[test]
    fn a_monitor_without_a_description_is_pinned_by_connector() {
        let choice = MonitorChoice::from_monitor(&monitor("HDMI-A-1", ""));
        assert_eq!(choice.selector, "HDMI-A-1");
    }

    /// Unplugging the monitor a pin points at must not make the pin look
    /// blank — editing anything else on the row would then quietly clear it.
    #[test]
    fn a_pin_for_a_disconnected_monitor_still_shows_what_it_points_at() {
        let stored = MonitorChoice::from_stored("desc:GWD ARZOPA");
        assert_eq!(stored.selector, "desc:GWD ARZOPA");
        assert!(stored.to_string().contains("not connected"));
    }

    #[test]
    fn pins_can_be_added_and_removed() {
        with_isolated_module(|m| {
            let _ = m.update(Message::AddWorkspacePin);
            let _ = m.update(Message::AddWorkspacePin);
            assert_eq!(m.workspace_rules.len(), 2);
            let _ = m.update(Message::PinWorkspace(1, "name:gaming".into()));
            let _ = m.update(Message::DeleteWorkspacePin(0));
            assert_eq!(m.workspace_rules.len(), 1);
            assert_eq!(m.workspace_rules[0].workspace, "name:gaming");
        });
    }

    /// Deleting past the end has to be inert rather than panicking — the
    /// same stale-index hazard the picker has.
    #[test]
    fn editing_a_pin_that_is_gone_does_nothing() {
        with_isolated_module(|m| {
            let _ = m.update(Message::DeleteWorkspacePin(3));
            let _ = m.update(Message::PinWorkspace(3, "x".into()));
            assert!(m.workspace_rules.is_empty());
        });
    }

    /// Adding a row before typing into it is the normal flow, so a blank pin
    /// must be storable and simply not emitted.
    #[test]
    fn a_half_filled_pin_is_kept_but_generates_nothing() {
        with_isolated_module(|m| {
            let _ = m.update(Message::AddWorkspacePin);
            let _ = m.update(Message::PinDefault(0, true));
            assert_eq!(m.workspace_rules.len(), 1);
            let lua = hyprforge_windowrules::codegen::generate(&[], &m.workspace_rules);
            assert!(!lua.contains("workspace_rule"), "{lua}");
        });
    }

    #[test]
    fn move_expressions_survive_verbatim_for_the_codegen_to_quote() {
        let draft = RuleDraft {
            move_x: "cursor_x-(window_w*0.5)".to_string(),
            move_y: "100".to_string(),
            ..Default::default()
        };
        let (_, effects) = draft.into_matcher_effects();
        assert_eq!(
            effects.r#move,
            Some(["cursor_x-(window_w*0.5)".to_string(), "100".to_string()])
        );
    }
}
