mod modules;

use hyprforge_core::theme::{app_theme, spacing, surface, FontScale, TEXT_DIM};
use hyprforge_core::widgets::{primary_button, scaled_text, secondary_button};
use hyprforge_core::SettingsModule;
use iced::keyboard::{self, key, Key};
use iced::widget::{column, container, operation, row, text_input, Id, Space};
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

/// Accessibility font-scale steps a user can cycle through with the
/// sidebar's A-/A+ controls (vision pillar #7).
const FONT_SCALE_STEPS: [f32; 5] = [0.85, 1.0, 1.15, 1.3, 1.5];

fn nearest_step_index(scale: f32) -> usize {
    FONT_SCALE_STEPS
        .iter()
        .enumerate()
        .min_by(|(_, a), (_, b)| (*a - scale).abs().partial_cmp(&(*b - scale).abs()).unwrap())
        .map(|(i, _)| i)
        .unwrap_or(1)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Screen {
    Displays,
    WindowRules,
}

impl Screen {
    const ALL: [Screen; 2] = [Screen::Displays, Screen::WindowRules];

    fn title(self) -> &'static str {
        match self {
            Screen::Displays => "Displays",
            Screen::WindowRules => "Window Rules",
        }
    }
}

#[derive(Debug, Clone)]
enum Message {
    Navigate(Screen),
    SearchChanged(String),
    FocusSearch,
    ClearOrCancel,
    RefreshActive,
    IncreaseFontScale,
    DecreaseFontScale,
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
                screen: Screen::Displays,
                displays,
                window_rules,
                search_query: String::new(),
                search_id: Id::unique(),
                font_scale: FontScale::default(),
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
                Screen::Displays => self
                    .displays
                    .update(modules::displays::Message::Refresh)
                    .map(Message::Displays),
                // Window Rules has no external state to refresh — it's the
                // sole writer of its own TOML, so it's always already
                // current.
                Screen::WindowRules => Task::none(),
            },
            Message::IncreaseFontScale => {
                let next = (nearest_step_index(self.font_scale.0) + 1)
                    .min(FONT_SCALE_STEPS.len() - 1);
                self.font_scale = FontScale(FONT_SCALE_STEPS[next]);
                Task::none()
            }
            Message::DecreaseFontScale => {
                let next = nearest_step_index(self.font_scale.0).saturating_sub(1);
                self.font_scale = FontScale(FONT_SCALE_STEPS[next]);
                Task::none()
            }
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

        let query = self.search_query.to_lowercase();
        let mut nav = column![].spacing(spacing::XS);
        let mut any_visible = false;
        for screen in Screen::ALL {
            if !query.is_empty() && !screen.title().to_lowercase().contains(&query) {
                continue;
            }
            any_visible = true;
            let (label, active) = match screen {
                Screen::Displays => (
                    format!("{}  Displays", self.displays.icon()),
                    self.screen == Screen::Displays,
                ),
                Screen::WindowRules => (
                    format!("{}  Window Rules", self.window_rules.icon()),
                    self.screen == Screen::WindowRules,
                ),
            };
            nav = nav.push(sidebar_button(label, screen, active));
        }
        if !any_visible {
            nav = nav.push(scaled_text("No matches", 13.0, scale).color(TEXT_DIM));
        }

        let font_scale_controls = column![
            scaled_text("TEXT SIZE", 11.0, scale).color(TEXT_DIM),
            row![
                secondary_button("A-").on_press(Message::DecreaseFontScale),
                secondary_button("A+").on_press(Message::IncreaseFontScale),
            ]
            .spacing(spacing::SM),
        ]
        .spacing(spacing::SM);

        let sidebar = container(
            column![
                scaled_text("Hyprforge", 20.0, scale),
                text_input("Search…", &self.search_query)
                    .id(self.search_id.clone())
                    .on_input(Message::SearchChanged)
                    .padding(8),
                nav,
                Space::new().height(Length::Fill),
                font_scale_controls,
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
            Screen::Displays => self.displays.view(scale).map(Message::Displays),
            Screen::WindowRules => self.window_rules.view(scale).map(Message::WindowRules),
        };
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
                Key::Character("1") => Some(Message::Navigate(Screen::Displays)),
                Key::Character("2") => Some(Message::Navigate(Screen::WindowRules)),
                Key::Character("f") => Some(Message::FocusSearch),
                Key::Character("r") => Some(Message::RefreshActive),
                _ => None,
            }
        });

        Subscription::batch([self.displays.subscription().map(Message::Displays), shortcuts])
    }
}
