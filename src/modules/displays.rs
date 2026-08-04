use hyprforge_core::displayd_proxy::DisplaydProxy;
use hyprforge_core::theme::{spacing, FontScale};
use hyprforge_core::widgets::{scaled_text, section};
use hyprforge_core::SettingsModule;
use iced::widget::{button, column, container, row, scrollable, text_input};
use iced::{Element, Length, Subscription, Task};

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
    ProfileApplied { name: String, tier: String },
    NewTopologySeen { summary: String },
}

#[derive(Debug, Clone)]
pub enum Message {
    Refresh,
    Loaded(Result<LoadedState, String>),
    Apply(String),
    Applied(Result<(), String>),
    RenameStart(String, String),
    RenameInput(String),
    RenameSubmit,
    RenameCancelled,
    Renamed(Result<(), String>),
    SignalReceived(SignalKind),
}

pub struct DisplaysModule {
    connected: bool,
    error: Option<String>,
    profiles: Vec<ProfileInfo>,
    current_fingerprint: String,
    competing_monitor_rules: Vec<String>,
    renaming: Option<(String, String)>,
    last_event: Option<String>,
}

impl DisplaysModule {
    pub fn new() -> (Self, Task<Message>) {
        (
            DisplaysModule {
                connected: false,
                error: None,
                profiles: Vec::new(),
                current_fingerprint: String::new(),
                competing_monitor_rules: Vec::new(),
                renaming: None,
                last_event: None,
            },
            Task::perform(load(), Message::Loaded),
        )
    }

    fn profile_row(&self, p: &ProfileInfo, scale: FontScale) -> Element<'_, Message> {
        row![
            scaled_text(p.name.clone(), 14.0, scale).width(Length::FillPortion(2)),
            scaled_text(format!("{} head(s)", p.head_count), 14.0, scale)
                .width(Length::FillPortion(1)),
            scaled_text(p.last_used.clone(), 14.0, scale).width(Length::FillPortion(2)),
            button("Apply").on_press(Message::Apply(p.id.clone())),
            button("Rename").on_press(Message::RenameStart(p.id.clone(), p.name.clone())),
        ]
        .spacing(spacing::SM)
        .align_y(iced::Alignment::Center)
        .into()
    }
}

impl SettingsModule for DisplaysModule {
    type Message = Message;

    fn title(&self) -> &str {
        "Displays"
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
                Task::none()
            }
            Message::Loaded(Err(e)) => {
                self.connected = false;
                self.error = Some(e);
                Task::none()
            }
            Message::Apply(id) => Task::perform(apply(id), Message::Applied),
            Message::Applied(Ok(())) => Task::perform(load(), Message::Loaded),
            Message::Applied(Err(e)) => {
                self.error = Some(e);
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
                    SignalKind::ProfileApplied { name, tier } => {
                        format!("Applied '{name}' ({tier})")
                    }
                    SignalKind::NewTopologySeen { summary } => format!("New topology: {summary}"),
                });
                Task::perform(load(), Message::Loaded)
            }
        }
    }

    fn view(&self, scale: FontScale) -> Element<'_, Message> {
        if !self.connected {
            return container(
                column![
                    scaled_text("Displays", 20.0, scale),
                    scaled_text(
                        self.error
                            .clone()
                            .unwrap_or_else(|| "hyprforge-displayd is not running.".to_string()),
                        14.0,
                        scale,
                    ),
                    scaled_text(
                        "Start it with: systemctl --user start hyprforge-displayd",
                        13.0,
                        scale,
                    ),
                    button("Retry").on_press(Message::Refresh),
                ]
                .spacing(spacing::SM),
            )
            .padding(spacing::LG)
            .into();
        }

        let mut content = column![scaled_text("Displays", 20.0, scale)].spacing(spacing::MD);

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

        let status = column![
            scaled_text(
                format!("Current fingerprint: {}", self.current_fingerprint),
                13.0,
                scale,
            ),
        ]
        .spacing(spacing::XS);
        let status = if let Some(event) = &self.last_event {
            status.push(scaled_text(event.clone(), 13.0, scale))
        } else {
            status
        };
        content = content.push(section("Status", scale, status));

        let mut list = column![].spacing(spacing::SM);
        if self.profiles.is_empty() {
            list = list.push(scaled_text(
                "No profiles yet — connect a display configuration and Hyprforge will learn it.",
                14.0,
                scale,
            ));
        }
        for p in &self.profiles {
            let row_el: Element<'_, Message> = match &self.renaming {
                Some((id, draft)) if id == &p.id => row![
                    text_input("Profile name", draft)
                        .on_input(Message::RenameInput)
                        .on_submit(Message::RenameSubmit),
                    button("Save").on_press(Message::RenameSubmit),
                    button("Cancel").on_press(Message::RenameCancelled),
                ]
                .spacing(spacing::SM)
                .into(),
                _ => self.profile_row(p, scale),
            };
            list = list.push(row_el);
        }
        content = content.push(section(
            "Profiles",
            scale,
            scrollable(list).height(Length::Fill),
        ));

        content = content.push(button("Refresh").on_press(Message::Refresh));

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

async fn apply(id: String) -> Result<(), String> {
    let conn = connect().await?;
    let proxy = DisplaydProxy::new(&conn).await.map_err(|e| e.to_string())?;
    proxy.apply_profile(&id).await.map_err(|e| e.to_string())
}

async fn rename(id: String, new_name: String) -> Result<(), String> {
    let conn = connect().await?;
    let proxy = DisplaydProxy::new(&conn).await.map_err(|e| e.to_string())?;
    proxy
        .rename_profile(&id, &new_name)
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
