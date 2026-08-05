use crate::modules::layout_canvas::{CanvasHead, LayoutCanvas};
use hyprforge_core::displayd_proxy::DisplaydProxy;
use hyprforge_core::theme::{spacing, FontScale};
use hyprforge_core::widgets::{
    divider, meta_text, primary_button, row_field, scaled_text, secondary_button, section,
};
use hyprforge_core::SettingsModule;
use iced::widget::{column, container, row, scrollable, text_input};
use iced::{Element, Length, Subscription, Task};
use serde::Deserialize;

/// Full geometry for one stored profile, fetched on demand for the
/// Monitors editor — `ListProfiles`' summary row doesn't carry per-head
/// detail. Field names/renames mirror `hyprforge_displayd::profile::Profile`
/// exactly, since it's what `GetProfile` serializes; kept as a local,
/// GUI-only type rather than a dependency on the (Wayland-heavy) daemon
/// crate.
#[derive(Debug, Clone, Deserialize)]
pub struct ProfileDetail {
    id: String,
    name: String,
    extra_output_policy: String,
    #[allow(dead_code)]
    head_swaps: Vec<(String, String)>,
    #[serde(rename = "head")]
    heads: Vec<HeadDetail>,
}

#[derive(Debug, Clone, Deserialize)]
struct HeadDetail {
    #[allow(dead_code)]
    make: String,
    #[allow(dead_code)]
    model: String,
    #[allow(dead_code)]
    serial: String,
    connector_hint: String,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    refresh_mhz: i32,
    scale: f64,
    #[allow(dead_code)]
    transform: String,
    enabled: bool,
}

/// One entry in the resolution/refresh-rate dropdown — populated from the
/// head's *live* supported modes (see `available_modes` on the daemon),
/// since a stored profile only ever remembers the one mode it was saved
/// with, not the full list a physical head supports.
#[derive(Debug, Clone, PartialEq)]
pub struct ModeOption {
    width: i32,
    height: i32,
    refresh_mhz: i32,
    preferred: bool,
}

impl std::fmt::Display for ModeOption {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} x {} @ {:.0}Hz{}",
            self.width,
            self.height,
            self.refresh_mhz as f64 / 1000.0,
            if self.preferred { " (recommended)" } else { "" }
        )
    }
}

/// Orientation options shown in the property panel, in the same order as
/// `Transform`'s wire representation (`GetProfile`'s JSON / `Transform`'s
/// default serde variant names) — index-paired with `TRANSFORM_RAW` so a
/// label picked from the dropdown maps straight back to the string
/// `SetHeadGeometry` expects.
const TRANSFORM_LABELS: [&str; 8] = [
    "Landscape",
    "Portrait (90°)",
    "Landscape (180°)",
    "Portrait (270°)",
    "Landscape (mirrored)",
    "Portrait (90°, mirrored)",
    "Landscape (180°, mirrored)",
    "Portrait (270°, mirrored)",
];
const TRANSFORM_RAW: [&str; 8] = [
    "Normal",
    "Rotate90",
    "Rotate180",
    "Rotate270",
    "Flipped",
    "Flipped90",
    "Flipped180",
    "Flipped270",
];

fn transform_to_label(raw: &str) -> &'static str {
    TRANSFORM_RAW
        .iter()
        .position(|r| *r == raw)
        .map(|i| TRANSFORM_LABELS[i])
        .unwrap_or(TRANSFORM_LABELS[0])
}

fn label_to_transform(label: &str) -> &'static str {
    TRANSFORM_LABELS
        .iter()
        .position(|l| *l == label)
        .map(|i| TRANSFORM_RAW[i])
        .unwrap_or(TRANSFORM_RAW[0])
}

/// The property-panel draft for whichever head is selected — text fields
/// so partial/invalid input (e.g. a bare "-" while typing a negative X)
/// never gets silently reformatted out from under the user, mirroring the
/// same pattern `RuleDraft` uses in the Window Rules module.
fn fields_from_head(h: &HeadDetail) -> (String, String, String, String, String, String, String) {
    (
        h.x.to_string(),
        h.y.to_string(),
        h.width.to_string(),
        h.height.to_string(),
        format!("{:.0}", h.refresh_mhz as f64 / 1000.0),
        format!("{:.0}", h.scale * 100.0),
        transform_to_label(&h.transform).to_string(),
    )
}

/// Writes the property panel's draft fields straight into the selected
/// head as soon as they parse cleanly, mirroring how dragging on the
/// canvas already updates the head live — so there's no separate "Apply to
/// head" step to remember. Silently no-ops while a field is mid-edit and
/// not yet a valid number (e.g. a bare "-"); the last valid value stays in
/// effect until Save & Apply persists it.
fn commit_selected_head(editor: &mut LayoutEditor) {
    let Some(selected) = editor.selected.clone() else {
        return;
    };
    let parsed = (
        editor.field_x.trim().parse::<i32>(),
        editor.field_y.trim().parse::<i32>(),
        editor.field_width.trim().parse::<i32>(),
        editor.field_height.trim().parse::<i32>(),
        editor.field_refresh.trim().parse::<f64>(),
        editor.field_scale.trim().parse::<f64>(),
    );
    if let (Ok(x), Ok(y), Ok(width), Ok(height), Ok(refresh_hz), Ok(scale_pct)) = parsed {
        if width > 0 && height > 0 && refresh_hz > 0.0 && scale_pct > 0.0 {
            if let Some(head) = editor
                .profile
                .heads
                .iter_mut()
                .find(|h| h.connector_hint == selected)
            {
                head.x = x;
                head.y = y;
                head.width = width;
                head.height = height;
                head.refresh_mhz = (refresh_hz * 1000.0).round() as i32;
                head.scale = scale_pct / 100.0;
                head.transform = label_to_transform(&editor.field_transform).to_string();
            }
        }
    }
}

struct LayoutEditor {
    profile: ProfileDetail,
    selected: Option<String>,
    /// Live modes for `selected`'s connector, if it's currently connected
    /// — empty otherwise, in which case the view falls back to free-text
    /// width/height/refresh fields.
    available_modes: Vec<ModeOption>,
    field_x: String,
    field_y: String,
    field_width: String,
    field_height: String,
    field_refresh: String,
    field_scale: String,
    field_transform: String,
    swap_a: Option<String>,
    swap_b: Option<String>,
    status: Option<String>,
    error: Option<String>,
}

impl LayoutEditor {
    fn new(profile: ProfileDetail) -> Self {
        let selected = profile.heads.first().map(|h| h.connector_hint.clone());
        let (field_x, field_y, field_width, field_height, field_refresh, field_scale, field_transform) =
            profile.heads.first().map(fields_from_head).unwrap_or_default();
        LayoutEditor {
            profile,
            selected,
            available_modes: Vec::new(),
            field_x,
            field_y,
            field_width,
            field_height,
            field_refresh,
            field_scale,
            field_transform,
            swap_a: None,
            swap_b: None,
            status: None,
            error: None,
        }
    }

    fn selected_head(&self) -> Option<&HeadDetail> {
        let hint = self.selected.as_ref()?;
        self.profile.heads.iter().find(|h| &h.connector_hint == hint)
    }
}

#[derive(Debug, Clone)]
pub struct ProfileInfo {
    pub id: String,
    pub name: String,
    pub head_count: u32,
    pub last_used: String,
}

#[derive(Debug, Clone)]
pub struct LoadedState {
    profiles: Vec<ProfileInfo>,
    current_fingerprint: String,
    competing_monitor_rules: Vec<String>,
}

#[derive(Debug, Clone)]
pub enum SignalKind {
    ProfileApplied { id: String, name: String, tier: String },
    NewTopologySeen { summary: String },
}

#[derive(Debug, Clone)]
pub enum Message {
    Refresh,
    Loaded(Result<LoadedState, String>),
    Apply(String),
    Applied(String, Result<(), String>),
    RenameStart(String, String),
    RenameInput(String),
    RenameSubmit,
    RenameCancelled,
    Renamed(Result<(), String>),
    SignalReceived(SignalKind),
    ToggleOtherProfiles,
    EditLayout(String),
    LayoutLoaded(Result<ProfileDetail, String>),
    DiscardEdits,
    SelectHead(String),
    ModesLoaded(String, Vec<(i32, i32, i32, bool)>),
    ModeSelected(ModeOption),
    HeadDragMoved(String, i32, i32),
    FieldX(String),
    FieldY(String),
    FieldWidth(String),
    FieldHeight(String),
    FieldRefresh(String),
    FieldScale(String),
    FieldTransform(String),
    SwapSelectA(String),
    SwapSelectB(String),
    ToggleSwap,
    SwapToggled(Result<ProfileDetail, String>),
    SetPolicy(String),
    PolicySet(Result<ProfileDetail, String>),
    SaveLayout,
    LayoutSaved(Result<(), String>),
}

pub struct DisplaysModule {
    connected: bool,
    error: Option<String>,
    profiles: Vec<ProfileInfo>,
    current_fingerprint: String,
    /// The profile the daemon actually matched/applied, tracked from the
    /// `ProfileApplied` signal (accurate for superset/subset matches too)
    /// and, as a fallback for the very first load, by comparing
    /// `current_fingerprint` against exact-match profile ids.
    current_profile_id: Option<String>,
    competing_monitor_rules: Vec<String>,
    renaming: Option<(String, String)>,
    last_event: Option<String>,
    /// Managing every stored profile (not just the one loaded in the
    /// editor) is the power-user case — collapsed by default.
    show_other_profiles: bool,
    /// The profile currently shown in the canvas/property panel. Starts
    /// out following `current_profile_id` automatically; once the user
    /// explicitly picks a different profile to edit (via `EditLayout`),
    /// it stops following so an unrelated apply/auto-learn elsewhere
    /// can't yank their in-progress edit out from under them.
    editor: Option<LayoutEditor>,
}

impl DisplaysModule {
    pub fn new() -> (Self, Task<Message>) {
        (
            DisplaysModule {
                connected: false,
                error: None,
                profiles: Vec::new(),
                current_fingerprint: String::new(),
                current_profile_id: None,
                competing_monitor_rules: Vec::new(),
                renaming: None,
                last_event: None,
                show_other_profiles: false,
                editor: None,
            },
            Task::perform(load(), Message::Loaded),
        )
    }

    /// Loads `current_profile_id` into the editor, but only if nothing is
    /// loaded there yet — see the `editor` field doc for why this doesn't
    /// run unconditionally on every update.
    fn maybe_autoload_editor(&self) -> Task<Message> {
        if self.editor.is_none() {
            if let Some(id) = &self.current_profile_id {
                return Task::perform(load_profile(id.clone()), Message::LayoutLoaded);
            }
        }
        Task::none()
    }

    fn profile_row(&self, p: &ProfileInfo, scale: FontScale, is_current: bool) -> Element<'_, Message> {
        let name = if is_current {
            format!("{} (current)", p.name)
        } else {
            p.name.clone()
        };
        let info = column![
            scaled_text(name, 14.0, scale),
            meta_text(
                format!("{} head(s) · last used {}", p.head_count, p.last_used),
                12.0,
                scale,
            ),
        ]
        .spacing(spacing::XS)
        .width(Length::Fill);

        container(
            row![
                info,
                secondary_button("Edit").on_press(Message::EditLayout(p.id.clone())),
                secondary_button("Rename")
                    .on_press(Message::RenameStart(p.id.clone(), p.name.clone())),
                primary_button("Apply").on_press(Message::Apply(p.id.clone())),
            ]
            .spacing(spacing::SM)
            .align_y(iced::Alignment::Center),
        )
        .padding([spacing::SM, 0.0])
        .into()
    }
}

impl SettingsModule for DisplaysModule {
    type Message = Message;

    fn title(&self) -> &str {
        "Monitors"
    }

    fn icon(&self) -> &'static str {
        "\u{1F5A5}"
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Refresh => Task::perform(load(), Message::Loaded),
            Message::Loaded(Ok(state)) => {
                self.connected = true;
                self.error = None;
                self.profiles = state.profiles;
                self.current_fingerprint = state.current_fingerprint;
                self.competing_monitor_rules = state.competing_monitor_rules;
                // Only covers the exact-match case (profile id == connected
                // fingerprint); superset/subset matches are only known once
                // a ProfileApplied signal arrives. Never clobber an already
                // -known current profile with "unknown" just because this
                // particular load can't derive it.
                if let Some(p) = self
                    .profiles
                    .iter()
                    .find(|p| p.id == self.current_fingerprint)
                {
                    self.current_profile_id = Some(p.id.clone());
                }
                self.maybe_autoload_editor()
            }
            Message::Loaded(Err(e)) => {
                self.connected = false;
                self.error = Some(e);
                Task::none()
            }
            Message::Apply(id) => Task::perform(apply(id), |(id, result)| Message::Applied(id, result)),
            Message::Applied(id, Ok(())) => {
                self.current_profile_id = Some(id);
                Task::perform(load(), Message::Loaded)
            }
            Message::Applied(_, Err(e)) => {
                self.error = Some(e);
                Task::none()
            }
            Message::ToggleOtherProfiles => {
                self.show_other_profiles = !self.show_other_profiles;
                Task::none()
            }
            Message::RenameStart(id, current) => {
                self.renaming = Some((id, current));
                Task::none()
            }
            Message::RenameInput(value) => {
                if let Some((_, draft)) = &mut self.renaming {
                    *draft = value;
                }
                Task::none()
            }
            Message::RenameSubmit => {
                if let Some((id, draft)) = self.renaming.take() {
                    Task::perform(rename(id, draft), Message::Renamed)
                } else {
                    Task::none()
                }
            }
            Message::RenameCancelled => {
                self.renaming = None;
                Task::none()
            }
            Message::Renamed(Ok(())) => Task::perform(load(), Message::Loaded),
            Message::Renamed(Err(e)) => {
                self.error = Some(e);
                Task::none()
            }
            Message::SignalReceived(kind) => {
                self.last_event = Some(match kind {
                    SignalKind::ProfileApplied { id, name, tier } => {
                        self.current_profile_id = Some(id);
                        format!("Applied '{name}' ({tier})")
                    }
                    SignalKind::NewTopologySeen { summary } => format!("New topology: {summary}"),
                });
                let reload = Task::perform(load(), Message::Loaded);
                Task::batch([reload, self.maybe_autoload_editor()])
            }
            Message::EditLayout(id) => Task::perform(load_profile(id), Message::LayoutLoaded),
            Message::LayoutLoaded(Ok(profile)) => {
                let selected = profile.heads.first().map(|h| h.connector_hint.clone());
                self.editor = Some(LayoutEditor::new(profile));
                match selected {
                    Some(hint) => Task::perform(fetch_modes(hint.clone()), move |modes| {
                        Message::ModesLoaded(hint.clone(), modes)
                    }),
                    None => Task::none(),
                }
            }
            Message::LayoutLoaded(Err(e)) => {
                self.error = Some(e);
                Task::none()
            }
            Message::DiscardEdits => {
                let Some(editor) = &self.editor else {
                    return Task::none();
                };
                Task::perform(load_profile(editor.profile.id.clone()), Message::LayoutLoaded)
            }
            Message::SelectHead(hint) => {
                if let Some(editor) = &mut self.editor {
                    if let Some(h) = editor.profile.heads.iter().find(|h| h.connector_hint == hint) {
                        let (x, y, w, ht, r, s, t) = fields_from_head(h);
                        editor.field_x = x;
                        editor.field_y = y;
                        editor.field_width = w;
                        editor.field_height = ht;
                        editor.field_refresh = r;
                        editor.field_scale = s;
                        editor.field_transform = t;
                    }
                    editor.available_modes = Vec::new();
                    editor.selected = Some(hint.clone());
                    return Task::perform(fetch_modes(hint.clone()), move |modes| {
                        Message::ModesLoaded(hint.clone(), modes)
                    });
                }
                Task::none()
            }
            Message::ModesLoaded(hint, modes) => {
                if let Some(editor) = &mut self.editor {
                    // Discard if the user already selected a different
                    // head before this (possibly slow) D-Bus round trip
                    // returned.
                    if editor.selected.as_deref() == Some(hint.as_str()) {
                        editor.available_modes = modes
                            .into_iter()
                            .map(|(width, height, refresh_mhz, preferred)| ModeOption {
                                width,
                                height,
                                refresh_mhz,
                                preferred,
                            })
                            .collect();
                    }
                }
                Task::none()
            }
            Message::ModeSelected(mode) => {
                if let Some(editor) = &mut self.editor {
                    editor.field_width = mode.width.to_string();
                    editor.field_height = mode.height.to_string();
                    editor.field_refresh = format!("{:.0}", mode.refresh_mhz as f64 / 1000.0);
                    commit_selected_head(editor);
                }
                Task::none()
            }
            Message::FieldTransform(label) => {
                if let Some(editor) = &mut self.editor {
                    editor.field_transform = label;
                    commit_selected_head(editor);
                }
                Task::none()
            }
            Message::HeadDragMoved(connector_hint, x, y) => {
                if let Some(editor) = &mut self.editor {
                    if let Some(head) = editor
                        .profile
                        .heads
                        .iter_mut()
                        .find(|h| h.connector_hint == connector_hint)
                    {
                        head.x = x;
                        head.y = y;
                    }
                    if editor.selected.as_deref() == Some(connector_hint.as_str()) {
                        editor.field_x = x.to_string();
                        editor.field_y = y.to_string();
                    }
                }
                Task::none()
            }
            Message::FieldX(v) => {
                if let Some(editor) = &mut self.editor {
                    editor.field_x = v;
                    commit_selected_head(editor);
                }
                Task::none()
            }
            Message::FieldY(v) => {
                if let Some(editor) = &mut self.editor {
                    editor.field_y = v;
                    commit_selected_head(editor);
                }
                Task::none()
            }
            Message::FieldWidth(v) => {
                if let Some(editor) = &mut self.editor {
                    editor.field_width = v;
                    commit_selected_head(editor);
                }
                Task::none()
            }
            Message::FieldHeight(v) => {
                if let Some(editor) = &mut self.editor {
                    editor.field_height = v;
                    commit_selected_head(editor);
                }
                Task::none()
            }
            Message::FieldRefresh(v) => {
                if let Some(editor) = &mut self.editor {
                    editor.field_refresh = v;
                    commit_selected_head(editor);
                }
                Task::none()
            }
            Message::FieldScale(v) => {
                if let Some(editor) = &mut self.editor {
                    editor.field_scale = v;
                    commit_selected_head(editor);
                }
                Task::none()
            }
            Message::SwapSelectA(hint) => {
                if let Some(editor) = &mut self.editor {
                    editor.swap_a = Some(hint);
                }
                Task::none()
            }
            Message::SwapSelectB(hint) => {
                if let Some(editor) = &mut self.editor {
                    editor.swap_b = Some(hint);
                }
                Task::none()
            }
            Message::ToggleSwap => {
                let Some(editor) = &self.editor else {
                    return Task::none();
                };
                let (Some(a), Some(b)) = (editor.swap_a.clone(), editor.swap_b.clone()) else {
                    return Task::none();
                };
                if a == b {
                    return Task::none();
                }
                Task::perform(toggle_swap(editor.profile.id.clone(), a, b), Message::SwapToggled)
            }
            Message::SwapToggled(Ok(profile)) => {
                if let Some(editor) = &mut self.editor {
                    editor.profile = profile;
                    editor.status = Some("Swap updated.".to_string());
                    editor.error = None;
                }
                Task::none()
            }
            Message::SwapToggled(Err(e)) => {
                if let Some(editor) = &mut self.editor {
                    editor.error = Some(e);
                }
                Task::none()
            }
            Message::SetPolicy(policy) => {
                let Some(editor) = &self.editor else {
                    return Task::none();
                };
                Task::perform(
                    set_policy(editor.profile.id.clone(), policy),
                    Message::PolicySet,
                )
            }
            Message::PolicySet(Ok(profile)) => {
                if let Some(editor) = &mut self.editor {
                    editor.profile = profile;
                    editor.status = Some("Policy updated.".to_string());
                    editor.error = None;
                }
                Task::none()
            }
            Message::PolicySet(Err(e)) => {
                if let Some(editor) = &mut self.editor {
                    editor.error = Some(e);
                }
                Task::none()
            }
            Message::SaveLayout => {
                let Some(editor) = &self.editor else {
                    return Task::none();
                };
                let heads: Vec<HeadGeometry> = editor
                    .profile
                    .heads
                    .iter()
                    .map(|h| {
                        (
                            h.connector_hint.clone(),
                            h.x,
                            h.y,
                            h.width,
                            h.height,
                            h.refresh_mhz,
                            h.scale,
                            h.transform.clone(),
                        )
                    })
                    .collect();
                Task::perform(
                    save_geometry(editor.profile.id.clone(), heads),
                    Message::LayoutSaved,
                )
            }
            Message::LayoutSaved(Ok(())) => {
                if let Some(editor) = &mut self.editor {
                    editor.status = Some("Saved and applied.".to_string());
                    editor.error = None;
                }
                Task::perform(load(), Message::Loaded)
            }
            Message::LayoutSaved(Err(e)) => {
                if let Some(editor) = &mut self.editor {
                    editor.error = Some(e);
                }
                Task::none()
            }
        }
    }

    fn view(&self, scale: FontScale) -> Element<'_, Message> {
        if !self.connected {
            return container(
                column![
                    scaled_text("Monitors", 22.0, scale),
                    scaled_text(
                        self.error
                            .clone()
                            .unwrap_or_else(|| "hyprforge-displayd is not running.".to_string()),
                        14.0,
                        scale,
                    ),
                    meta_text(
                        "Start it with: systemctl --user start hyprforge-displayd",
                        13.0,
                        scale,
                    ),
                    container(primary_button("Retry").on_press(Message::Refresh))
                        .width(Length::Fill)
                        .align_x(iced::alignment::Horizontal::Right),
                ]
                .spacing(spacing::SM),
            )
            .padding(spacing::XL)
            .into();
        }

        let mut content = column![scaled_text("Monitors", 22.0, scale)].spacing(spacing::LG);

        if !self.competing_monitor_rules.is_empty() {
            content = content.push(section(
                "Warnings",
                scale,
                scaled_text(
                    format!(
                        "hl.monitor() rules found in {} — these are re-applied on every \
                         hyprctl reload and may override Hyprforge's auto-applied layout. \
                         Hyprforge will never edit these files for you.",
                        self.competing_monitor_rules.join(", ")
                    ),
                    13.0,
                    scale,
                ),
            ));
        }

        match &self.editor {
            Some(editor) => content = content.push(self.editor_body(editor, scale)),
            None => {
                let msg = if self.current_profile_id.is_some() {
                    "Loading current setup…"
                } else {
                    "Hyprforge hasn't matched a saved profile to this display setup yet — \
                     it will learn one automatically."
                };
                content = content.push(section("Current Setup", scale, meta_text(msg, 14.0, scale)));
            }
        }

        content = content.push(meta_text(
            format!("Fingerprint: {}", self.current_fingerprint),
            11.0,
            scale,
        ));
        if let Some(event) = &self.last_event {
            content = content.push(meta_text(event.clone(), 11.0, scale));
        }

        content = content.push(
            container(
                secondary_button(if self.show_other_profiles {
                    "Hide other display profiles"
                } else {
                    "Other display profiles"
                })
                .on_press(Message::ToggleOtherProfiles),
            )
            .width(Length::Fill)
            .align_x(iced::alignment::Horizontal::Right),
        );

        if self.show_other_profiles {
            let mut list = column![].spacing(spacing::SM);
            if self.profiles.is_empty() {
                list = list.push(meta_text(
                    "No profiles yet — connect a display configuration and Hyprforge will learn it.",
                    14.0,
                    scale,
                ));
            }
            for (i, p) in self.profiles.iter().enumerate() {
                if i > 0 {
                    list = list.push(divider());
                }
                let is_current = Some(&p.id) == self.current_profile_id.as_ref();
                let row_el: Element<'_, Message> = match &self.renaming {
                    Some((id, draft)) if id == &p.id => row![
                        text_input("Profile name", draft)
                            .on_input(Message::RenameInput)
                            .on_submit(Message::RenameSubmit)
                            .width(Length::Fill),
                        primary_button("Save").on_press(Message::RenameSubmit),
                        secondary_button("Cancel").on_press(Message::RenameCancelled),
                    ]
                    .spacing(spacing::SM)
                    .padding([spacing::SM, 0.0])
                    .into(),
                    _ => self.profile_row(p, scale, is_current),
                };
                list = list.push(row_el);
            }
            content = content.push(section(
                "Other Display Profiles",
                scale,
                container(scrollable(list).height(Length::Shrink)).max_height(360.0),
            ));
        }

        content = content.push(
            container(secondary_button("Refresh").on_press(Message::Refresh))
                .width(Length::Fill)
                .align_x(iced::alignment::Horizontal::Right),
        );

        container(content).padding(spacing::LG).into()
    }

    fn subscription(&self) -> Subscription<Message> {
        if self.connected {
            Subscription::run(signal_stream)
        } else {
            Subscription::none()
        }
    }
}

impl DisplaysModule {
    /// The canvas + property panel + policy/swap controls — the "editing a
    /// profile" view, embedded directly in the Monitors screen rather than
    /// behind a separate button (Windows-Display-Settings-style: land on
    /// the diagram, not a summary card).
    fn editor_body<'a>(&'a self, editor: &'a LayoutEditor, scale: FontScale) -> Element<'a, Message> {
        let mut body = column![].spacing(spacing::LG);

        if Some(&editor.profile.id) != self.current_profile_id.as_ref() {
            let mut notice = row![meta_text(
                format!(
                    "Editing '{}' — not the currently-applied setup.",
                    editor.profile.name
                ),
                12.0,
                scale,
            )]
            .spacing(spacing::SM)
            .align_y(iced::Alignment::Center);
            if let Some(current_id) = &self.current_profile_id {
                notice = notice
                    .push(secondary_button("Back to current").on_press(Message::EditLayout(current_id.clone())));
            }
            body = body.push(notice);
        }

        let canvas_heads: Vec<CanvasHead> = editor
            .profile
            .heads
            .iter()
            .map(|h| CanvasHead {
                connector_hint: h.connector_hint.clone(),
                x: h.x,
                y: h.y,
                width: h.width,
                height: h.height,
                enabled: h.enabled,
            })
            .collect();
        let canvas = LayoutCanvas::new(
            canvas_heads,
            editor.selected.clone(),
            Message::SelectHead,
            Message::HeadDragMoved,
        )
        .into_element();
        body = body.push(section("Layout — drag or click a monitor to select it", scale, canvas));

        let head_hints: Vec<String> = editor
            .profile
            .heads
            .iter()
            .map(|h| h.connector_hint.clone())
            .collect();
        let monitor_picker = row_field(
            "Monitor",
            iced::widget::pick_list(head_hints, editor.selected.clone(), Message::SelectHead)
                .placeholder("Select a monitor"),
        );

        let properties: Element<'_, Message> = if editor.selected_head().is_some() {
            let resolution_field: Element<'_, Message> = if editor.available_modes.is_empty() {
                column![
                    row_field(
                        "Width (px)",
                        text_input("1920", &editor.field_width).on_input(Message::FieldWidth),
                    ),
                    row_field(
                        "Height (px)",
                        text_input("1080", &editor.field_height).on_input(Message::FieldHeight),
                    ),
                    row_field(
                        "Refresh rate (Hz)",
                        text_input("60", &editor.field_refresh).on_input(Message::FieldRefresh),
                    ),
                    meta_text(
                        "This monitor isn't currently connected, so its supported \
                         resolutions aren't known — enter values manually.",
                        11.0,
                        scale,
                    ),
                ]
                .spacing(spacing::SM)
                .into()
            } else {
                let current_mode = editor
                    .field_width
                    .trim()
                    .parse::<i32>()
                    .ok()
                    .zip(editor.field_height.trim().parse::<i32>().ok())
                    .zip(editor.field_refresh.trim().parse::<f64>().ok())
                    .and_then(|((width, height), refresh_hz)| {
                        let refresh_mhz = (refresh_hz * 1000.0).round() as i32;
                        editor
                            .available_modes
                            .iter()
                            .find(|m| m.width == width && m.height == height && m.refresh_mhz == refresh_mhz)
                            .cloned()
                    });
                row_field(
                    "Resolution",
                    iced::widget::pick_list(
                        editor.available_modes.clone(),
                        current_mode,
                        Message::ModeSelected,
                    )
                    .placeholder("Select a resolution"),
                )
            };

            let orientation_field = row_field(
                "Orientation",
                iced::widget::pick_list(
                    TRANSFORM_LABELS.to_vec(),
                    Some(editor.field_transform.as_str()),
                    |label: &str| Message::FieldTransform(label.to_string()),
                ),
            );

            column![
                monitor_picker,
                row_field(
                    "Position X",
                    text_input("0", &editor.field_x).on_input(Message::FieldX),
                ),
                row_field(
                    "Position Y",
                    text_input("0", &editor.field_y).on_input(Message::FieldY),
                ),
                resolution_field,
                row_field(
                    "Scale (%)",
                    text_input("100", &editor.field_scale).on_input(Message::FieldScale),
                ),
                orientation_field,
            ]
            .spacing(spacing::SM)
            .into()
        } else {
            column![monitor_picker, meta_text("This profile has no heads.", 13.0, scale)]
                .spacing(spacing::SM)
                .into()
        };
        body = body.push(section("Selected monitor", scale, properties));

        let swap_hints: Vec<String> = editor
            .profile
            .heads
            .iter()
            .map(|h| h.connector_hint.clone())
            .collect();
        let swap_row = row![
            iced::widget::pick_list(swap_hints.clone(), editor.swap_a.clone(), Message::SwapSelectA)
                .placeholder("Head A"),
            iced::widget::pick_list(swap_hints, editor.swap_b.clone(), Message::SwapSelectB)
                .placeholder("Head B"),
            {
                let ready = matches!(
                    (&editor.swap_a, &editor.swap_b),
                    (Some(a), Some(b)) if a != b
                );
                let btn = secondary_button("Toggle Swap");
                if ready {
                    btn.on_press(Message::ToggleSwap)
                } else {
                    btn
                }
            },
        ]
        .spacing(spacing::SM)
        .align_y(iced::Alignment::Center);

        let policy_button = |label: &'static str, value: &'static str| {
            let active = editor.profile.extra_output_policy == value;
            let btn = if active {
                primary_button(label)
            } else {
                secondary_button(label)
            };
            btn.on_press(Message::SetPolicy(value.to_string()))
        };
        let policy_row = row![
            policy_button("Extend Right", "extend_right"),
            policy_button("Mirror", "mirror"),
            policy_button("Disable", "disable"),
        ]
        .spacing(spacing::SM);

        body = body.push(section("Extra output policy", scale, policy_row));
        body = body.push(section(
            "Swap heads (for duplicate/blank-serial identities)",
            scale,
            swap_row,
        ));

        if let Some(status) = &editor.status {
            body = body.push(meta_text(status.clone(), 13.0, scale));
        }
        if let Some(err) = &editor.error {
            body = body.push(scaled_text(format!("Error: {err}"), 13.0, scale));
        }

        body = body.push(
            container(
                row![
                    secondary_button("Discard changes").on_press(Message::DiscardEdits),
                    primary_button("Save & Apply").on_press(Message::SaveLayout),
                ]
                .spacing(spacing::SM),
            )
            .width(Length::Fill)
            .align_x(iced::alignment::Horizontal::Right),
        );

        body.into()
    }
}

async fn connect() -> Result<zbus::Connection, String> {
    zbus::Connection::session().await.map_err(|e| e.to_string())
}

async fn load() -> Result<LoadedState, String> {
    let conn = connect().await?;
    let proxy = DisplaydProxy::new(&conn).await.map_err(|e| e.to_string())?;
    let profiles = proxy
        .list_profiles()
        .await
        .map_err(|e| e.to_string())?
        .into_iter()
        .map(|(id, name, head_count, last_used)| ProfileInfo {
            id,
            name,
            head_count,
            last_used,
        })
        .collect();
    let current_fingerprint = proxy
        .get_current_fingerprint()
        .await
        .map_err(|e| e.to_string())?;
    let competing_monitor_rules = proxy
        .competing_monitor_rules()
        .await
        .map_err(|e| e.to_string())?;
    Ok(LoadedState {
        profiles,
        current_fingerprint,
        competing_monitor_rules,
    })
}

async fn apply(id: String) -> (String, Result<(), String>) {
    let result = apply_inner(&id).await;
    (id, result)
}

async fn apply_inner(id: &str) -> Result<(), String> {
    let conn = connect().await?;
    let proxy = DisplaydProxy::new(&conn).await.map_err(|e| e.to_string())?;
    proxy.apply_profile(id).await.map_err(|e| e.to_string())
}

async fn rename(id: String, new_name: String) -> Result<(), String> {
    let conn = connect().await?;
    let proxy = DisplaydProxy::new(&conn).await.map_err(|e| e.to_string())?;
    proxy
        .rename_profile(&id, &new_name)
        .await
        .map_err(|e| e.to_string())
}

async fn load_profile(id: String) -> Result<ProfileDetail, String> {
    let conn = connect().await?;
    let proxy = DisplaydProxy::new(&conn).await.map_err(|e| e.to_string())?;
    let json = proxy.get_profile(&id).await.map_err(|e| e.to_string())?;
    serde_json::from_str(&json).map_err(|e| e.to_string())
}

async fn toggle_swap(profile_id: String, a: String, b: String) -> Result<ProfileDetail, String> {
    let conn = connect().await?;
    let proxy = DisplaydProxy::new(&conn).await.map_err(|e| e.to_string())?;
    proxy
        .swap_heads(&profile_id, &a, &b)
        .await
        .map_err(|e| e.to_string())?;
    load_profile(profile_id).await
}

async fn set_policy(profile_id: String, policy: String) -> Result<ProfileDetail, String> {
    let conn = connect().await?;
    let proxy = DisplaydProxy::new(&conn).await.map_err(|e| e.to_string())?;
    proxy
        .set_extra_output_policy(&profile_id, &policy)
        .await
        .map_err(|e| e.to_string())?;
    load_profile(profile_id).await
}

/// `(connector_hint, x, y, width, height, refresh_mhz, scale, transform)`.
type HeadGeometry = (String, i32, i32, i32, i32, i32, f64, String);

async fn save_geometry(profile_id: String, heads: Vec<HeadGeometry>) -> Result<(), String> {
    let conn = connect().await?;
    let proxy = DisplaydProxy::new(&conn).await.map_err(|e| e.to_string())?;
    for (connector_hint, x, y, width, height, refresh_mhz, scale, transform) in heads {
        proxy
            .set_head_geometry(
                &profile_id,
                &connector_hint,
                x,
                y,
                width,
                height,
                refresh_mhz,
                scale,
                &transform,
            )
            .await
            .map_err(|e| e.to_string())?;
    }
    proxy
        .apply_profile(&profile_id)
        .await
        .map_err(|e| e.to_string())
}

async fn fetch_modes(connector_hint: String) -> Vec<(i32, i32, i32, bool)> {
    let Ok(conn) = connect().await else {
        return Vec::new();
    };
    let Ok(proxy) = DisplaydProxy::new(&conn).await else {
        return Vec::new();
    };
    proxy.get_available_modes(&connector_hint).await.unwrap_or_default()
}

fn signal_stream() -> impl iced::futures::Stream<Item = Message> {
    iced::stream::channel(100, |mut output| async move {
        loop {
            if let Err(e) = forward_signals(&mut output).await {
                tracing::warn!(error = %e, "displayd signal stream disconnected; retrying in 3s");
            }
            tokio::time::sleep(std::time::Duration::from_secs(3)).await;
        }
    })
}

async fn forward_signals(
    output: &mut iced::futures::channel::mpsc::Sender<Message>,
) -> anyhow::Result<()> {
    use iced::futures::SinkExt;
    use iced::futures::StreamExt;

    let conn = zbus::Connection::session().await?;
    let proxy = DisplaydProxy::new(&conn).await?;
    let mut applied = proxy.receive_profile_applied().await?;
    let mut seen = proxy.receive_new_topology_seen().await?;

    loop {
        tokio::select! {
            next = applied.next() => {
                let Some(signal) = next else { break };
                let args = signal.args()?;
                let _ = output
                    .send(Message::SignalReceived(SignalKind::ProfileApplied {
                        id: args.id.clone(),
                        name: args.name.clone(),
                        tier: args.tier.clone(),
                    }))
                    .await;
            }
            next = seen.next() => {
                let Some(signal) = next else { break };
                let args = signal.args()?;
                let _ = output
                    .send(Message::SignalReceived(SignalKind::NewTopologySeen {
                        summary: args.summary.clone(),
                    }))
                    .await;
            }
        }
    }
    Ok(())
}
