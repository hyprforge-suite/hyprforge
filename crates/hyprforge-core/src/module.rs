use crate::theme::FontScale;
use iced::{Element, Subscription, Task};

/// Shared shape for anything hosted inside the Settings app shell, modeled
/// directly on iced's own `update`/`view` split so a module feels like a
/// miniature iced application rather than a bespoke plugin API.
pub trait SettingsModule {
    type Message: std::fmt::Debug + Send + Clone + 'static;

    fn title(&self) -> &str;
    fn icon(&self) -> &'static str;

    fn update(&mut self, message: Self::Message) -> Task<Self::Message>;
    /// `scale` is the app-wide accessibility font scale (vision pillar #7)
    /// — every module receives it rather than reading it from ambient
    /// state, so it's impossible to build a screen that forgets to honor it.
    fn view(&self, scale: FontScale) -> Element<'_, Self::Message>;

    fn subscription(&self) -> Subscription<Self::Message> {
        Subscription::none()
    }
}
