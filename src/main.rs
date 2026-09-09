//! A lock screen for Hyprland, sharing its look with the greeter.
//!
//! Runs as you, so it reads your own theme directly. The greeter reads
//! an exported copy — see `hyprforge_authui::theme`.

mod pam;
mod surface;

use clap::Parser;
use hyprforge_authui::conversation::{Backend, Prompt, Response};
use hyprforge_authui::Theme;
use surface::{LockScreen, Outcome};

#[derive(Parser)]
#[command(about = "Lock the session")]
struct Args {
    /// Authenticate against a fixed password instead of PAM.
    ///
    /// For testing the Wayland and drawing paths in a nested compositor
    /// without touching real credentials. On this machine that isn't
    /// merely tidier: pam_faillock is enabled with a failure already
    /// recorded, so a couple of deliberately wrong passwords would lock
    /// the account.
    ///
    /// Refuses to run against the session you are actually using.
    #[arg(long, value_name = "PASSWORD")]
    fake_password: Option<String>,

    /// Which Wayland display to lock. Defaults to $WAYLAND_DISPLAY.
    #[arg(long)]
    display: Option<String>,

    /// Type this in automatically once a frame has been drawn, then
    /// press Enter.
    ///
    /// This is how the unlock path gets tested without a keyboard: it
    /// proves lock, draw, authenticate and unlock end to end. Passing
    /// something other than the fake password exercises the failure
    /// path just as honestly — and against the fake backend rather than
    /// PAM, so no real account gets a failed attempt recorded against
    /// it. Only meaningful alongside `--fake-password`, and so inherits
    /// its refusal to run against the session you are using.
    #[arg(long, value_name = "TEXT", requires = "fake_password")]
    type_in: Option<String>,

    /// Make the fake backend take this many milliseconds to answer.
    ///
    /// Stands in for `pam_unix`, which deliberately sleeps for about two
    /// seconds after a wrong password. The screen has to keep drawing
    /// and keep accepting input throughout — a surface that stops
    /// repainting for two seconds is indistinguishable from one that
    /// crashed, and on a lock screen the user's only other option is a
    /// hard reboot.
    #[arg(long, value_name = "MS", requires = "fake_password")]
    fake_delay: Option<u64>,
}

/// A backend that accepts one fixed password. Testing only.
///
/// Answers through a channel and a ping exactly as the PAM backend
/// does, rather than returning inline. That is deliberate: it means
/// `--fake-delay` exercises the real waking path instead of a shortcut,
/// so what it demonstrates about the UI staying alive is also true of
/// PAM.
struct Fake {
    password: String,
    delay: std::time::Duration,
    to_ui: std::sync::mpsc::Sender<Response>,
    from_worker: std::sync::mpsc::Receiver<Response>,
    ping: calloop::ping::Ping,
}

impl Fake {
    fn new(password: String, delay: std::time::Duration) -> (Fake, calloop::ping::PingSource) {
        let (to_ui, from_worker) = std::sync::mpsc::channel();
        let (ping, source) = calloop::ping::make_ping().expect("failed to create a wakeup pipe");
        (Fake { password, delay, to_ui, from_worker, ping }, source)
    }

    /// Posts a response, after the configured delay if there is one.
    fn emit(&self, response: Response) {
        if self.delay.is_zero() {
            let _ = self.to_ui.send(response);
            self.ping.ping();
            return;
        }
        let (to_ui, ping, delay) = (self.to_ui.clone(), self.ping.clone(), self.delay);
        std::thread::spawn(move || {
            std::thread::sleep(delay);
            let _ = to_ui.send(response);
            ping.ping();
        });
    }
}

impl Backend for Fake {
    fn start(&mut self, _username: &str) {
        self.emit(Response::Ask(Prompt::secret("Password:")));
    }

    fn answer(&mut self, answer: &str) {
        self.emit(if answer == self.password {
            Response::Success
        } else {
            Response::Failure { reason: "Incorrect password".into() }
        });
    }

    fn proceed(&mut self) {
        self.emit(Response::Ask(Prompt::secret("Password:")));
    }

    fn poll(&mut self) -> Option<Response> {
        self.from_worker.try_recv().ok()
    }
}

fn main() -> std::process::ExitCode {
    let args = Args::parse();

    if let Some(display) = &args.display {
        // SAFETY: single-threaded, before anything reads the environment.
        unsafe { std::env::set_var("WAYLAND_DISPLAY", display) };
    }
    let display = std::env::var("WAYLAND_DISPLAY").unwrap_or_default();

    // A fake password on the real session would be a lock screen anyone
    // could open by guessing a test string. Refusing is the only safe
    // default; --display names the nested compositor to test against.
    if args.fake_password.is_some() && args.display.is_none() {
        eprintln!(
            "--fake-password needs --display: it must not be used on the session \
             you're using. Start a nested compositor and point at its display."
        );
        return std::process::ExitCode::FAILURE;
    }

    let username = std::env::var("USER").unwrap_or_else(|_| "unknown".into());
    let theme = Theme::load(&theme_path()).unwrap_or_default();

    let connection = match wayland_client::Connection::connect_to_env() {
        Ok(connection) => connection,
        Err(e) => {
            eprintln!("couldn't connect to the compositor on {display}: {e}");
            return std::process::ExitCode::FAILURE;
        }
    };

    let outcome = match args.fake_password {
        Some(password) => {
            eprintln!("locking {display} with a fake password (testing only)");
            let delay = std::time::Duration::from_millis(args.fake_delay.unwrap_or(0));
            let (fake, wake) = Fake::new(password, delay);
            LockScreen::run(connection, fake, username, theme, Some(wake), args.type_in)
        }
        None => {
            let service = pam::service_name();
            eprintln!("locking {display}, authenticating against PAM service {service:?}");
            // The backend is handed over unstarted on purpose: it only
            // begins talking to PAM once the session is locked. The ping
            // is what lets it answer later without the screen waiting.
            let (backend, wake) = pam::PamBackend::new(service);
            LockScreen::run(connection, backend, username, theme, Some(wake), None)
        }
    };

    match outcome {
        Ok(Outcome::Unlocked) => std::process::ExitCode::SUCCESS,
        // Both causes are already reported where they're diagnosed, and
        // in detail this layer doesn't have. The exit code is the part
        // that differs: a refusal means nothing got locked.
        Ok(Outcome::Refused) => std::process::ExitCode::FAILURE,
        Ok(Outcome::Revoked) => std::process::ExitCode::SUCCESS,
        Ok(Outcome::Disconnected) => {
            eprintln!("lost the connection; the session stays locked");
            std::process::ExitCode::FAILURE
        }
        Err(e) => {
            eprintln!("couldn't lock: {e}");
            std::process::ExitCode::FAILURE
        }
    }
}

/// The user's own theme, which the greeter gets an exported copy of.
fn theme_path() -> std::path::PathBuf {
    hyprforge_paths::lock_toml_path()
}
