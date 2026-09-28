//! Everything one frame of the auth screen shows, as plain data.
//!
//! [`crate::screen::view`] draws a [`Scene`] and decides nothing. The
//! decisions — is the screen idle, how far has the shake travelled,
//! which power actions are offered, what the hint under the clock says —
//! are the pure functions in this module, so every one of them is a test
//! away rather than a nested compositor away.
//!
//! Split this way because the two hosts know different things. The lock
//! screen has a keyboard layout, a battery, a fingerprint reader and a
//! media player to report; the greeter, running as its own user before
//! anyone has logged in, has none of that. Each field here that a host
//! cannot fill is simply left at its default, and the screen draws a
//! little less rather than drawing something wrong.

use crate::conversation::State;
use chrono::{DateTime, Local};
use hyprforge_look::Theme;
use std::time::{Duration, Instant};

/// How long after the last keystroke an empty prompt goes back to the
/// clock.
///
/// Long enough that someone who paused to remember a password does not
/// have the field vanish under them; short enough that a screen nobody
/// is using settles back to the quiet one.
pub const IDLE_AFTER: Duration = Duration::from_secs(12);

/// How long a rejected password shakes, and keeps its red dots, before
/// the field clears.
///
/// Three swings over 450ms. The dots staying red for the duration is the
/// point: it says *that* attempt, the one you just typed, was the wrong
/// one — clearing them instantly reads as the keystrokes having been
/// lost.
pub const SHAKE: Duration = Duration::from_millis(450);

/// How far the field travels at the start of a shake, in logical pixels.
const SHAKE_AMPLITUDE: f32 = 8.0;

/// How many full swings a shake makes.
const SHAKE_SWINGS: f32 = 3.0;

/// Which monitor this frame is for.
///
/// The mockup's multi-monitor rule: the prompt belongs to exactly one
/// output. Three password fields across three screens is three places a
/// person has to check for which one has the keyboard.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Role {
    /// Has the keyboard: the clock, the card and every extra.
    #[default]
    Primary,
    /// Any other output: the clock and the wallpaper, nothing to act on.
    Secondary,
}

/// Whether the card is up.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Mode {
    /// Clock only, large, with a hint to type. What a lock screen looks
    /// like while nobody is touching it.
    Idle,
    /// The card with the avatar and the prompt.
    #[default]
    Entry,
}

/// What the fingerprint reader is doing, as far as the screen cares.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Fingerprint {
    /// No reader, no enrolled finger, or no fprintd. The screen never
    /// mentions a sensor that is not there.
    #[default]
    Unavailable,
    /// Listening for a finger.
    Ready,
    /// The last touch was not accepted; listening again. fprintd's own
    /// reason, already turned into words — "Not recognised", "Finger not
    /// centred".
    Retry(String),
    /// Too many unrecognised touches; only the password is left. Said
    /// rather than silently hidden, or a person keeps pressing a sensor
    /// that has stopped listening.
    Exhausted,
}

impl Fingerprint {
    /// Whether a touch can unlock right now.
    pub fn listening(&self) -> bool {
        matches!(self, Fingerprint::Ready | Fingerprint::Retry(_))
    }
}

/// The battery, reduced to what a lock screen shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Battery {
    /// 0 to 100.
    pub percent: u8,
    pub charging: bool,
    /// Low enough to warn about. Decided by `hyprforge-power`, which owns
    /// the threshold, rather than by a second copy of it here.
    pub low: bool,
}

/// The top-right status line. Each part is optional because each comes
/// from a different daemon, and one being down must not take the others
/// with it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Status {
    /// The active keyboard layout's short name — `us`, `de`. Shown twice,
    /// deliberately: top right as status, and under the field as the
    /// thing a password is about to be typed in.
    pub layout: Option<String>,
    /// The connected network's name, or `None` when offline or unknown.
    pub network: Option<String>,
    pub battery: Option<Battery>,
}

/// What is playing, for the idle screen's media card.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Media {
    pub title: String,
    pub artist: String,
    /// The player's own name — "Spotify", "Firefox".
    pub player: String,
    pub playing: bool,
    /// How far through, 0.0 to 1.0, when the player says.
    pub progress: Option<f32>,
    pub can_previous: bool,
    pub can_next: bool,
}

/// One application's unread count. The app and the count, never the
/// text: a notification's body is exactly the thing a locked screen is
/// hiding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotificationCount {
    pub app: String,
    pub count: u32,
}

/// Something the power menu can do.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PowerAction {
    Suspend,
    Hibernate,
    Reboot,
    PowerOff,
}

impl PowerAction {
    /// Every action, in the order the menu lists them.
    pub const ALL: [PowerAction; 4] =
        [PowerAction::Suspend, PowerAction::Hibernate, PowerAction::Reboot, PowerAction::PowerOff];

    pub fn label(self) -> &'static str {
        match self {
            PowerAction::Suspend => "Suspend",
            PowerAction::Hibernate => "Hibernate",
            PowerAction::Reboot => "Reboot",
            PowerAction::PowerOff => "Shut down",
        }
    }

    /// The key that chooses it while the menu is open.
    pub fn key(self) -> char {
        match self {
            PowerAction::Suspend => 's',
            PowerAction::Hibernate => 'h',
            PowerAction::Reboot => 'r',
            PowerAction::PowerOff => 'p',
        }
    }

    /// Whether the row is drawn in the danger colour.
    ///
    /// Shut down only, as the mockup has it. Reboot loses open work too,
    /// but it comes back on its own; the red is for the one row after
    /// which the machine is simply off.
    pub fn dangerous(self) -> bool {
        matches!(self, PowerAction::PowerOff)
    }
}

/// The power menu, when it is open.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PowerMenu {
    /// What logind says this machine can do right now, in menu order.
    /// Something it cannot do is left out rather than shown and refused:
    /// hibernate on a machine with no swap is `na`, and offering it is a
    /// button that can only fail.
    pub actions: Vec<PowerAction>,
    /// The highlighted row, an index into `actions`.
    pub selected: usize,
}

/// Something the person asked the screen to do with the pointer.
///
/// The only messages the screen sends. None of them authenticates; the
/// password and the fingerprint reach the conversation by other routes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    TogglePowerMenu,
    Power(PowerAction),
    MediaPrevious,
    MediaPlayPause,
    MediaNext,
}

/// A rejected attempt, as it is being shown.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Rejection {
    /// How many red dots — the length of the attempt that failed, never
    /// its characters, and zero once the shake is over.
    pub dots: usize,
    /// Horizontal offset of the field right now, in logical pixels.
    pub offset: f32,
    /// Which attempt this was, counting from one, for "attempt 2".
    pub attempt: u32,
}

/// One frame of the auth screen.
#[derive(Debug, Clone)]
pub struct Scene<'a> {
    pub state: &'a State,
    pub username: &'a str,
    pub theme: &'a Theme,
    pub now: DateTime<Local>,
    pub caps_lock: bool,
    pub role: Role,
    pub mode: Mode,
    /// The output's logical size, so the clock can be kept to a sensible
    /// share of it. `(0, 0)` when the host does not know, which leaves
    /// the clock at its ideal size.
    pub output: (f32, f32),
    /// Dots to keep showing while the backend checks an answer — the
    /// length of what was submitted, so the field does not empty the
    /// instant Enter is pressed.
    pub submitted: usize,
    pub rejection: Option<Rejection>,
    pub status: &'a Status,
    pub fingerprint: &'a Fingerprint,
    pub media: Option<&'a Media>,
    pub notifications: &'a [NotificationCount],
    /// `None`: this host offers no power button at all (the greeter).
    /// `Some(None)`: a button, menu closed. `Some(Some(menu))`: open.
    pub power: Option<Option<&'a PowerMenu>>,
    /// A picture for the avatar. `None` draws the initial instead.
    pub avatar: Option<&'a std::path::Path>,
    /// The wallpaper already scaled to this output's exact physical size,
    /// cropped to cover it and with the theme's dim baked in. Drawn
    /// one-to-one in place of the theme's wallpaper path — see the lock's
    /// `backdrop` module for why resampling every frame was the cost.
    /// `None` draws from the path, as a host without one always does.
    pub backdrop: Option<&'a iced_runtime::core::image::Handle>,
}

impl<'a> Scene<'a> {
    /// A scene with nothing but the conversation in it — what a host
    /// with no extras to report draws.
    pub fn new(state: &'a State, username: &'a str, theme: &'a Theme, now: DateTime<Local>) -> Scene<'a> {
        static NO_STATUS: Status = Status { layout: None, network: None, battery: None };
        static NO_FINGERPRINT: Fingerprint = Fingerprint::Unavailable;
        Scene {
            state,
            username,
            theme,
            now,
            caps_lock: false,
            role: Role::Primary,
            mode: Mode::Entry,
            output: (0.0, 0.0),
            submitted: 0,
            rejection: None,
            status: &NO_STATUS,
            fingerprint: &NO_FINGERPRINT,
            media: None,
            notifications: &[],
            power: None,
            avatar: None,
            backdrop: None,
        }
    }
}

/// Whether the screen should be showing the clock alone.
///
/// Idle only when there is nothing on screen worth keeping up: nothing
/// typed, no failure or message to read, no answer being checked, and no
/// key for [`IDLE_AFTER`]. Every one of those exceptions is a state where
/// hiding the card would hide something the person needs.
///
/// `since_key` is `None` when no key has been pressed since the lock
/// began, which is the case the lock opens in: idle, like every other
/// lock screen.
pub fn mode(state: &State, typed: usize, since_key: Option<Duration>) -> Mode {
    let quiet = match state {
        // Asking, with nothing typed, is the resting state. `Working`
        // counts as quiet only while nothing was typed, so the few
        // milliseconds before PAM's first prompt do not flash a card.
        State::Asking { .. } | State::Working => typed == 0,
        State::Failed { .. } | State::Telling { .. } | State::Authenticated => false,
    };
    let stale = since_key.is_none_or(|elapsed| elapsed >= IDLE_AFTER);
    if quiet && stale {
        Mode::Idle
    } else {
        Mode::Entry
    }
}

/// The rejection to show, `elapsed` after a failed attempt of
/// `attempted` characters.
///
/// A damped sine: `SHAKE_SWINGS` full swings that die away to nothing
/// at [`SHAKE`], where the red dots go too. After that the field stays
/// red, empty and still, until the person types — a failure that fades
/// out by itself would leave someone who looked away not knowing why the
/// field emptied.
pub fn rejection(elapsed: Duration, attempted: usize, attempt: u32) -> Rejection {
    if elapsed >= SHAKE {
        return Rejection { dots: 0, offset: 0.0, attempt };
    }
    let t = elapsed.as_secs_f32() / SHAKE.as_secs_f32();
    let swing = (t * SHAKE_SWINGS * std::f32::consts::TAU).sin();
    Rejection { dots: attempted, offset: -SHAKE_AMPLITUDE * swing * (1.0 - t), attempt }
}

/// The two moments a host has to remember for the screen to move: the
/// last keystroke, which decides idle, and the start of a failure, which
/// drives the shake.
///
/// One copy for both hosts. The key grammar was written out twice once
/// and the copies drifted in the way that mattered; timing would drift
/// the same way, and a greeter whose field never went back to the clock
/// would be the same bug in a different place.
#[derive(Debug, Clone, Default)]
pub struct Pacing {
    last_key: Option<Instant>,
    failed_at: Option<Instant>,
}

impl Pacing {
    /// A key was pressed — any key, whether or not it typed anything.
    pub fn key(&mut self, now: Instant) {
        self.last_key = Some(now);
    }

    /// Notes what the conversation is doing, once per frame or event.
    /// The shake starts the first time a failure is seen, and a failure
    /// that ends forgets it.
    ///
    /// Returns whether a shake began just now, so a host whose repaints
    /// are timer-driven can switch to frame rate at the moment it needs
    /// to rather than on its next tick.
    pub fn observe(&mut self, state: &State, now: Instant) -> bool {
        match (state, self.failed_at) {
            (State::Failed { .. }, None) => {
                self.failed_at = Some(now);
                true
            }
            (State::Failed { .. }, Some(_)) => false,
            _ => {
                self.failed_at = None;
                false
            }
        }
    }

    /// Whether the screen is idle — see [`mode`].
    pub fn mode(&self, state: &State, typed: usize, now: Instant) -> Mode {
        mode(state, typed, self.last_key.map(|at| now.saturating_duration_since(at)))
    }

    /// The rejection to draw, if a failure is on screen.
    pub fn rejection(&self, state: &State, submitted: usize, failures: u32, now: Instant) -> Option<Rejection> {
        let at = self.failed_at.filter(|_| matches!(state, State::Failed { .. }))?;
        Some(rejection(now.saturating_duration_since(at), submitted, failures))
    }

    /// Whether something is moving and the host should repaint at frame
    /// rate rather than once a second.
    pub fn animating(&self, now: Instant) -> bool {
        self.failed_at.is_some_and(|at| now.saturating_duration_since(at) < SHAKE)
    }
}

/// What the line at the bottom of the idle screen says.
///
/// Mentions the sensor only when it is listening. A hint to touch a
/// reader that is absent, or that has given up after too many misses, is
/// an instruction that cannot work.
pub fn idle_hint(fingerprint: &Fingerprint) -> &'static str {
    if fingerprint.listening() {
        "Type, or touch the sensor, to unlock"
    } else {
        "Type to unlock"
    }
}

/// Whether the card should offer the fingerprint rather than the field.
///
/// Only with nothing typed: the first character means someone has chosen
/// the password, and swapping the field out from under them would be the
/// screen changing its mind mid-word.
pub fn offers_fingerprint(state: &State, typed: usize, fingerprint: &Fingerprint) -> bool {
    matches!(state, State::Asking { .. }) && typed == 0 && fingerprint.listening()
}

/// The power menu's row for a key pressed while it is open, if any.
///
/// Case-insensitive, because Caps Lock being on is the one state a lock
/// screen already knows people are in by accident.
pub fn power_action_for_key(menu: &PowerMenu, text: &str) -> Option<PowerAction> {
    let mut chars = text.chars();
    let (Some(c), None) = (chars.next(), chars.next()) else {
        return None;
    };
    let c = c.to_ascii_lowercase();
    menu.actions.iter().copied().find(|action| action.key() == c)
}

/// The avatar's fallback: the first letter of the name, uppercased.
pub fn initial(username: &str) -> String {
    username
        .chars()
        .find(|c| c.is_alphanumeric())
        .map(|c| c.to_uppercase().collect())
        .unwrap_or_else(|| "?".into())
}

/// A name as a person would write it: `apost` stays `apost`, but the
/// first letter is raised, which is what the mockup's "Adam" is.
pub fn display_name(username: &str) -> String {
    let mut chars = username.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::conversation::Prompt;

    fn asking() -> State {
        State::Asking { prompt: Prompt::secret("Password:"), entered: String::new().into() }
    }

    #[test]
    fn a_lock_opens_on_the_clock_and_a_key_brings_the_card_up() {
        assert_eq!(mode(&asking(), 0, None), Mode::Idle);
        assert_eq!(mode(&asking(), 0, Some(Duration::from_millis(10))), Mode::Entry);
        assert_eq!(mode(&asking(), 0, Some(IDLE_AFTER)), Mode::Idle);
    }

    /// Hiding the card over anything the person still has to read would
    /// hide the reason they cannot get in.
    #[test]
    fn the_card_never_hides_a_failure_a_message_or_something_typed() {
        let long_ago = Some(IDLE_AFTER * 10);
        assert_eq!(mode(&asking(), 3, long_ago), Mode::Entry, "typed");
        assert_eq!(mode(&State::Failed { reason: "no".into() }, 0, long_ago), Mode::Entry);
        assert_eq!(mode(&State::Telling { text: "t".into(), error: false }, 0, long_ago), Mode::Entry);
        assert_eq!(mode(&State::Working, 4, long_ago), Mode::Entry, "being checked");
    }

    #[test]
    fn a_shake_swings_and_dies_away_then_the_red_dots_clear() {
        let start = rejection(Duration::ZERO, 7, 2);
        assert_eq!(start.dots, 7);
        let peak = (0..45)
            .map(|ms| rejection(Duration::from_millis(ms * 10), 7, 2).offset.abs())
            .fold(0.0f32, f32::max);
        assert!(peak > 4.0 && peak <= SHAKE_AMPLITUDE, "{peak}");
        let late = rejection(SHAKE - Duration::from_millis(20), 7, 2);
        assert!(late.offset.abs() < 1.0, "it should have nearly settled: {}", late.offset);
        let over = rejection(SHAKE, 7, 2);
        assert_eq!((over.dots, over.offset, over.attempt), (0, 0.0, 2));
    }

    /// A shake that changed direction fewer than three times reads as a
    /// jolt, not a "no" — the swing count is the design, so pin it.
    #[test]
    fn a_shake_changes_direction_at_least_five_times() {
        let offsets: Vec<f32> =
            (1..450).map(|ms| rejection(Duration::from_millis(ms), 1, 1).offset).collect();
        let reversals = offsets.windows(2).filter(|w| w[0].signum() != w[1].signum()).count();
        assert!(reversals >= 5, "{reversals}");
    }

    /// The shake begins when the failure is first seen — not when it is
    /// drawn, which on an idle lock could be a second later — and a
    /// second failure shakes again rather than inheriting the first's
    /// finished clock.
    #[test]
    fn each_failure_starts_its_own_shake() {
        let failed = State::Failed { reason: "no".into() };
        let t0 = Instant::now();
        let mut pacing = Pacing::default();
        assert!(pacing.observe(&failed, t0), "the first sight of a failure begins a shake");
        assert!(pacing.animating(t0 + Duration::from_millis(100)));
        assert_eq!(pacing.rejection(&failed, 6, 1, t0).map(|r| r.dots), Some(6));
        assert!(!pacing.observe(&failed, t0 + Duration::from_millis(300)));
        assert!(!pacing.animating(t0 + SHAKE), "observing again must not restart it");

        pacing.observe(&asking(), t0 + SHAKE);
        assert_eq!(pacing.rejection(&asking(), 6, 1, t0 + SHAKE), None);
        let t1 = t0 + Duration::from_secs(5);
        pacing.observe(&failed, t1);
        assert!(pacing.animating(t1 + Duration::from_millis(10)), "a new failure shakes again");
    }

    #[test]
    fn a_key_holds_the_card_up_until_it_has_been_quiet_long_enough() {
        let t0 = Instant::now();
        let mut pacing = Pacing::default();
        assert_eq!(pacing.mode(&asking(), 0, t0), Mode::Idle);
        pacing.key(t0);
        assert_eq!(pacing.mode(&asking(), 0, t0 + Duration::from_secs(1)), Mode::Entry);
        assert_eq!(pacing.mode(&asking(), 0, t0 + IDLE_AFTER), Mode::Idle);
    }

    #[test]
    fn the_hint_only_mentions_a_sensor_that_is_listening() {
        assert_eq!(idle_hint(&Fingerprint::Unavailable), "Type to unlock");
        assert_eq!(idle_hint(&Fingerprint::Exhausted), "Type to unlock");
        assert!(idle_hint(&Fingerprint::Ready).contains("sensor"));
        assert!(idle_hint(&Fingerprint::Retry("x".into())).contains("sensor"));
    }

    #[test]
    fn the_fingerprint_gives_way_to_the_first_typed_character() {
        assert!(offers_fingerprint(&asking(), 0, &Fingerprint::Ready));
        assert!(!offers_fingerprint(&asking(), 1, &Fingerprint::Ready));
        assert!(!offers_fingerprint(&asking(), 0, &Fingerprint::Unavailable));
        assert!(!offers_fingerprint(&asking(), 0, &Fingerprint::Exhausted), "it stopped listening");
        assert!(!offers_fingerprint(&State::Failed { reason: "x".into() }, 0, &Fingerprint::Ready));
    }

    #[test]
    fn a_power_key_chooses_only_an_action_the_menu_offers() {
        let menu = PowerMenu {
            actions: vec![PowerAction::Suspend, PowerAction::Reboot, PowerAction::PowerOff],
            selected: 0,
        };
        assert_eq!(power_action_for_key(&menu, "s"), Some(PowerAction::Suspend));
        assert_eq!(power_action_for_key(&menu, "P"), Some(PowerAction::PowerOff), "Caps Lock");
        assert_eq!(power_action_for_key(&menu, "h"), None, "hibernate is not offered");
        assert_eq!(power_action_for_key(&menu, "sr"), None);
        assert_eq!(power_action_for_key(&menu, ""), None);
    }

    #[test]
    fn every_power_action_has_its_own_key() {
        let keys: std::collections::HashSet<char> = PowerAction::ALL.iter().map(|a| a.key()).collect();
        assert_eq!(keys.len(), PowerAction::ALL.len());
    }

    #[test]
    fn the_avatar_falls_back_to_an_initial_and_the_name_is_capitalised() {
        assert_eq!(initial("apost"), "A");
        assert_eq!(initial("_svc"), "S");
        assert_eq!(initial(""), "?");
        assert_eq!(display_name("apost"), "Apost");
        assert_eq!(display_name("émile"), "Émile");
        assert_eq!(display_name(""), "");
    }
}
