//! The greeter: the same screen as the lock, in front of a login.
//!
//! It draws nothing of its own. `hyprforge_authui::screen` is the screen
//! and `hyprforge_authui::conversation` is the exchange; this crate
//! supplies a window and greetd. That is the whole reason a greeter and
//! a lock screen can look like one system — not because they were styled
//! to match, but because there is one of them.
//!
//! **It cannot read your home directory.** A greeter runs as its own
//! user and `$HOME` is `drwx------`, so the theme comes from the copy the
//! Settings app exports to `/var/lib/hyprforge/greet`. Nothing here
//! reaches into a user's files.

mod greetd;

use clap::Parser;
use greetd::GreetdBackend;
use hyprforge_authui::conversation::{Conversation, State};
use hyprforge_authui::Theme;
use iced::keyboard::{key::Named, Key};
use iced::{Element, Subscription, Task};

#[derive(Parser)]
#[command(about = "Log in")]
struct Args {
    /// Who to log in.
    ///
    /// A single account for now: this screen shows a username but has no
    /// field for choosing one, so offering a picker would be a promise
    /// the UI does not keep. Multi-user selection is its own piece of
    /// work.
    #[arg(long)]
    user: String,

    /// What to run once the login succeeds, split on spaces.
    ///
    /// greetd starts it when this process exits, so it is scheduled and
    /// then this window goes away.
    #[arg(long, default_value = "Hyprland")]
    command: String,

    /// Where the exported theme lives. The default is the directory the
    /// Settings app writes to.
    #[arg(long)]
    theme_dir: Option<std::path::PathBuf>,

    /// Type this in once a question is being asked, then submit.
    ///
    /// The only way to prove the whole greetd handshake — including the
    /// `start_session` at the end — without a keyboard. Compiled out of
    /// release builds: a flag that submits a password from argv puts it
    /// in `ps` output, which is not something a login screen should
    /// offer however convenient it is for testing.
    #[cfg(debug_assertions)]
    #[arg(long, value_name = "TEXT")]
    type_in: Option<String>,
}

#[derive(Debug, Clone)]
enum Message {
    /// A key, and the text it actually produced.
    ///
    /// Both, because they answer different questions. `Key` says which
    /// key it was — Enter, Backspace — while the text is what typing it
    /// means with the modifiers applied. Using the key alone loses the
    /// shift: `SHIFT + j` reports `j`, so every capital and symbol in a
    /// password is silently wrong, and PAM rejects a password the user
    /// typed correctly.
    Key(Key, Option<String>),
    /// Both the clock and the authenticator are driven from here: one
    /// needs the time, the other needs somebody to collect its answers.
    Tick,
}

struct Greeter {
    conversation: Conversation<GreetdBackend>,
    /// Whether a frame was ever drawn.
    ///
    /// A greeter that exits without having shown anything has not
    /// "finished"; it has failed, and saying so is the difference
    /// between a one-line diagnosis and an afternoon. This one exited
    /// with status 0 and printed nothing when it lost a startup race
    /// with the compositor, so greetd reported `conversation failed`
    /// for a password nobody had been asked for.
    drew: std::rc::Rc<std::cell::Cell<bool>>,
    #[cfg(debug_assertions)]
    type_in: Option<String>,
    theme: Theme,
    command: Vec<String>,
    /// Set once the session has been scheduled, so it is asked for
    /// exactly once — greetd refuses a second `StartSession`, and the
    /// window is on its way out anyway.
    launched: bool,
}

impl Greeter {
    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Key(key, text) => self.key(key, text),
            // Collect anything greetd has said. Polling rather than
            // waking on the socket because the clock needs a tick
            // regardless, so there is already something arriving
            // regularly to pump on.
            Message::Tick => {
                self.conversation.pump();
            }
        }

        // Only once there is actually a question to answer. Typing at a
        // conversation that is still working would be correctly ignored,
        // which looks exactly like the test not working.
        #[cfg(debug_assertions)]
        if self.conversation.state().accepts_input() {
            if let Some(text) = self.type_in.take() {
                eprintln!("self test: answering with {} character(s)", text.chars().count());
                self.conversation.type_into(text);
                self.conversation.submit();
            }
        }

        if self.conversation.state().is_authenticated() && !self.launched {
            self.launched = true;
            // greetd runs this when the greeter exits, so scheduling it
            // and closing the window are one action.
            self.conversation.backend().launch(self.command.clone(), Vec::new());
            return iced::exit();
        }
        Task::none()
    }

    fn key(&mut self, key: Key, text: Option<String>) {
        apply_key(&mut self.conversation, key, text);
    }

    fn view(&self) -> Element<'_, Message, iced_widget::Theme, iced::Renderer> {
        self.drew.set(true);
        // Caps Lock is passed as false because iced's modifier state does
        // not carry it. The lock screen shows it, and this should too —
        // it is the difference between a stuck key and an apparently
        // forgotten password. Noted rather than quietly dropped.
        hyprforge_authui::screen::view(
            self.conversation.state(),
            self.conversation.username(),
            &self.theme,
            chrono::Local::now(),
            false,
        )
    }

    fn subscription(&self) -> Subscription<Message> {
        Subscription::batch([
            iced::keyboard::listen().filter_map(|event| match event {
                iced::keyboard::Event::KeyPressed { key, text, .. } => {
                    Some(Message::Key(key, text.map(|t| t.to_string())))
                }
                _ => None,
            }),
            // Fast while the authenticator is working so the screen stays
            // visibly alive through PAM's deliberate pause; slow
            // otherwise, which is all the clock needs.
            iced::time::every(if matches!(self.conversation.state(), State::Working) {
                std::time::Duration::from_millis(100)
            } else {
                std::time::Duration::from_secs(1)
            })
            .map(|_| Message::Tick),
        ])
    }
}

/// One key, in the same terms the lock screen uses.
///
/// Free-standing and generic over the backend so it can be tested
/// without a greetd socket. What reaches a password is the part worth
/// testing: getting it wrong rejects a correctly typed password and
/// spends a faillock attempt doing it.
fn apply_key<B: hyprforge_authui::conversation::Backend>(
    conversation: &mut Conversation<B>,
    key: Key,
    text: Option<String>,
) {
        match key {
            Key::Named(Named::Enter) => match conversation.state() {
                State::Telling { .. } => conversation.acknowledge(),
                State::Failed { .. } => conversation.retry(),
                _ => conversation.submit(),
            },
            Key::Named(Named::Escape) => conversation.clear(),
            Key::Named(Named::Backspace) => {
                let mut entered = conversation.typed().to_string();
                entered.pop();
                conversation.type_into(entered);
            }
            _ => {
                let Some(text) = text else {
                    return;
                };
                // Any key at all leaves the failed state, so someone can
                // simply start typing again rather than work out which
                // key dismisses the error.
                if matches!(conversation.state(), State::Failed { .. }) {
                    conversation.retry();
                }
                // Control characters would otherwise count as typed
                // characters and show a dot for nothing — Enter and
                // Backspace both produce text as well as being keys.
                if !text.is_empty() && !text.chars().any(char::is_control) {
                    let mut entered = conversation.typed().to_string();
                    entered.push_str(&text);
                    conversation.type_into(entered);
                }
            }
        }
    }


fn main() -> iced::Result {
    // Defaulting to ERROR would silence every `warn!` here, and a greeter
    // is started by greetd with no RUST_LOG and no terminal — the journal
    // is the only place anyone can see what it did. See the same note in
    // hyprforge-settings.
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
        )
        .init();
    let args = Args::parse();

    let backend = match GreetdBackend::connect() {
        Ok(backend) => backend,
        Err(e) => {
            // Not a panic: this is the program standing between a person
            // and their machine, and a stack trace on a login screen
            // helps nobody.
            eprintln!("can't reach the login service: {e}");
            std::process::exit(1);
        }
    };

    let dir = args
        .theme_dir
        .unwrap_or_else(hyprforge_look::theme::export_dir);
    // `load_exported_from` is the one that degrades a missing or
    // unparseable export to the default theme rather than an error — see
    // its doc for why. `--theme-dir` is why this calls that with a
    // variable directory instead of the fixed-path `load_exported()`.
    let theme = hyprforge_authui::screen::renderable(Theme::load_exported_from(&dir));

    let command: Vec<String> = args.command.split_whitespace().map(str::to_string).collect();
    let font = hyprforge_authui::screen::font(&theme);

    // `boot` is an `Fn`, and a greetd connection cannot be cloned into
    // it, so the state is built once and handed over on the first call.
    // A single-window application boots exactly once; if that ever stops
    // being true, this will say so loudly rather than silently open a
    // second login screen with no way to authenticate.
    let drew = std::rc::Rc::new(std::cell::Cell::new(false));
    let once = std::cell::RefCell::new(Some(Greeter {
        conversation: Conversation::new(backend, args.user.clone()),
        drew: std::rc::Rc::clone(&drew),
        #[cfg(debug_assertions)]
        type_in: args.type_in.clone(),
        theme,
        command,
        launched: false,
    }));

    iced::application(
        move || {
            (
                once.borrow_mut().take().expect("the greeter boots once"),
                Task::none(),
            )
        },
        Greeter::update,
        Greeter::view,
    )
    .subscription(Greeter::subscription)
    .default_font(font)
    .window(iced::window::Settings {
        // A login screen owns the display. Fullscreen rather than
        // maximised so there is no chrome to look behind.
        fullscreen: true,
        decorations: false,
        ..iced::window::Settings::default()
    })
    .run()?;

    // Ending without ever having drawn is a failure, whatever the event
    // loop thought. Reporting it as success is what made a lost startup
    // race look like a rejected password.
    if !drew.get() {
        eprintln!(
            "the greeter exited without ever drawing — it could not open a window on \
             WAYLAND_DISPLAY={}. If it was started alongside the compositor, it has to \
             wait for the compositor's socket first; see config/hyprland-greeter.lua.",
            std::env::var("WAYLAND_DISPLAY").unwrap_or_else(|_| "(unset)".into())
        );
        std::process::exit(1);
    }
    Ok(())
}


#[cfg(test)]
mod tests {
    use super::*;
    use hyprforge_authui::conversation::{Backend, Prompt, Response};

    /// A backend that just asks for a password, so key handling can be
    /// exercised without a socket.
    struct Asking(Option<Response>);
    impl Backend for Asking {
        fn start(&mut self, _username: &str) {
            self.0 = Some(Response::Ask(Prompt::secret("Password:")));
        }
        fn answer(&mut self, _answer: &str) {}
        fn proceed(&mut self) {}
        fn poll(&mut self) -> Option<Response> {
            self.0.take()
        }
    }

    fn typing() -> Conversation<Asking> {
        Conversation::new(Asking(None), "apost")
    }

    /// Shifted characters must survive.
    ///
    /// iced reports three things for a key press: the unmodified key,
    /// the modified key, and the text produced. Reading the first one
    /// turns `SHIFT + j` into `j`, so every capital and symbol in a
    /// password is silently wrong and PAM rejects a password that was
    /// typed correctly — while each attempt spends a faillock slot.
    #[test]
    fn what_reaches_the_password_is_the_text_that_was_typed() {
        let mut c = typing();
        // What iced sends for SHIFT+j: the key is still lowercase and
        // the text carries the capital.
        apply_key(&mut c, Key::Character("j".into()), Some("J".into()));
        apply_key(&mut c, Key::Character("1".into()), Some("!".into()));
        apply_key(&mut c, Key::Character("a".into()), Some("a".into()));
        assert_eq!(c.entered(), "J!a", "the shift was lost somewhere");
    }

    /// Enter and Backspace also produce text — "\r" and "\u{8}" — and
    /// appending those would put invisible characters in the password.
    #[test]
    fn keys_that_are_not_characters_never_reach_the_password() {
        let mut c = typing();
        apply_key(&mut c, Key::Character("a".into()), Some("a".into()));
        apply_key(&mut c, Key::Named(Named::Backspace), Some("\u{8}".into()));
        apply_key(&mut c, Key::Character("b".into()), Some("b".into()));
        assert_eq!(c.entered(), "b", "backspace should delete, not type");

        // Escape clears rather than typing an escape character.
        apply_key(&mut c, Key::Character("x".into()), Some("x".into()));
        apply_key(&mut c, Key::Named(Named::Escape), Some("\u{1b}".into()));
        assert_eq!(c.entered(), "");
    }
}
