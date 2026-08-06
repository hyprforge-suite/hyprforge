use crate::modules::layout_canvas::{CanvasHead, LayoutCanvas};
use hyprforge_core::displayd_proxy::DisplaydProxy;
use hyprforge_core::theme::{spacing, FontScale};
use hyprforge_core::widgets::{
    confirm_dialog, danger_button, divider, meta_text, primary_button, row_field, scaled_text,
    secondary_button, section,
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
    make: String,
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

/// Connector prefixes the kernel uses for internal panels. A laptop's own
/// screen is "Built-in display" to everyone except the DRM subsystem.
const BUILTIN_CONNECTOR_PREFIXES: [&str; 3] = ["edp", "lvds", "dsi"];

impl HeadDetail {
    /// A name a person would recognise on sight.
    ///
    /// `eDP-2` is a connector, not a monitor — it tells the user nothing
    /// about which physical screen they're editing. Every mainstream
    /// display panel (Windows, macOS, GNOME) leads with a friendly name and
    /// demotes the connector to metadata, so this does too. Built-in panels
    /// get a generic label because their EDID model is typically a part
    /// number (`0x0BC9`), which is no more meaningful than the connector.
    fn display_name(&self) -> String {
        let connector = self.connector_hint.to_ascii_lowercase();
        if BUILTIN_CONNECTOR_PREFIXES
            .iter()
            .any(|p| connector.starts_with(p))
        {
            return "Built-in display".to_string();
        }
        let make = self.make.trim();
        let model = self.model.trim();
        match (make.is_empty(), model.is_empty()) {
            // Nothing usable in the EDID — the connector is all we have.
            (_, true) => self.connector_hint.clone(),
            (true, false) => model.to_string(),
            (false, false) => format!("{make} {model}"),
        }
    }
}

/// One entry in a monitor picker. Carries the `connector_hint` as the
/// identity (that's what every message and D-Bus call is keyed on) while
/// showing the friendly name.
#[derive(Debug, Clone, PartialEq)]
struct HeadChoice {
    hint: String,
    label: String,
}

impl HeadChoice {
    fn new(h: &HeadDetail) -> Self {
        HeadChoice {
            hint: h.connector_hint.clone(),
            label: h.display_name(),
        }
    }
}

impl std::fmt::Display for HeadChoice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The connector still earns its place here: it's the only thing
        // that disambiguates two identical monitors in a picker.
        if self.label == self.hint {
            write!(f, "{}", self.label)
        } else {
            write!(f, "{} ({})", self.label, self.hint)
        }
    }
}

/// The scale factors offered in the dropdown, matching the steps Windows
/// exposes. The head's current value is inserted if it isn't one of these,
/// so an existing 160% profile stays representable and editable.
const SCALE_PRESETS: [u32; 6] = [100, 125, 150, 175, 200, 225];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ScaleChoice(u32);

impl std::fmt::Display for ScaleChoice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.0 == 100 {
            write!(f, "100% (recommended)")
        } else {
            write!(f, "{}%", self.0)
        }
    }
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

/// A distinct resolution, independent of refresh rate.
///
/// Windows and macOS both split these into two controls: you pick a
/// resolution, then a refresh rate valid *for* it. One combined
/// "2560 x 1600 @ 165Hz" list multiplies out to every mode a monitor
/// supports, which on a modern panel is dozens of near-identical rows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResolutionOption {
    width: i32,
    height: i32,
    preferred: bool,
}

impl std::fmt::Display for ResolutionOption {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} × {}{}",
            self.width,
            self.height,
            if self.preferred { " (recommended)" } else { "" }
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RefreshOption {
    mhz: i32,
}

impl std::fmt::Display for RefreshOption {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let hz = self.mhz as f64 / 1000.0;
        // Rates like 59.94 are real and must not be rounded away, but the
        // common integer case shouldn't read "165.000 Hz".
        if (hz - hz.round()).abs() < 0.01 {
            write!(f, "{:.0} Hz", hz)
        } else {
            write!(f, "{:.2} Hz", hz)
        }
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
    RevertPending { seconds: u32 },
    RevertResolved { reverted: bool },
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
    ToggleAdvanced,
    DeleteStart(String),
    DeleteConfirm,
    DeleteCancel,
    Deleted(Result<(), String>),
    KeepLayout,
    RevertLayoutNow,
    RevertActionDone(Result<(), String>),
    /// One tick of the revert countdown.
    RevertTick,
    SignalReceived(SignalKind),
    ToggleOtherProfiles,
    EditLayout(String),
    LayoutLoaded(Result<ProfileDetail, String>),
    SelectHead(String),
    ModesLoaded(String, Vec<(i32, i32, i32, bool)>),
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
    /// Fires after edits stop. `u64` is the generation it was scheduled
    /// for; a stale one is ignored (see `apply_generation`).
    ApplySettled(u64),
    LayoutSaved(Result<(), String>),
    ResolutionSelected(ResolutionOption),
    RefreshSelected(RefreshOption),
    ToggleWarnings,
}

/// A profile delete the user has asked for but not yet confirmed.
struct PendingDelete {
    id: String,
    body: String,
    detail: String,
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
    /// A delete awaiting confirmation. Holds pre-rendered strings because
    /// `confirm_dialog` borrows them for the dialog's lifetime, and the
    /// wording differs for the connected topology (which gets re-learned).
    deleting: Option<PendingDelete>,
    /// Seconds left before the daemon rolls back a provisional display
    /// change. `None` means nothing is pending. The daemon owns the real
    /// deadline — this is only what the banner counts down, so a stalled or
    /// killed GUI can't keep a bad layout alive.
    revert_seconds_left: Option<u32>,
    /// Whether the rarely-used per-head controls (numeric position, output
    /// policy, head swaps) are expanded. Collapsed on open.
    show_advanced: bool,
    /// Whether the competing-`hl.monitor()` warning is expanded. It's a
    /// standing condition, not news, so it sits collapsed at the bottom
    /// rather than shouting above the controls every visit.
    show_warnings: bool,
    /// Bumped on every edit. The debounced apply carries the generation it
    /// was scheduled under and does nothing if a newer edit has landed
    /// since — so a drag that emits a message per frame applies once, on
    /// settle, instead of fighting itself.
    apply_generation: u64,
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
                deleting: None,
                revert_seconds_left: None,
                show_advanced: false,
                show_warnings: false,
                apply_generation: 0,
                last_event: None,
                show_other_profiles: false,
                editor: None,
            },
            Task::perform(load(), Message::Loaded),
        )
    }

    /// Seconds left before the daemon rolls back a provisional change, or
    /// `None` when nothing is pending. The app shell reads this to decide
    /// whether the pinned countdown window should exist.
    pub fn revert_seconds_left(&self) -> Option<u32> {
        self.revert_seconds_left
    }

    /// Schedules an apply for shortly after edits stop.
    ///
    /// Settings apply on change rather than behind a Save button: GNOME's
    /// HIG calls for instant-apply pages to have no dismissal button at
    /// all, and Windows' display panel applies immediately and then offers
    /// a timed "keep these settings?" — which is exactly the revert window
    /// the daemon already provides. A Save button on top of that would be a
    /// second, weaker safety net.
    ///
    /// The debounce exists because edits arrive in bursts: a canvas drag
    /// emits a message per frame, and a text field one per keystroke.
    /// Applying each would thrash the compositor.
    fn schedule_apply(&mut self) -> Task<Message> {
        self.apply_generation = self.apply_generation.wrapping_add(1);
        let generation = self.apply_generation;
        Task::perform(
            async move {
                tokio::time::sleep(std::time::Duration::from_millis(700)).await;
                generation
            },
            Message::ApplySettled,
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
                danger_button("Delete", Message::DeleteStart(p.id.clone())),
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
            Message::DeleteStart(id) => {
                let name = self
                    .profiles
                    .iter()
                    .find(|p| p.id == id)
                    .map(|p| p.name.clone())
                    .unwrap_or_else(|| id.clone());
                // Deleting the profile for what's plugged in right now isn't
                // permanent — auto-learn recreates one on the next topology
                // settle. Say that plainly instead of implying it's gone for
                // good (vision pillar #4: no faith required).
                let is_current = self.current_profile_id.as_deref() == Some(id.as_str());
                let body = if is_current {
                    format!(
                        "\"{name}\" is the profile for your current monitors. Deleting \
                         it discards its saved arrangement; Hyprforge will learn a \
                         fresh one from your live layout the next time your monitors \
                         change. It won't stay deleted while this setup is plugged in."
                    )
                } else {
                    format!(
                        "\"{name}\" will be forgotten, along with its saved arrangement, \
                         head swaps, and output policy. If you plug this set of monitors \
                         back in, Hyprforge will learn them again from scratch."
                    )
                };
                self.deleting = Some(PendingDelete {
                    detail: format!("Profile: {name}\nID: {id}"),
                    id,
                    body,
                });
                Task::none()
            }
            Message::ToggleAdvanced => {
                self.show_advanced = !self.show_advanced;
                Task::none()
            }
            Message::DeleteCancel => {
                self.deleting = None;
                Task::none()
            }
            Message::DeleteConfirm => {
                let Some(pending) = self.deleting.take() else {
                    return Task::none();
                };
                // Drop the editor if it's showing what we just deleted, so
                // the panel can't keep editing a profile that no longer
                // exists (every save would fail with "no such profile").
                if self
                    .editor
                    .as_ref()
                    .is_some_and(|e| e.profile.id == pending.id)
                {
                    self.editor = None;
                }
                Task::perform(delete(pending.id), Message::Deleted)
            }
            Message::Deleted(Ok(())) => {
                self.error = None;
                Task::perform(load(), Message::Loaded)
            }
            Message::Deleted(Err(e)) => {
                self.error = Some(e);
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
                    SignalKind::RevertPending { seconds } => {
                        self.revert_seconds_left = Some(seconds);
                        return Task::none();
                    }
                    SignalKind::RevertResolved { reverted } => {
                        self.revert_seconds_left = None;
                        let msg = if reverted {
                            "Display change reverted."
                        } else {
                            "Display change kept."
                        };
                        self.last_event = Some(msg.to_string());
                        // A revert rolls the stored profile back too, so the
                        // editor's copy is stale either way.
                        let reload = Task::perform(load(), Message::Loaded);
                        let refresh_editor = match &self.editor {
                            Some(e) => Task::perform(
                                load_profile(e.profile.id.clone()),
                                Message::LayoutLoaded,
                            ),
                            None => Task::none(),
                        };
                        return Task::batch([reload, refresh_editor]);
                    }
                });
                let reload = Task::perform(load(), Message::Loaded);
                Task::batch([reload, self.maybe_autoload_editor()])
            }
            Message::RevertTick => {
                // Purely cosmetic: the daemon's timer is the one that counts.
                // Floor at 1 rather than 0 so the banner never reads "0s" for
                // however long the round trip to the daemon takes.
                if let Some(left) = self.revert_seconds_left {
                    self.revert_seconds_left = Some(left.saturating_sub(1).max(1));
                }
                Task::none()
            }
            Message::KeepLayout => {
                self.revert_seconds_left = None;
                Task::perform(confirm_layout(), Message::RevertActionDone)
            }
            Message::RevertLayoutNow => {
                self.revert_seconds_left = None;
                Task::perform(revert_layout(), Message::RevertActionDone)
            }
            Message::RevertActionDone(Ok(())) => Task::none(),
            Message::RevertActionDone(Err(e)) => {
                self.error = Some(e);
                Task::none()
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
            Message::FieldTransform(label) => {
                if let Some(editor) = &mut self.editor {
                    editor.field_transform = label;
                    commit_selected_head(editor);
                }
                self.schedule_apply()
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
                // Fires per frame while dragging; the debounce collapses the
                // whole gesture into one apply when the pointer settles.
                self.schedule_apply()
            }
            Message::FieldX(v) => {
                if let Some(editor) = &mut self.editor {
                    editor.field_x = v;
                    commit_selected_head(editor);
                }
                self.schedule_apply()
            }
            Message::FieldY(v) => {
                if let Some(editor) = &mut self.editor {
                    editor.field_y = v;
                    commit_selected_head(editor);
                }
                self.schedule_apply()
            }
            Message::FieldWidth(v) => {
                if let Some(editor) = &mut self.editor {
                    editor.field_width = v;
                    commit_selected_head(editor);
                }
                self.schedule_apply()
            }
            Message::FieldHeight(v) => {
                if let Some(editor) = &mut self.editor {
                    editor.field_height = v;
                    commit_selected_head(editor);
                }
                self.schedule_apply()
            }
            Message::FieldRefresh(v) => {
                if let Some(editor) = &mut self.editor {
                    editor.field_refresh = v;
                    commit_selected_head(editor);
                }
                self.schedule_apply()
            }
            Message::FieldScale(v) => {
                if let Some(editor) = &mut self.editor {
                    editor.field_scale = v;
                    commit_selected_head(editor);
                }
                self.schedule_apply()
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
            Message::ApplySettled(generation) => {
                // Ignore a timer from an edit that's already been
                // superseded — only the last one in a burst applies.
                if generation != self.apply_generation {
                    return Task::none();
                }
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
                let is_current = self.current_profile_id.as_ref() == Some(&editor.profile.id);
                Task::perform(
                    save_geometry(editor.profile.id.clone(), heads, is_current),
                    Message::LayoutSaved,
                )
            }
            Message::ResolutionSelected(res) => {
                if let Some(editor) = &mut self.editor {
                    editor.field_width = res.width.to_string();
                    editor.field_height = res.height.to_string();
                    // The old refresh rate may not exist at the new
                    // resolution; snap to the fastest one that does.
                    if let Some(best) = editor
                        .available_modes
                        .iter()
                        .filter(|m| m.width == res.width && m.height == res.height)
                        .map(|m| m.refresh_mhz)
                        .max()
                    {
                        let current = editor
                            .field_refresh
                            .trim()
                            .parse::<f64>()
                            .map(|hz| (hz * 1000.0).round() as i32)
                            .unwrap_or(0);
                        let still_valid = editor.available_modes.iter().any(|m| {
                            m.width == res.width
                                && m.height == res.height
                                && m.refresh_mhz == current
                        });
                        if !still_valid {
                            editor.field_refresh = format!("{:.0}", best as f64 / 1000.0);
                        }
                    }
                    commit_selected_head(editor);
                }
                self.schedule_apply()
            }
            Message::RefreshSelected(rate) => {
                if let Some(editor) = &mut self.editor {
                    editor.field_refresh = format!("{:.3}", rate.mhz as f64 / 1000.0);
                    commit_selected_head(editor);
                }
                self.schedule_apply()
            }
            Message::ToggleWarnings => {
                self.show_warnings = !self.show_warnings;
                Task::none()
            }
            Message::LayoutSaved(Ok(())) => {
                if let Some(editor) = &mut self.editor {
                    editor.status = Some("Saved.".to_string());
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

        if let Some(pending) = &self.deleting {
            return container(confirm_dialog(
                "Delete this display profile?",
                &pending.body,
                &pending.detail,
                Message::DeleteConfirm,
                Message::DeleteCancel,
            ))
            .center(Length::Fill)
            .into();
        }

        let mut content = column![scaled_text("Monitors", 22.0, scale)].spacing(spacing::LG);

        // The countdown lives in its own pinned, always-visible window (see
        // `revert_popup_view` in main.rs) rather than here. A change that
        // scrambles a display can leave this window unreadable or on a
        // workspace the user can't see — which is exactly when the prompt
        // matters most.


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

        // The fingerprint is a 64-char hash — diagnostic output, not
        // something a user acts on, so it lives behind Advanced rather than
        // sitting under the controls on every visit.
        if self.show_advanced {
            content = content.push(meta_text(
                format!("Fingerprint: {}", self.current_fingerprint),
                11.0,
                scale,
            ));
        }
        if let Some(event) = &self.last_event {
            content = content.push(meta_text(event.clone(), 11.0, scale));
        }

        let profiles_button = secondary_button(if self.show_other_profiles {
            "Hide other display profiles"
        } else {
            "Other display profiles"
        })
        .on_press(Message::ToggleOtherProfiles);

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
                container(scrollable(list).width(Length::Fill).height(Length::Shrink))
                .max_height(360.0),
            ));
        }

        // A standing condition, not news: it's true on every visit until
        // the user edits their own config, so it sits collapsed at the
        // bottom instead of pushing the actual controls below the fold.
        if !self.competing_monitor_rules.is_empty() {
            let count = self.competing_monitor_rules.len();
            let summary = row![
                meta_text(
                    format!(
                        "{count} config file{} may override Hyprforge's layout.",
                        if count == 1 { "" } else { "s" }
                    ),
                    12.0,
                    scale,
                ),
                secondary_button(if self.show_warnings { "Hide" } else { "Details" })
                    .on_press(Message::ToggleWarnings),
            ]
            .spacing(spacing::SM)
            .align_y(iced::Alignment::Center);

            let block: Element<'_, Message> = if self.show_warnings {
                column![
                    summary,
                    scaled_text(
                        format!(
                            "hl.monitor() rules found in {} — these are re-applied on \
                             every hyprctl reload and may override Hyprforge's \
                             auto-applied layout. Hyprforge will never edit these files \
                             for you.",
                            self.competing_monitor_rules.join(", ")
                        ),
                        13.0,
                        scale,
                    ),
                ]
                .spacing(spacing::SM)
                .into()
            } else {
                summary.into()
            };
            content = content.push(block);
        }

        // One footer row rather than two separately right-aligned buttons
        // stacked on top of each other.
        content = content.push(
            container(
                row![profiles_button, secondary_button("Refresh").on_press(Message::Refresh)]
                    .spacing(spacing::SM),
            )
            .width(Length::Fill)
            .align_x(iced::alignment::Horizontal::Right),
        );

        container(content).padding(spacing::LG).into()
    }

    fn subscription(&self) -> Subscription<Message> {
        if !self.connected {
            return Subscription::none();
        }
        let signals = Subscription::run(signal_stream);
        match self.revert_seconds_left {
            Some(_) => Subscription::batch([
                signals,
                iced::time::every(std::time::Duration::from_secs(1))
                    .map(|_| Message::RevertTick),
            ]),
            None => signals,
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
                label: h.display_name(),
                x: h.x,
                y: h.y,
                width: h.width,
                height: h.height,
                enabled: h.enabled,
            })
            .collect();

        // The arrangement canvas only earns its space when there's an
        // arrangement to make. With one display it's a 380px box holding a
        // single rectangle you can't meaningfully drag — Windows, macOS and
        // GNOME all hide the diagram entirely below two displays.
        if canvas_heads.len() > 1 {
            let canvas = LayoutCanvas::new(
                canvas_heads,
                editor.selected.clone(),
                Message::SelectHead,
                Message::HeadDragMoved,
            )
            .into_element();
            body = body.push(section(
                "Arrangement — drag a monitor to match your desk",
                scale,
                canvas,
            ));
        }

        let choices: Vec<HeadChoice> = editor.profile.heads.iter().map(HeadChoice::new).collect();
        let selected_choice = editor
            .selected
            .as_ref()
            .and_then(|hint| choices.iter().find(|c| &c.hint == hint).cloned());
        // With a single display there is nothing to pick between, so the
        // picker is just a row restating the name — show it as a heading.
        let monitor_picker: Element<'_, Message> = if choices.len() > 1 {
            row_field(
                "Monitor",
                iced::widget::pick_list(choices, selected_choice, |c: HeadChoice| {
                    Message::SelectHead(c.hint)
                })
                .placeholder("Select a monitor"),
            )
        } else {
            match selected_choice {
                Some(c) => column![
                    scaled_text(c.label.clone(), 16.0, scale),
                    meta_text(c.hint.clone(), 12.0, scale),
                ]
                .spacing(spacing::XS)
                .into(),
                None => meta_text("No monitor selected.", 13.0, scale).into(),
            }
        };

        let properties: Element<'_, Message> = if editor.selected_head().is_some() {
            // The stored mode is always offered, even when the live mode
            // list is unavailable, so there's never a bare text-box
            // fallback on the common path.
            let stored = editor
                .field_width
                .trim()
                .parse::<i32>()
                .ok()
                .zip(editor.field_height.trim().parse::<i32>().ok());
            let stored_refresh = editor
                .field_refresh
                .trim()
                .parse::<f64>()
                .map(|hz| (hz * 1000.0).round() as i32)
                .ok();

            let mut resolutions: Vec<ResolutionOption> = Vec::new();
            for m in &editor.available_modes {
                if let Some(existing) = resolutions
                    .iter_mut()
                    .find(|r| r.width == m.width && r.height == m.height)
                {
                    existing.preferred |= m.preferred;
                } else {
                    resolutions.push(ResolutionOption {
                        width: m.width,
                        height: m.height,
                        preferred: m.preferred,
                    });
                }
            }
            if let Some((w, h)) = stored {
                if !resolutions.iter().any(|r| r.width == w && r.height == h) {
                    resolutions.push(ResolutionOption {
                        width: w,
                        height: h,
                        preferred: false,
                    });
                }
            }
            // Largest first — the order every other display panel uses.
            resolutions.sort_by(|a, b| (b.width, b.height).cmp(&(a.width, a.height)));

            let selected_resolution = stored.map(|(w, h)| ResolutionOption {
                width: w,
                height: h,
                preferred: resolutions
                    .iter()
                    .any(|r| r.width == w && r.height == h && r.preferred),
            });

            let mut refresh_rates: Vec<RefreshOption> = editor
                .available_modes
                .iter()
                .filter(|m| stored.is_none_or(|(w, h)| m.width == w && m.height == h))
                .map(|m| RefreshOption { mhz: m.refresh_mhz })
                .collect();
            if let Some(mhz) = stored_refresh {
                if !refresh_rates.iter().any(|r| r.mhz == mhz) {
                    refresh_rates.push(RefreshOption { mhz });
                }
            }
            refresh_rates.sort_by(|a, b| b.mhz.cmp(&a.mhz));
            refresh_rates.dedup();

            let resolution_field: Element<'_, Message> = column![
                row_field(
                    "Resolution",
                    iced::widget::pick_list(
                        resolutions,
                        selected_resolution,
                        Message::ResolutionSelected,
                    )
                    .placeholder("Select a resolution"),
                ),
                row_field(
                    "Refresh rate",
                    iced::widget::pick_list(
                        refresh_rates,
                        stored_refresh.map(|mhz| RefreshOption { mhz }),
                        Message::RefreshSelected,
                    )
                    .placeholder("Select a refresh rate"),
                ),
            ]
            .spacing(spacing::SM)
            .into();

            let orientation_field = row_field(
                "Orientation",
                iced::widget::pick_list(
                    TRANSFORM_LABELS.to_vec(),
                    Some(editor.field_transform.as_str()),
                    |label: &str| Message::FieldTransform(label.to_string()),
                ),
            );

            // Offer the standard steps, plus whatever this head is already
            // set to if it's off-preset — dropping to the nearest preset
            // would silently rescale someone's working setup.
            let current_scale = editor.field_scale.trim().parse::<u32>().ok();
            let mut scale_options: Vec<ScaleChoice> =
                SCALE_PRESETS.iter().copied().map(ScaleChoice).collect();
            if let Some(current) = current_scale {
                if !SCALE_PRESETS.contains(&current) {
                    scale_options.push(ScaleChoice(current));
                    scale_options.sort_by_key(|c| c.0);
                }
            }
            let scale_field = row_field(
                "Scale",
                iced::widget::pick_list(
                    scale_options,
                    current_scale.map(ScaleChoice),
                    |c: ScaleChoice| Message::FieldScale(c.0.to_string()),
                )
                .placeholder("Select a scale"),
            );

            // Scale first, then resolution, then orientation — the Windows
            // ordering, and the rough order of how often each is touched.
            column![monitor_picker, scale_field, resolution_field, orientation_field]
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

        // Everything below is either rarely touched (numeric position —
        // dragging the canvas is the real interaction, and Windows/macOS
        // don't expose coordinates at all) or a fix for a specific problem
        // you only reach after hitting it. Collapsed so the common path is
        // scale/resolution/orientation and nothing else.
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
            if editor.available_modes.is_empty() && editor.selected_head().is_some() {
                body = body.push(section(
                    "Custom resolution",
                    scale,
                    column![
                        // Deliberately doesn't assert *why*. The mode list is
                        // empty when the head isn't plugged in, but also if
                        // the daemon simply couldn't read it — claiming
                        // "not connected" about a screen the user is looking
                        // at is worse than saying nothing.
                        meta_text(
                            "Supported modes for this monitor couldn't be read, so \
                             only its saved resolution is listed above. Set one \
                             manually here if you need a different mode.",
                            12.0,
                            scale,
                        ),
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
                    ]
                    .spacing(spacing::SM),
                ));
            }

            if editor.selected_head().is_some() {
                // The arrangement canvas is only on screen with 2+ heads, so
                // don't point at something that isn't there.
                let hint = if editor.profile.heads.len() > 1 {
                    "Usually set by dragging on the arrangement above."
                } else {
                    "A single display sits at 0,0 — this only matters once a \
                     second monitor is attached."
                };
                body = body.push(section(
                    "Position",
                    scale,
                    column![
                        meta_text(hint, 12.0, scale),
                        row_field("X", text_input("0", &editor.field_x).on_input(Message::FieldX)),
                        row_field("Y", text_input("0", &editor.field_y).on_input(Message::FieldY)),
                    ]
                    .spacing(spacing::SM),
                ));
            }

            body = body.push(section(
                "Monitors this profile doesn't cover",
                scale,
                column![
                    meta_text(
                        "What to do with a display that's plugged in but isn't part \
                         of this profile.",
                        12.0,
                        scale,
                    ),
                    policy_row,
                ]
                .spacing(spacing::SM),
            ));

            // Only reachable — and only meaningful — with two or more heads.
            if editor.profile.heads.len() > 1 {
                body = body.push(section(
                    "My monitors are swapped",
                    scale,
                    column![
                        meta_text(
                            "If two identical monitors got each other's settings, \
                             Hyprforge can't tell them apart from their EDID alone. \
                             Pick both and swap them.",
                            12.0,
                            scale,
                        ),
                        swap_row,
                    ]
                    .spacing(spacing::SM),
                ));
            }
        }

        if let Some(status) = &editor.status {
            body = body.push(meta_text(status.clone(), 13.0, scale));
        }
        if let Some(err) = &editor.error {
            body = body.push(scaled_text(format!("Error: {err}"), 13.0, scale));
        }

        // No Save/Discard bar: changes apply as you make them, and the
        // daemon's confirm/revert banner is the undo. GNOME's HIG is
        // explicit that instant-apply pages carry no dismissal button, and
        // Windows' display panel behaves the same way — apply, then offer a
        // timed revert. Two competing safety mechanisms would be worse than
        // one good one.
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

/// Always the reversible variant here: a GUI apply is exactly the case the
/// confirm/revert window exists for. `ApplyProfile` stays immediate for
/// scripted `displayctl` use, which has no banner to click.
async fn apply_inner(id: &str) -> Result<(), String> {
    let conn = connect().await?;
    let proxy = DisplaydProxy::new(&conn).await.map_err(|e| e.to_string())?;
    proxy
        .apply_profile_reversible(id)
        .await
        .map(|_seconds| ())
        .map_err(|e| e.to_string())
}

async fn confirm_layout() -> Result<(), String> {
    let conn = connect().await?;
    let proxy = DisplaydProxy::new(&conn).await.map_err(|e| e.to_string())?;
    proxy.confirm_layout().await.map_err(|e| e.to_string())
}

async fn revert_layout() -> Result<(), String> {
    let conn = connect().await?;
    let proxy = DisplaydProxy::new(&conn).await.map_err(|e| e.to_string())?;
    proxy.revert_layout().await.map_err(|e| e.to_string())
}

async fn rename(id: String, new_name: String) -> Result<(), String> {
    let conn = connect().await?;
    let proxy = DisplaydProxy::new(&conn).await.map_err(|e| e.to_string())?;
    proxy
        .rename_profile(&id, &new_name)
        .await
        .map_err(|e| e.to_string())
}

async fn delete(id: String) -> Result<(), String> {
    let conn = connect().await?;
    let proxy = DisplaydProxy::new(&conn).await.map_err(|e| e.to_string())?;
    proxy.delete_profile(&id).await.map_err(|e| e.to_string())
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

/// Persists head geometry, and — only when `apply` — makes it live.
///
/// Editing a profile that isn't the active one must never switch the user's
/// displays out from under them just because they touched a dropdown. Those
/// edits are saved and take effect the next time that setup is plugged in.
async fn save_geometry(
    profile_id: String,
    heads: Vec<HeadGeometry>,
    apply: bool,
) -> Result<(), String> {
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
    if !apply {
        return Ok(());
    }
    proxy
        .apply_profile_reversible(&profile_id)
        .await
        .map(|_seconds| ())
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
    let mut revert_pending = proxy.receive_revert_pending().await?;
    let mut revert_resolved = proxy.receive_revert_resolved().await?;

    loop {
        tokio::select! {
            next = revert_pending.next() => {
                let Some(signal) = next else { break };
                let args = signal.args()?;
                let _ = output
                    .send(Message::SignalReceived(SignalKind::RevertPending {
                        seconds: args.seconds,
                    }))
                    .await;
            }
            next = revert_resolved.next() => {
                let Some(signal) = next else { break };
                let args = signal.args()?;
                let _ = output
                    .send(Message::SignalReceived(SignalKind::RevertResolved {
                        reverted: args.reverted,
                    }))
                    .await;
            }
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

#[cfg(test)]
mod tests {
    use super::*;

    /// A verbatim `GetProfile` payload, captured from the daemon running
    /// against its mock backend. Kept literal so a rename on the daemon side
    /// (`heads` vs the serialized `head`, say) fails here instead of
    /// silently producing an editor with no monitors in it.
    const GET_PROFILE_JSON: &str = r#"{
        "id": "eb0c69588ccaf30b4715a073d1d7776b6291232267e08088fa6ccb0e5d81119f",
        "name": "1 display incl. BOE 0x0BC9",
        "last_used": "2026-08-06T01:04:31Z",
        "extra_output_policy": "extend_right",
        "head_swaps": [],
        "head": [{
            "make": "BOE", "model": "0x0BC9", "serial": "",
            "connector_hint": "MOCK-1",
            "x": 0, "y": 0, "width": 1920, "height": 1080,
            "refresh_mhz": 60000, "scale": 1.0,
            "transform": "Normal", "enabled": true
        }]
    }"#;

    fn detail() -> ProfileDetail {
        serde_json::from_str(GET_PROFILE_JSON).expect("GetProfile payload must decode")
    }

    #[test]
    fn decodes_a_real_get_profile_payload() {
        let p = detail();
        assert_eq!(p.name, "1 display incl. BOE 0x0BC9");
        assert_eq!(p.extra_output_policy, "extend_right");
        assert_eq!(p.heads.len(), 1, "the daemon serializes heads as \"head\"");
        assert_eq!(p.heads[0].connector_hint, "MOCK-1");
        assert_eq!(p.heads[0].refresh_mhz, 60000);
    }

    fn head(connector: &str, make: &str, model: &str) -> HeadDetail {
        let mut h = detail().heads.remove(0);
        h.connector_hint = connector.to_string();
        h.make = make.to_string();
        h.model = model.to_string();
        h
    }

    #[test]
    fn internal_panels_get_a_generic_friendly_name() {
        // Their EDID model is a part number, so it's no more useful than
        // the connector it replaces.
        assert_eq!(head("eDP-2", "BOE", "0x0BC9").display_name(), "Built-in display");
        assert_eq!(head("LVDS-1", "", "").display_name(), "Built-in display");
        assert_eq!(head("DSI-1", "", "").display_name(), "Built-in display");
    }

    #[test]
    fn external_monitors_are_named_from_their_edid() {
        assert_eq!(head("DP-4", "DELL", "U2720Q").display_name(), "DELL U2720Q");
        assert_eq!(head("DP-4", "", "U2720Q").display_name(), "U2720Q");
    }

    #[test]
    fn a_blank_edid_falls_back_to_the_connector() {
        // Blank EDID is normal, not a bug — showing an empty name would be
        // worse than showing DP-4.
        assert_eq!(head("DP-4", "", "").display_name(), "DP-4");
        assert_eq!(head("HDMI-A-1", "SOMEMAKE", "").display_name(), "HDMI-A-1");
    }

    #[test]
    fn the_picker_keeps_the_connector_to_disambiguate_identical_monitors() {
        // Two of the same model must not render as two identical entries.
        let choice = HeadChoice::new(&head("DP-4", "DELL", "U2720Q"));
        assert_eq!(choice.to_string(), "DELL U2720Q (DP-4)");
        assert_eq!(choice.hint, "DP-4", "the connector stays the identity");

        // No point repeating it when the name already *is* the connector.
        let bare = HeadChoice::new(&head("DP-4", "", ""));
        assert_eq!(bare.to_string(), "DP-4");
    }

    #[test]
    fn refresh_rates_render_without_losing_fractional_values() {
        // 59.94Hz is a real mode; rounding it to 60 would offer the user a
        // rate their monitor doesn't have.
        assert_eq!(RefreshOption { mhz: 165000 }.to_string(), "165 Hz");
        assert_eq!(RefreshOption { mhz: 59940 }.to_string(), "59.94 Hz");
    }

    #[test]
    fn resolutions_mark_the_preferred_mode() {
        let r = ResolutionOption { width: 2560, height: 1600, preferred: true };
        assert_eq!(r.to_string(), "2560 × 1600 (recommended)");
        let plain = ResolutionOption { width: 1920, height: 1080, preferred: false };
        assert_eq!(plain.to_string(), "1920 × 1080");
    }

    #[test]
    fn editor_seeds_its_fields_from_the_first_head() {
        let editor = LayoutEditor::new(detail());
        assert_eq!(editor.selected.as_deref(), Some("MOCK-1"));
        assert_eq!(editor.field_width, "1920");
        // mHz is shown as Hz, and scale as a percentage.
        assert_eq!(editor.field_refresh, "60");
        assert_eq!(editor.field_scale, "100");
    }

    #[test]
    fn a_mid_edit_field_leaves_the_head_untouched() {
        let mut editor = LayoutEditor::new(detail());
        // "-" is what you have after typing the first character of "-100".
        editor.field_x = "-".to_string();
        commit_selected_head(&mut editor);
        assert_eq!(editor.profile.heads[0].x, 0, "a half-typed value must not commit");
    }

    #[test]
    fn zero_or_negative_dimensions_are_rejected() {
        let mut editor = LayoutEditor::new(detail());
        editor.field_width = "0".to_string();
        commit_selected_head(&mut editor);
        assert_eq!(
            editor.profile.heads[0].width, 1920,
            "a zero width would be applied to a real monitor"
        );

        editor.field_width = "1920".to_string();
        editor.field_scale = "0".to_string();
        commit_selected_head(&mut editor);
        assert_eq!(editor.profile.heads[0].scale, 1.0);
    }

    #[test]
    fn valid_fields_commit_with_unit_conversion() {
        let mut editor = LayoutEditor::new(detail());
        editor.field_x = "-1920".to_string();
        editor.field_refresh = "144".to_string();
        editor.field_scale = "150".to_string();
        commit_selected_head(&mut editor);

        let head = &editor.profile.heads[0];
        assert_eq!(head.x, -1920, "negative positions are valid — monitor to the left");
        assert_eq!(head.refresh_mhz, 144000);
        assert_eq!(head.scale, 1.5);
    }
}
