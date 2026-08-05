use crate::modules::layout_canvas::{CanvasHead, LayoutCanvas};
use hyprforge_core::displayd_proxy::DisplaydProxy;
use hyprforge_core::theme::{spacing, FontScale};
use hyprforge_core::widgets::{divider, meta_text, primary_button, scaled_text, secondary_button, section};
use hyprforge_core::SettingsModule;
use iced::widget::{column, container, pick_list, row, scrollable, text_input};
use iced::{Element, Length, Subscription, Task};
use serde::Deserialize;

/// Full geometry for one stored profile, fetched on demand for the layout
/// editor — `ListProfiles`' summary row doesn't carry per-head detail.
/// Field names/renames mirror `hyprforge_displayd::profile::Profile`
/// exactly, since it's what `GetProfile` serializes; kept as a local,
/// GUI-only type rather than a dependency on the (Wayland-heavy) daemon
/// crate.
#[derive(Debug, Clone, Deserialize)]
pub struct ProfileDetail {
    id: String,
    #[allow(dead_code)]
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
    #[allow(dead_code)]
    refresh_mhz: i32,
    #[allow(dead_code)]
    scale: f64,
    #[allow(dead_code)]
    transform: String,
    enabled: bool,
}

struct LayoutEditor {
    profile: ProfileDetail,
    swap_a: Option<String>,
    swap_b: Option<String>,
    status: Option<String>,
    error: Option<String>,
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
    ToggleAdvanced,
    EditLayout(String),
    LayoutLoaded(Result<ProfileDetail, String>),
    HeadDragMoved(String, i32, i32),
    SwapSelectA(String),
    SwapSelectB(String),
    ToggleSwap,
    SwapToggled(Result<ProfileDetail, String>),
    SetPolicy(String),
    PolicySet(Result<ProfileDetail, String>),
    SaveLayout,
    LayoutSaved(Result<(), String>),
    CloseEditor,
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
    /// Managing every stored profile (not just the one that's active) is
    /// the power-user case — collapsed by default so the common case
    /// ("here's what's applied right now") isn't buried under it.
    show_advanced: bool,
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
                show_advanced: false,
                editor: None,
            },
            Task::perform(load(), Message::Loaded),
        )
    }

    fn current_profile(&self) -> Option<&ProfileInfo> {
        let id = self.current_profile_id.as_ref()?;
        self.profiles.iter().find(|p| &p.id == id)
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
                secondary_button("Edit Layout").on_press(Message::EditLayout(p.id.clone())),
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
                Task::none()
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
            Message::ToggleAdvanced => {
                self.show_advanced = !self.show_advanced;
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
                Task::perform(load(), Message::Loaded)
            }
            Message::EditLayout(id) => Task::perform(load_profile(id), Message::LayoutLoaded),
            Message::LayoutLoaded(Ok(profile)) => {
                self.editor = Some(LayoutEditor {
                    profile,
                    swap_a: None,
                    swap_b: None,
                    status: None,
                    error: None,
                });
                Task::none()
            }
            Message::LayoutLoaded(Err(e)) => {
                self.error = Some(e);
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
                let heads: Vec<(String, i32, i32)> = editor
                    .profile
                    .heads
                    .iter()
                    .map(|h| (h.connector_hint.clone(), h.x, h.y))
                    .collect();
                Task::perform(
                    save_positions(editor.profile.id.clone(), heads),
                    Message::LayoutSaved,
                )
            }
            Message::LayoutSaved(Ok(())) => {
                self.editor = None;
                Task::perform(load(), Message::Loaded)
            }
            Message::LayoutSaved(Err(e)) => {
                if let Some(editor) = &mut self.editor {
                    editor.error = Some(e);
                }
                Task::none()
            }
            Message::CloseEditor => {
                self.editor = None;
                Task::none()
            }
        }
    }

    fn view(&self, scale: FontScale) -> Element<'_, Message> {
        if let Some(editor) = &self.editor {
            return self.editor_view(editor, scale);
        }

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
                    primary_button("Retry").on_press(Message::Refresh),
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

        // The common case: what's applied right now, front and center —
        // renaming/editing the setup you're actually using shouldn't
        // require digging through a list of every profile you've ever had.
        let is_renaming_current = matches!(
            (&self.renaming, &self.current_profile_id),
            (Some((id, _)), Some(current)) if id == current
        );
        let mut current_col = column![].spacing(spacing::SM);
        let current_row: Element<'_, Message> = match (&self.renaming, self.current_profile()) {
            (Some((_, draft)), _) if is_renaming_current => row![
                text_input("Profile name", draft)
                    .on_input(Message::RenameInput)
                    .on_submit(Message::RenameSubmit),
                primary_button("Save").on_press(Message::RenameSubmit),
                secondary_button("Cancel").on_press(Message::RenameCancelled),
            ]
            .spacing(spacing::SM)
            .into(),
            (_, Some(p)) => row![
                column![
                    scaled_text(p.name.clone(), 16.0, scale),
                    meta_text(
                        format!("{} head(s) · last used {}", p.head_count, p.last_used),
                        12.0,
                        scale,
                    ),
                ]
                .spacing(spacing::XS)
                .width(Length::Fill),
                secondary_button("Rename")
                    .on_press(Message::RenameStart(p.id.clone(), p.name.clone())),
                primary_button("Edit Layout").on_press(Message::EditLayout(p.id.clone())),
            ]
            .spacing(spacing::SM)
            .align_y(iced::Alignment::Center)
            .into(),
            (_, None) => meta_text(
                "Hyprforge hasn't matched a saved profile to this display setup yet — \
                 it will learn one automatically.",
                14.0,
                scale,
            )
            .into(),
        };
        current_col = current_col.push(current_row);
        current_col = current_col.push(divider());
        current_col = current_col.push(meta_text(
            format!("Fingerprint: {}", self.current_fingerprint),
            11.0,
            scale,
        ));
        if let Some(event) = &self.last_event {
            current_col = current_col.push(meta_text(event.clone(), 11.0, scale));
        }
        content = content.push(section("Current Setup", scale, current_col));

        content = content.push(
            secondary_button(if self.show_advanced {
                "Hide advanced"
            } else {
                "Show advanced"
            })
            .on_press(Message::ToggleAdvanced),
        );

        if self.show_advanced {
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
                    Some((id, draft)) if id == &p.id && !is_renaming_current => row![
                        text_input("Profile name", draft)
                            .on_input(Message::RenameInput)
                            .on_submit(Message::RenameSubmit),
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
                "All Profiles",
                scale,
                container(scrollable(list).height(Length::Shrink)).max_height(360.0),
            ));
        }

        content = content.push(secondary_button("Refresh").on_press(Message::Refresh));

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
    fn editor_view(&self, editor: &LayoutEditor, scale: FontScale) -> Element<'_, Message> {
        let connector_hints: Vec<String> = editor
            .profile
            .heads
            .iter()
            .map(|h| h.connector_hint.clone())
            .collect();

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
        let canvas = LayoutCanvas::new(canvas_heads, Message::HeadDragMoved).into_element();

        let swap_row = row![
            pick_list(connector_hints.clone(), editor.swap_a.clone(), Message::SwapSelectA)
                .placeholder("Head A"),
            pick_list(connector_hints.clone(), editor.swap_b.clone(), Message::SwapSelectB)
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

        let mut content = column![
            scaled_text("Edit Layout", 22.0, scale),
            meta_text(
                "Drag heads to reposition them, then Save & Apply. Extra output \
                 policy governs any output this profile doesn't cover; head swap \
                 fixes assignment when two heads share an identical (often \
                 blank-serial) EDID identity.",
                13.0,
                scale,
            ),
            section("Layout", scale, canvas),
        ]
        .spacing(spacing::LG);

        content = content.push(section("Extra output policy", scale, policy_row));
        content = content.push(section(
            "Swap heads (for duplicate/blank-serial identities)",
            scale,
            swap_row,
        ));

        if let Some(status) = &editor.status {
            content = content.push(meta_text(status.clone(), 13.0, scale));
        }
        if let Some(err) = &editor.error {
            content = content.push(scaled_text(format!("Error: {err}"), 13.0, scale));
        }

        content = content.push(
            row![
                secondary_button("Cancel").on_press(Message::CloseEditor),
                primary_button("Save & Apply").on_press(Message::SaveLayout),
            ]
            .spacing(spacing::SM),
        );

        container(content).padding(spacing::LG).into()
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

async fn save_positions(profile_id: String, heads: Vec<(String, i32, i32)>) -> Result<(), String> {
    let conn = connect().await?;
    let proxy = DisplaydProxy::new(&conn).await.map_err(|e| e.to_string())?;
    for (connector_hint, x, y) in heads {
        proxy
            .set_head_position(&profile_id, &connector_hint, x, y)
            .await
            .map_err(|e| e.to_string())?;
    }
    proxy
        .apply_profile(&profile_id)
        .await
        .map_err(|e| e.to_string())
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
