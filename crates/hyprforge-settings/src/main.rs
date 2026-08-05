mod modules;

use hyprforge_core::theme::{app_theme, spacing, surface, FontScale, TEXT_DIM};
use hyprforge_core::widgets::{primary_button, scaled_text, secondary_button};
use hyprforge_core::SettingsModule;
use iced::keyboard::{self, key, Key};
use iced::widget::{column, container, operation, row, text_input, Id};
use iced::{window, Background, Element, Length, Size, Subscription, Task, Theme};
use modules::displays::DisplaysModule;
use modules::window_rules::WindowRulesModule;

const SIDEBAR_WIDTH: f32 = 240.0;
const CONTENT_MAX_WIDTH: f32 = 880.0;

fn main() -> iced::Result {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    iced::application(App::new, App::update, App::view)
        .title("Hyprforge Settings")
        .theme(App::theme)
        .subscription(App::subscription)
        .window(window::Settings {
            size: Size::new(1000.0, 700.0),
            min_size: Some(Size::new(760.0, 520.0)),
            position: window::Position::Centered,
            ..window::Settings::default()
        })
        .run()
}

/// Reads the desktop's own accessibility text-scaling-factor once at
/// startup (vision pillar #7: accessibility, but following the *system*
/// setting rather than a bespoke per-app control the user would have to
/// discover and set separately). `FontScale` itself stays fully wired
/// through the shared widget layer — this is the only thing that changed:
/// where the value comes from.
fn read_global_font_scale() -> FontScale {
    std::process::Command::new("gsettings")
        .args(["get", "org.gnome.desktop.interface", "text-scaling-factor"])
        .output()
        .ok()
        .filter(|out| out.status.success())
        .and_then(|out| String::from_utf8(out.stdout).ok())
        .and_then(|s| s.trim().parse::<f32>().ok())
        .map(FontScale)
        .unwrap_or_default()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Screen {
    Monitors,
    WindowRules,
}

impl Screen {
    fn title(self) -> &'static str {
        match self {
            Screen::Monitors => "Monitors",
            Screen::WindowRules => "Window Rules",
        }
    }
}

/// A top-level sidebar grouping. Both current screens live under the same
/// "Displays" category today (they're both about how windows/outputs are
/// arranged) — later categories (Network, Bluetooth, ...) each get their
/// own entry here rather than flattening everything into one nav list.
struct NavCategory {
    label: &'static str,
    screens: &'static [Screen],
}

const NAV: &[NavCategory] = &[NavCategory {
    label: "Displays",
    screens: &[Screen::Monitors, Screen::WindowRules],
}];

#[derive(Debug, Clone)]
enum Message {
    Navigate(Screen),
    SearchChanged(String),
    FocusSearch,
    ClearOrCancel,
    RefreshActive,
    Displays(modules::displays::Message),
    WindowRules(modules::window_rules::Message),
}

struct App {
    screen: Screen,
    displays: DisplaysModule,
    window_rules: WindowRulesModule,
    search_query: String,
    search_id: Id,
    font_scale: FontScale,
}

impl App {
    fn new() -> (Self, Task<Message>) {
        let (displays, displays_task) = DisplaysModule::new();
        let (window_rules, window_rules_task) = WindowRulesModule::new();
        (
            App {
                screen: Screen::Monitors,
                displays,
                window_rules,
                search_query: String::new(),
                search_id: Id::unique(),
                font_scale: read_global_font_scale(),
            },
            Task::batch([
                displays_task.map(Message::Displays),
                window_rules_task.map(Message::WindowRules),
            ]),
        )
    }

    fn theme(&self) -> Theme {
        app_theme()
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
                        self.window_rules
                            .update(modules::window_rules::Message::DraftCancel)
                            .map(Message::WindowRules),
                        self.window_rules
                            .update(modules::window_rules::Message::CancelSetup)
                            .map(Message::WindowRules),
                    ])
                }
            }
            Message::RefreshActive => match self.screen {
                Screen::Monitors => self
                    .displays
                    .update(modules::displays::Message::Refresh)
                    .map(Message::Displays),
                // Window Rules has no external state to refresh — it's the
                // sole writer of its own TOML, so it's always already
                // current.
                Screen::WindowRules => Task::none(),
            },
            Message::Displays(msg) => self.displays.update(msg).map(Message::Displays),
            Message::WindowRules(msg) => self.window_rules.update(msg).map(Message::WindowRules),
        }
    }

    fn view(&self) -> Element<'_, Message> {
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
                scaled_text(category.label.to_uppercase(), 11.0, scale).color(TEXT_DIM),
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
            nav = nav.push(scaled_text("No matches", 13.0, scale).color(TEXT_DIM));
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
            background: Some(Background::Color(surface::SIDEBAR)),
            ..container::Style::default()
        });

        let content: Element<'_, Message> = match self.screen {
            Screen::Monitors => self.displays.view(scale).map(Message::Displays),
            Screen::WindowRules => self.window_rules.view(scale).map(Message::WindowRules),
        };
        // The Monitors editor (canvas + full property panel + policy/swap
        // sections) routinely exceeds window height — without scrolling,
        // everything past the window edge was just clipped and silently
        // invisible, not merely off-screen.
        let content = iced::widget::scrollable(content)
            .width(Length::Fill)
            .height(Length::Fill);
        let content = container(content)
            .max_width(CONTENT_MAX_WIDTH)
            .width(Length::Fill);
        let content = container(content)
            .width(Length::Fill)
            .height(Length::Fill)
            .center_x(Length::Fill);

        container(row![sidebar, content])
            .style(|_theme: &Theme| container::Style {
                background: Some(Background::Color(surface::ROOT)),
                ..container::Style::default()
            })
            .into()
    }

    /// The one keyboard grammar used across the whole app (vision pillar
    /// #9 — pick once, document it, never diverge per-module):
    ///
    /// - `Ctrl+1` / `Ctrl+2` — switch to Displays / Window Rules
    /// - `Ctrl+F` — focus the sidebar search box
    /// - `Ctrl+R` — refresh the active module
    /// - `Escape` — clear the search box if it has text, else cancel
    ///   whatever draft/dialog is open in the active module
    fn subscription(&self) -> Subscription<Message> {
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
                Key::Character("f") => Some(Message::FocusSearch),
                Key::Character("r") => Some(Message::RefreshActive),
                _ => None,
            }
        });

        Subscription::batch([self.displays.subscription().map(Message::Displays), shortcuts])
    }
}
