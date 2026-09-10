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
    Key(Key),
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
            Message::Key(key) => self.key(key),
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

    /// One key, in the same terms the lock screen uses.
    fn key(&mut self, key: Key) {
        match key {
            Key::Named(Named::Enter) => match self.conversation.state() {
                State::Telling { .. } => self.conversation.acknowledge(),
                State::Failed { .. } => self.conversation.retry(),
                _ => self.conversation.submit(),
            },
            Key::Named(Named::Escape) => self.conversation.clear(),
            Key::Named(Named::Backspace) => {
                let mut entered = self.conversation.entered().to_string();
                entered.pop();
                self.conversation.type_into(entered);
            }
            Key::Character(text) => {
                // Any key at all leaves the failed state, so someone can
                // simply start typing again rather than work out which
                // key dismisses the error.
                if matches!(self.conversation.state(), State::Failed { .. }) {
                    self.conversation.retry();
                }
                if !text.chars().any(char::is_control) {
                    let mut entered = self.conversation.entered().to_string();
                    entered.push_str(&text);
                    self.conversation.type_into(entered);
                }
            }
            Key::Named(_) | Key::Unidentified => {}
        }
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
                iced::keyboard::Event::KeyPressed { key, .. } => Some(Message::Key(key)),
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

fn main() -> iced::Result {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
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
        .unwrap_or_else(|| std::path::PathBuf::from(hyprforge_look::theme::EXPORT_DIR));
    // A greeter that refused to draw because a theme file was missing
    // would be a machine nobody can log into, so every failure here ends
    // at the default look.
    let theme = hyprforge_authui::screen::renderable(
        Theme::load(&dir.join("theme.toml")).unwrap_or_default(),
    );

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
