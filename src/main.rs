mod look;
mod module;
mod modules;

use hyprforge_ui::theme::{app_theme, spacing, surface, FontScale, text_dim};
use hyprforge_ui::widgets::{primary_button, scaled_text, secondary_button};
use crate::module::SettingsModule;
use iced::keyboard::{self, key, Key};
use iced::widget::{column, container, operation, row, text_input, Id};
use iced::{window, Background, Element, Length, Size, Subscription, Task, Theme};
use modules::appearance::AppearanceModule;
use modules::desktop::DesktopModule;
use modules::session::SessionModule;
use modules::system::SystemModule;
use modules::displays::DisplaysModule;
use modules::input::InputModule;
use modules::shortcuts::ShortcutsModule;
use modules::window_rules::WindowRulesModule;

const SIDEBAR_WIDTH: f32 = 240.0;
const CONTENT_MAX_WIDTH: f32 = 880.0;

fn main() -> iced::Result {
    // `from_default_env()` alone defaults to ERROR, and these crates emit
    // no `error!` at all — so with RUST_LOG unset, which is how a GUI
    // launched from a menu always runs, every `warn!` in the app went
    // nowhere. That included the one saying the greeter's theme could
    // not be exported, which was itself the fix for a bug whose whole
    // symptom was silence. `hyprforge-displayd` already got this right;
    // this makes the GUI agree. RUST_LOG still overrides.
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
        )
        .init();

    // Resolve the shared look before the first frame. This is the one
    // place that joins the two halves: hyprforge-appearance knows what
    // the user set, hyprforge-ui knows how to draw it, and neither
    // depends on the other — the app they are both part of does the
    // introduction.
    hyprforge_ui::theme::init(hyprforge_appearance::look::resolve());

    // `daemon` rather than `application` because the revert countdown needs
    // its own window: a change that blanks a screen may well leave the
    // settings window itself invisible, so the prompt to undo it can't live
    // inside that window.
    iced::daemon(App::new, App::update, App::view)
        .title(App::title)
        .theme(App::theme)
        .subscription(App::subscription)
        .run()
}

/// Settings for the main window, opened on boot.
fn main_window_settings() -> window::Settings {
    window::Settings {
        size: Size::new(1000.0, 700.0),
        min_size: Some(Size::new(760.0, 520.0)),
        position: window::Position::Centered,
        ..window::Settings::default()
    }
}

/// The revert prompt's window title.
///
/// Also the handle Hyprland's dispatchers use to find it (see
/// [`pin_revert_popup`]), so it must stay stable, unique, and free of regex
/// metacharacters. The window is undecorated, so this is an identifier
/// rather than a visible caption.
const REVERT_POPUP_TITLE: &str = "Hyprforge Keep Display Settings";

fn revert_popup_settings() -> window::Settings {
    window::Settings {
        size: Size::new(420.0, 190.0),
        position: window::Position::Centered,
        resizable: false,
        decorations: false,
        // Honoured on X11/Windows/macOS. Wayland has no always-on-top
        // protocol, so this is a no-op there and `pin_revert_popup` does
        // the real work — it's set anyway so the intent survives a port.
        level: window::Level::AlwaysOnTop,
        ..window::Settings::default()
    }
}

/// Floats and pins the popup via Hyprland's IPC.
///
/// A Wayland client cannot ask to be kept above other windows — there's no
/// protocol for it — so `window::Level::AlwaysOnTop` does nothing here.
/// Hyprland can do it on our behalf: `float` lifts it out of the tiling
/// layout, and `pin` keeps it visible on every workspace. Without this the
/// prompt can end up tiled behind, or on a workspace the user isn't looking
/// at, which defeats the entire point of a countdown you must answer.
///
/// Best-effort by design: if the dispatch fails the prompt is still a real
/// window the user can reach, and the daemon reverts on its own regardless.
async fn pin_revert_popup() {
    // Give the compositor a moment to map the window before addressing it.
    tokio::time::sleep(std::time::Duration::from_millis(250)).await;

    // Lua long-bracket strings pass their contents through verbatim, with no
    // escape processing, so a selector can never break the dispatch's own
    // parse — the same reason the window-rules codegen quotes regexes this
    // way. (A `\?` inside a normal Lua "..." literal is a hard parse error.)
    let selector = format!("[[title:^{REVERT_POPUP_TITLE}$]]");
    for dispatcher in ["float", "pin"] {
        let call =
            format!("hl.dsp.window.{dispatcher}({{ action = \"set\", window = {selector} }})");
        let dispatched = tokio::time::timeout(
            hyprforge_core::command::TIMEOUT,
            tokio::process::Command::new("hyprctl").arg("dispatch").arg(&call).output(),
        )
        .await;
        match dispatched.unwrap_or_else(|_| {
            Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "hyprctl did not answer",
            ))
        }) {
            Ok(out) => {
                // hyprctl exits 0 even when the Lua call itself errored, so
                // the response body is what actually reports success.
                let body = String::from_utf8_lossy(&out.stdout);
                if !body.trim().eq_ignore_ascii_case("ok") {
                    tracing::warn!(
                        dispatcher,
                        response = %body.trim(),
                        "hyprctl rejected the dispatch; revert prompt may not stay on top"
                    );
                }
            }
            Err(e) => {
                // Not running Hyprland, or hyprctl isn't installed. The
                // prompt is still a real window the user can reach, and the
                // daemon reverts on its own either way.
                tracing::debug!(error = %e, "could not run hyprctl to pin the revert prompt");
                return;
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Screen {
    Monitors,
    WindowRules,
    Shortcuts,
    Input,
    Appearance,
    Desktop,
    Session,
    System,
}

impl Screen {
    fn title(self) -> &'static str {
        match self {
            Screen::Monitors => "Monitors",
            Screen::WindowRules => "Window Rules",
            Screen::Shortcuts => "Shortcuts",
            Screen::Input => "Input",
            Screen::Appearance => "Appearance",
            Screen::Desktop => "Desktop",
            Screen::Session => "Session",
            Screen::System => "System",
        }
    }
}

/// A top-level sidebar grouping. Monitors and Window Rules share the
/// "Displays" category (both are about how windows/outputs are arranged);
/// Shortcuts gets its own category since keybindings are a different kind
/// of setting entirely — later categories (Network, Bluetooth, ...) each
/// get their own entry here too, rather than flattening everything into
/// one nav list.
struct NavCategory {
    label: &'static str,
    screens: &'static [Screen],
}

const NAV: &[NavCategory] = &[
    NavCategory {
        label: "Displays",
        screens: &[Screen::Monitors, Screen::WindowRules],
    },
    NavCategory {
        label: "Shortcuts",
        screens: &[Screen::Shortcuts],
    },
    NavCategory {
        label: "Input",
        screens: &[Screen::Input],
    },
    NavCategory {
        label: "Appearance",
        screens: &[Screen::Appearance, Screen::Desktop],
    },
    NavCategory {
        label: "Session",
        screens: &[Screen::Session, Screen::System],
    },
];

#[derive(Debug, Clone)]
enum Message {
    Navigate(Screen),
    SearchChanged(String),
    FocusSearch,
    ClearOrCancel,
    RefreshActive,
    Displays(modules::displays::Message),
    WindowRules(modules::window_rules::Message),
    Shortcuts(modules::shortcuts::Message),
    Input(modules::input::Message),
    Appearance(modules::appearance::Message),
    Desktop(modules::desktop::Message),
    Session(modules::session::Message),
    System(modules::catalog_screen::Message),
    WindowOpened(window::Id),
    WindowClosed(window::Id),
    RevertPopupOpened(window::Id),
    Noop,
}

struct App {
    screen: Screen,
    displays: DisplaysModule,
    window_rules: WindowRulesModule,
    shortcuts: ShortcutsModule,
    input: InputModule,
    appearance: AppearanceModule,
    desktop: DesktopModule,
    session: SessionModule,
    system: SystemModule,
    search_query: String,
    search_id: Id,
    font_scale: FontScale,
    main_window: Option<window::Id>,
    /// The pinned countdown prompt, while a display change is provisional.
    revert_popup: Option<window::Id>,
}

impl App {
    fn new() -> (Self, Task<Message>) {
        let (displays, displays_task) = DisplaysModule::new();
        let (window_rules, window_rules_task) = WindowRulesModule::new();
        let (shortcuts, shortcuts_task) = ShortcutsModule::new();
        let (input, input_task) = InputModule::new();
        let (appearance, appearance_task) = AppearanceModule::new();
        let (desktop, desktop_task) = DesktopModule::new();
        let (session, session_task) = SessionModule::new();
        let (system, system_task) = SystemModule::new();
        (
            App {
                screen: Screen::Monitors,
                displays,
                window_rules,
                shortcuts,
                input,
                appearance,
                desktop,
                session,
                system,
                search_query: String::new(),
                search_id: Id::unique(),
                font_scale: FontScale(hyprforge_ui::theme::active().font_scale),
                main_window: None,
                revert_popup: None,
            },
            Task::batch([
                window::open(main_window_settings()).1.map(Message::WindowOpened),
                displays_task.map(Message::Displays),
                window_rules_task.map(Message::WindowRules),
                shortcuts_task.map(Message::Shortcuts),
                input_task.map(Message::Input),
                appearance_task.map(Message::Appearance),
                desktop_task.map(Message::Desktop),
                session_task.map(Message::Session),
                system_task.map(Message::System),
            ]),
        )
    }

    fn theme(&self, _window: window::Id) -> Theme {
        app_theme()
    }

    fn title(&self, window: window::Id) -> String {
        if Some(window) == self.revert_popup {
            REVERT_POPUP_TITLE.to_string()
        } else {
            "Hyprforge Settings".to_string()
        }
    }

    /// Opens or closes the countdown prompt to match the module's state.
    ///
    /// Driven from the module rather than duplicated: `revert_seconds_left`
    /// is set by the daemon's `RevertPending` signal and cleared by
    /// `RevertResolved`, so the window's lifetime tracks the daemon's own
    /// notion of "a change is provisional" instead of a second timer that
    /// could drift out of sync with it.
    fn sync_revert_popup(&mut self) -> Task<Message> {
        match (self.displays.revert_seconds_left(), self.revert_popup) {
            (Some(_), None) => {
                // Record the id now, not on `RevertPopupOpened`. `window::open`
                // hands it back before the window exists, and this runs on
                // every message from the module — including the `RevertTick`
                // that arrives every second while a countdown is live. Waiting
                // for the open to round-trip leaves those ticks looking at a
                // popup that is still `None`, so each one opens another window,
                // and every window whose id isn't the one recorded here falls
                // through `view` to the full settings UI, drawn at the prompt's
                // 420x190.
                let (id, open) = window::open(revert_popup_settings());
                self.revert_popup = Some(id);
                open.map(Message::RevertPopupOpened)
            }
            (None, Some(id)) => {
                self.revert_popup = None;
                window::close(id)
            }
            _ => Task::none(),
        }
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Navigate(screen) => {
                self.screen = screen;
                Task::none()
            }
            Message::SearchChanged(query) => {
                self.search_query = query;
                Task::none()
            }
            Message::FocusSearch => operation::focus(self.search_id.clone()),
            Message::ClearOrCancel => {
                if !self.search_query.is_empty() {
                    self.search_query.clear();
                    Task::none()
                } else {
                    // Harmless no-ops when nothing's open: RenameCancelled
                    // clears a rename that may not be in progress,
                    // DraftCancel/CancelSetup close forms/dialogs that may
                    // already be closed. Cheaper and more robust than
                    // tracking "is anything open" at the App level.
                    Task::batch([
                        self.displays
                            .update(modules::displays::Message::RenameCancelled)
                            .map(Message::Displays),
                        self.displays
                            .update(modules::displays::Message::DeleteCancel)
                            .map(Message::Displays),
                        self.displays
                            .update(modules::displays::Message::ImportClose)
                            .map(Message::Displays),
                        self.window_rules
                            .update(modules::window_rules::Message::DraftCancel)
                            .map(Message::WindowRules),
                        self.window_rules
                            .update(modules::window_rules::Message::ImportCancel)
                            .map(Message::WindowRules),
                        self.shortcuts
                            .update(modules::shortcuts::Message::DraftCancel)
                            .map(Message::Shortcuts),
                        self.shortcuts
                            .update(modules::shortcuts::Message::ImportCancel)
                            .map(Message::Shortcuts),
                    ])
                }
            }
            Message::RefreshActive => match self.screen {
                Screen::Monitors => self
                    .displays
                    .update(modules::displays::Message::Refresh)
                    .map(Message::Displays),
                // Window Rules and Shortcuts have no external state to
                // refresh — each is the sole writer of its own TOML, so
                // it's always already current.
                Screen::WindowRules => Task::none(),
                Screen::Shortcuts => Task::none(),
                Screen::Input => Task::none(),
                Screen::Appearance => Task::none(),
                Screen::Desktop => Task::none(),
                Screen::Session => Task::none(),
                Screen::System => Task::none(),
            },
            Message::Displays(msg) => {
                let task = self.displays.update(msg).map(Message::Displays);
                Task::batch([task, self.sync_revert_popup()])
            }
            Message::Input(msg) => self.input.update(msg).map(Message::Input),
            Message::Appearance(msg) => self.appearance.update(msg).map(Message::Appearance),
            Message::Desktop(msg) => self.desktop.update(msg).map(Message::Desktop),
            Message::Session(msg) => self.session.update(msg).map(Message::Session),
            Message::System(msg) => self.system.update(msg).map(Message::System),
            Message::WindowOpened(id) => {
                if self.main_window.is_none() {
                    self.main_window = Some(id);
                }
                Task::none()
            }
            Message::RevertPopupOpened(id) => {
                self.revert_popup = Some(id);
                // Ask Hyprland to float + pin it; see `pin_revert_popup`.
                Task::perform(pin_revert_popup(), |()| Message::Noop)
            }
            Message::WindowClosed(id) => {
                if Some(id) == self.revert_popup {
                    self.revert_popup = None;
                    // Closing the prompt is not an answer. The daemon keeps
                    // its own countdown and still reverts, so the change
                    // can't be silently kept by dismissing the window.
                    return Task::none();
                }
                if Some(id) == self.main_window {
                    return iced::exit();
                }
                Task::none()
            }
            Message::Noop => Task::none(),
            Message::WindowRules(msg) => self.window_rules.update(msg).map(Message::WindowRules),
            Message::Shortcuts(msg) => self.shortcuts.update(msg).map(Message::Shortcuts),
        }
    }

    fn view(&self, window: window::Id) -> Element<'_, Message> {
        if Some(window) == self.revert_popup {
            return self.revert_popup_view();
        }
        self.settings_view()
    }

    /// The countdown prompt. Deliberately tiny and self-contained: it may
    /// be the only thing legible on a screen that a bad mode change just
    /// scrambled, so it states what happened, how long is left, and the two
    /// ways out — nothing else.
    fn revert_popup_view(&self) -> Element<'_, Message> {
        let scale = self.font_scale;
        let left = self.displays.revert_seconds_left().unwrap_or(0);
        container(
            column![
                scaled_text("Keep these display settings?", 17.0, scale),
                scaled_text(
                    format!("Reverting in {left}s if you don't choose."),
                    13.0,
                    scale,
                )
                .color(text_dim()),
                row![
                    secondary_button("Revert now")
                        .on_press(Message::Displays(modules::displays::Message::RevertLayoutNow)),
                    primary_button("Keep changes")
                        .on_press(Message::Displays(modules::displays::Message::KeepLayout)),
                ]
                .spacing(spacing::SM),
            ]
            .spacing(spacing::MD)
            .padding(spacing::LG),
        )
        .width(Length::Fill)
        .height(Length::Fill)
        .center(Length::Fill)
        .style(|_theme: &Theme| container::Style {
            background: Some(Background::Color(surface::card())),
            border: iced::Border {
                radius: 10.0.into(),
                width: 1.0,
                color: surface::card_border(),
            },
            ..container::Style::default()
        })
        .into()
    }

    fn settings_view(&self) -> Element<'_, Message> {
        let scale = self.font_scale;

        let sidebar_button = |label: String, screen: Screen, active: bool| {
            let btn = if active {
                primary_button(label)
            } else {
                secondary_button(label)
            };
            btn.width(Length::Fill)
                .padding([10, 12])
                .on_press(Message::Navigate(screen))
        };

        let icon_for = |screen: Screen| match screen {
            Screen::Monitors => self.displays.icon(),
            Screen::WindowRules => self.window_rules.icon(),
            Screen::Shortcuts => self.shortcuts.icon(),
            Screen::Input => self.input.icon(),
            Screen::Appearance => self.appearance.icon(),
            Screen::Desktop => self.desktop.icon(),
            Screen::Session => self.session.icon(),
            Screen::System => self.system.icon(),
        };

        let query = self.search_query.to_lowercase();
        let mut nav = column![].spacing(spacing::MD);
        let mut any_visible = false;
        for category in NAV {
            let visible_screens: Vec<Screen> = category
                .screens
                .iter()
                .copied()
                .filter(|s| query.is_empty() || s.title().to_lowercase().contains(&query))
                .collect();
            if visible_screens.is_empty() {
                continue;
            }
            any_visible = true;

            let mut sub_items = column![].spacing(spacing::XS);
            for screen in visible_screens {
                let label = format!("{}  {}", icon_for(screen), screen.title());
                sub_items = sub_items.push(sidebar_button(label, screen, self.screen == screen));
            }

            // The category header is itself a button to its first/default
            // sub-item, not just a static label — "Displays" takes you to
            // Monitors the same way clicking "Monitors" does.
            let header = iced::widget::button(
                scaled_text(category.label.to_uppercase(), 11.0, scale).color(text_dim()),
            )
            .style(|_theme: &Theme, _status| iced::widget::button::Style::default())
            .padding(0)
            .on_press(Message::Navigate(category.screens[0]));

            nav = nav.push(
                column![
                    header,
                    container(sub_items).padding(iced::Padding {
                        left: 4.0,
                        ..iced::Padding::default()
                    }),
                ]
                .spacing(spacing::XS),
            );
        }
        if !any_visible {
            nav = nav.push(scaled_text("No matches", 13.0, scale).color(text_dim()));
        }

        let sidebar = container(
            column![
                scaled_text("Hyprforge", 20.0, scale),
                text_input("Search…", &self.search_query)
                    .id(self.search_id.clone())
                    .on_input(Message::SearchChanged)
                    .padding(8),
                nav,
            ]
            .spacing(spacing::MD)
            .padding(spacing::MD)
            .width(Length::Fixed(SIDEBAR_WIDTH)),
        )
        .height(Length::Fill)
        .style(|_theme: &Theme| container::Style {
            background: Some(Background::Color(surface::sidebar())),
            ..container::Style::default()
        });

        let content: Element<'_, Message> = match self.screen {
            Screen::Monitors => self.displays.view(scale).map(Message::Displays),
            Screen::WindowRules => self.window_rules.view(scale).map(Message::WindowRules),
            Screen::Shortcuts => self.shortcuts.view(scale).map(Message::Shortcuts),
            Screen::Input => self.input.view(scale).map(Message::Input),
            Screen::Appearance => self.appearance.view(scale).map(Message::Appearance),
            Screen::Desktop => self.desktop.view(scale).map(Message::Desktop),
            Screen::Session => self.session.view(scale).map(Message::Session),
            Screen::System => self.system.view(scale).map(Message::System),
        };
        // The Monitors editor (canvas + full property panel + policy/swap
        // sections) routinely exceeds window height — without scrolling,
        // everything past the window edge was just clipped and silently
        // invisible, not merely off-screen.
        //
        // Order matters here: the scrollable has to be the *outermost* of
        // these three, so its scrollbar tracks the edge of the content pane.
        // Nesting it inside the max-width/centering containers instead pins
        // the bar to the right edge of the centered column, which on a wide
        // window reads as a scrollbar floating in the middle of the screen.
        let content = container(content)
            .max_width(CONTENT_MAX_WIDTH)
            .width(Length::Fill);
        // Centred in the pane, GNOME-style. Left-aligning a capped column in
        // a very wide window dumps all the slack on one side, which reads as
        // a broken layout; splitting it evenly reads as deliberate margin.
        let content = container(content).width(Length::Fill).center_x(Length::Fill);
        let content = iced::widget::scrollable(content)
            .width(Length::Fill)
            .height(Length::Fill);

        container(row![sidebar, content])
            .style(|_theme: &Theme| container::Style {
                background: Some(Background::Color(surface::root())),
                ..container::Style::default()
            })
            .into()
    }

    /// The one keyboard grammar used across the whole app (vision pillar
    /// #9 — pick once, document it, never diverge per-module):
    ///
    /// - `Ctrl+1` / `Ctrl+2` / `Ctrl+3` — switch to Monitors / Window Rules /
    ///   Shortcuts
    /// - `Ctrl+F` — focus the sidebar search box
    /// - `Ctrl+R` — refresh the active module
    /// - `Escape` — clear the search box if it has text, else cancel
    ///   whatever draft/dialog is open in the active module
    fn subscription(&self) -> Subscription<Message> {
        // While the Shortcuts module is recording a chord, every key on the
        // keyboard belongs to it — including Ctrl+F and Escape. Binding
        // Ctrl+F would otherwise navigate to the search box instead of being
        // recorded, and there'd be no way to bind it at all.
        if self.shortcuts.is_capturing() {
            return Subscription::batch([
                self.displays.subscription().map(Message::Displays),
                self.shortcuts.subscription().map(Message::Shortcuts),
                window::close_events().map(Message::WindowClosed),
            ]);
        }

        let shortcuts = keyboard::listen().filter_map(|event| {
            let keyboard::Event::KeyPressed { key, modifiers, .. } = event else {
                return None;
            };
            if !modifiers.control() {
                return match key {
                    Key::Named(key::Named::Escape) => Some(Message::ClearOrCancel),
                    _ => None,
                };
            }
            match key.as_ref() {
                Key::Character("1") => Some(Message::Navigate(Screen::Monitors)),
                Key::Character("2") => Some(Message::Navigate(Screen::WindowRules)),
                Key::Character("3") => Some(Message::Navigate(Screen::Shortcuts)),
                Key::Character("f") => Some(Message::FocusSearch),
                Key::Character("r") => Some(Message::RefreshActive),
                _ => None,
            }
        });

        Subscription::batch([
            self.displays.subscription().map(Message::Displays),
            self.shortcuts.subscription().map(Message::Shortcuts),
            shortcuts,
            window::close_events().map(Message::WindowClosed),
        ])
    }
}
