//! Authenticating through greetd.
//!
//! greetd is a proxy for a PAM conversation: it asks the questions PAM
//! asks and carries the answers back. That is why this crate and the
//! lock screen can share one screen — the shape of the exchange is the
//! same, and [`hyprforge_authui::conversation`] is that shape.
//!
//! The daemon's own documentation makes the point better than this could:
//! an auth message "can consist of anything ... it is therefore important
//! that no assumptions are made about the questions that will be asked,
//! and attempts to automatically answer these questions should not be
//! made." A greeter that modelled a password box would be wrong the first
//! time someone enabled a hardware token.
//!
//! Like PAM, greetd is answered on its own thread. The reason is the
//! same: PAM sits behind greetd, so a wrong password still costs the
//! deliberate `pam_unix` delay, and a login screen that stops repainting
//! for two seconds looks broken.

use greetd_ipc::codec::SyncCodec;
use greetd_ipc::{AuthMessageType, ErrorType, Request, Response as Greetd};
use hyprforge_authui::conversation::{Backend, Prompt, Response};
use std::os::unix::net::UnixStream;
use std::sync::mpsc::{Receiver, Sender};

/// Where greetd told us to talk to it.
///
/// Set by greetd for the process it launches, so its absence means this
/// is not running as a greeter — which is worth saying plainly rather
/// than failing to connect to a path nobody mentioned.
pub const SOCKET_ENV: &str = "GREETD_SOCK";

#[derive(Debug, thiserror::Error)]
pub enum GreetdError {
    #[error("{SOCKET_ENV} is not set — this is only meaningful when greetd starts it")]
    NotAGreeter,
    #[error("couldn't reach greetd on {path}: {source}")]
    Connect {
        path: String,
        #[source]
        source: std::io::Error,
    },
}

/// What the UI thread asks the greetd thread to do.
///
/// No `CancelSession`, deliberately. greetd cancels a session itself on
/// error, so a failed password needs only another `CreateSession` — which
/// is what `Conversation::retry` already does. The remaining use for
/// cancelling is abandoning a *working* session to switch users, and
/// there is no way to switch users yet: the screen shows a username but
/// has no field for choosing one. Adding the request before the UI that
/// needs it would be guessing at its shape.
enum Ask {
    Start(String),
    Answer(Option<String>),
    /// Not part of the conversation: the session to launch once this
    /// process exits.
    Launch { command: Vec<String>, environment: Vec<String> },
}

/// greetd, answered on its own thread.
pub struct GreetdBackend {
    to_greetd: Sender<Ask>,
    from_greetd: Receiver<Response>,
    /// Queued for the next `poll` when the thread cannot be reached.
    lost: Option<Response>,
    /// Whether the thread's death has already been reported. A `poll`
    /// that answers a disconnected channel every time turns
    /// `Conversation::pump` into a spin, because pump loops while poll
    /// keeps yielding.
    reported_loss: bool,
}

impl GreetdBackend {
    /// Connects to the socket greetd named.
    pub fn connect() -> Result<GreetdBackend, GreetdError> {
        let path = std::env::var(SOCKET_ENV).map_err(|_| GreetdError::NotAGreeter)?;
        let stream = UnixStream::connect(&path)
            .map_err(|source| GreetdError::Connect { path: path.clone(), source })?;
        Ok(GreetdBackend::over(stream))
    }

    /// Talks to an already-connected socket.
    ///
    /// Split out so the whole exchange can be driven against a stand-in
    /// greetd speaking the real wire format, with no daemon installed
    /// and nothing touching how this machine logs in.
    pub fn over(stream: UnixStream) -> GreetdBackend {
        let (to_greetd, asks) = std::sync::mpsc::channel();
        let (answers, from_greetd) = std::sync::mpsc::channel();
        std::thread::Builder::new()
            .name("greetd".into())
            .spawn(move || run(stream, asks, answers))
            .expect("failed to start the greetd thread");
        GreetdBackend { to_greetd, from_greetd, lost: None, reported_loss: false }
    }

    /// Asks greetd to run `command` once this process exits.
    ///
    /// Only valid after the conversation reported success. greetd starts
    /// the session when the greeter goes away, so the caller's next move
    /// is to exit — see the note on `Outcome` in `main`.
    pub fn launch(&mut self, command: Vec<String>, environment: Vec<String>) {
        self.post(Ask::Launch { command, environment });
    }

    fn post(&mut self, ask: Ask) {
        if self.to_greetd.send(ask).is_err() {
            // A failed send and a disconnected receiver are the same
            // fact, so they share one "already said" flag rather than
            // each counting to one separately.
            self.reported_loss = true;
            self.lost = Some(unreachable_service());
        }
    }
}

impl Backend for GreetdBackend {
    fn start(&mut self, username: &str) {
        self.post(Ask::Start(username.to_string()));
    }

    fn answer(&mut self, answer: &str) {
        self.post(Ask::Answer(Some(answer.to_string())));
    }

    /// Acknowledges a message greetd did not want an answer to.
    ///
    /// greetd still expects a `PostAuthMessageResponse` for an info or
    /// error message — with no response in it. Skipping it leaves the
    /// conversation waiting forever on a question the user has already
    /// read and dismissed.
    fn proceed(&mut self) {
        self.post(Ask::Answer(None));
    }

    fn poll(&mut self) -> Option<Response> {
        if let Some(lost) = self.lost.take() {
            return Some(lost);
        }
        match self.from_greetd.try_recv() {
            Ok(response) => Some(response),
            Err(std::sync::mpsc::TryRecvError::Empty) => None,
            Err(std::sync::mpsc::TryRecvError::Disconnected) if !self.reported_loss => {
                self.reported_loss = true;
                Some(unreachable_service())
            }
            Err(std::sync::mpsc::TryRecvError::Disconnected) => None,
        }
    }
}

/// The greetd thread. Kept small: everything here can start a session.
fn run(mut stream: UnixStream, asks: Receiver<Ask>, answers: Sender<Response>) {
    while let Ok(ask) = asks.recv() {
        let request = match ask {
            Ask::Start(username) => Request::CreateSession { username },
            Ask::Answer(response) => Request::PostAuthMessageResponse { response },
            Ask::Launch { command, environment } => {
                Request::StartSession { cmd: command, env: environment }
            }
        };

        if request.write_to(&mut stream).is_err() {
            let _ = answers.send(unreachable_service());
            return;
        }
        let Ok(reply) = Greetd::read_from(&mut stream) else {
            let _ = answers.send(unreachable_service());
            return;
        };
        if answers.send(translate(reply)).is_err() {
            return;
        }
    }
}

fn unreachable_service() -> Response {
    Response::Failure { reason: "the login service stopped responding".into() }
}

/// greetd's reply in the terms the shared screen understands.
///
/// The mapping is almost the identity, which is the point: greetd is
/// relaying a PAM conversation, and this is the same conversation.
fn translate(reply: Greetd) -> Response {
    match reply {
        Greetd::Success => Response::Success,
        Greetd::AuthMessage { auth_message_type, auth_message } => match auth_message_type {
            // Secret and visible are both questions; which one decides
            // whether what is typed is shown. Getting this backwards
            // would either hide a 2FA code or print a password.
            AuthMessageType::Secret => Response::Ask(Prompt::secret(auth_message)),
            AuthMessageType::Visible => Response::Ask(Prompt::visible(auth_message)),
            // Info and error are statements. They still have to be
            // acknowledged, which `proceed` does.
            AuthMessageType::Info => Response::Tell { text: auth_message, error: false },
            AuthMessageType::Error => Response::Tell { text: auth_message, error: true },
        },
        // An auth error is a wrong password and reads as one; anything
        // else is the service itself having a problem, and saying
        // "incorrect password" for a broken PAM stack would send someone
        // retyping a password that was right.
        Greetd::Error { error_type: ErrorType::AuthError, description } => {
            Response::Failure { reason: plainly(&description) }
        }
        Greetd::Error { error_type: ErrorType::Error, description } => {
            Response::Failure { reason: format!("Login failed: {description}") }
        }
    }
}

/// greetd passes PAM's wording through, and PAM's wording for the common
/// case reads like the program broke. The lock screen makes the same
/// substitution for the same reason.
fn plainly(description: &str) -> String {
    if description.to_lowercase().contains("authentication failure") {
        return "Incorrect password".into();
    }
    description.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use hyprforge_authui::conversation::{Conversation, State};
    use std::os::unix::net::UnixListener;

    /// A stand-in greetd, speaking the real wire format over a real
    /// socket.
    ///
    /// Not a mock of this crate's own types: it uses `greetd_ipc`'s
    /// codec to read requests and write replies, so a mistake in the
    /// framing or the JSON tags would fail here rather than at the login
    /// screen. Which matters more than usual — the thing this stands in
    /// for is the only way into the machine.
    fn fake_greetd(replies: Vec<Greetd>) -> (UnixStream, std::thread::JoinHandle<Vec<Request>>) {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("greetd.sock");
        let listener = UnixListener::bind(&path).expect("bind");
        let client = UnixStream::connect(&path).expect("connect");

        let server = std::thread::spawn(move || {
            // `dir` is moved in so the socket outlives the bind.
            let _dir = dir;
            let (mut stream, _) = listener.accept().expect("accept");
            let mut seen = Vec::new();
            for reply in replies {
                match Request::read_from(&mut stream) {
                    Ok(request) => seen.push(request),
                    Err(_) => break,
                }
                if reply.write_to(&mut stream).is_err() {
                    break;
                }
            }
            seen
        });
        (client, server)
    }

    /// Waits for the worker thread to answer.
    ///
    /// The backend is deliberately non-blocking, so a test has to pump
    /// like the real host does rather than assume the reply is instant.
    fn settle<B: Backend>(conversation: &mut Conversation<B>) {
        for _ in 0..200 {
            if conversation.pump() {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        panic!("greetd never answered");
    }

    #[test]
    fn a_password_login_runs_end_to_end() {
        let (stream, server) = fake_greetd(vec![
            Greetd::AuthMessage {
                auth_message_type: AuthMessageType::Secret,
                auth_message: "Password:".into(),
            },
            Greetd::Success,
        ]);
        let mut conversation = Conversation::new(GreetdBackend::over(stream), "apost");
        settle(&mut conversation);

        match conversation.state() {
            State::Asking { prompt, .. } => {
                assert_eq!(prompt.text, "Password:");
                assert!(prompt.secret, "a password must not be shown as it is typed");
            }
            other => panic!("expected a question, got {other:?}"),
        }

        conversation.type_into("hunter2".into());
        conversation.submit();
        settle(&mut conversation);
        assert!(conversation.state().is_authenticated());

        // And the requests that actually went down the socket were the
        // ones greetd expects, in order.
        let seen = server.join().expect("server thread");
        assert!(matches!(&seen[0], Request::CreateSession { username } if username == "apost"));
        assert!(
            matches!(&seen[1], Request::PostAuthMessageResponse { response } if response.as_deref() == Some("hunter2"))
        );
    }

    /// greetd relays whatever PAM asks, which is not always a password.
    /// A greeter that assumed otherwise would break the first time
    /// someone enabled a token.
    #[test]
    fn a_visible_question_is_not_treated_as_a_password() {
        let (stream, _server) = fake_greetd(vec![Greetd::AuthMessage {
            auth_message_type: AuthMessageType::Visible,
            auth_message: "Verification code:".into(),
        }]);
        let mut conversation = Conversation::new(GreetdBackend::over(stream), "apost");
        settle(&mut conversation);
        match conversation.state() {
            State::Asking { prompt, .. } => {
                assert_eq!(prompt.text, "Verification code:");
                assert!(!prompt.secret, "a 2FA code is not hidden input");
            }
            other => panic!("expected a visible question, got {other:?}"),
        }
    }

    /// An info message is a statement, and greetd still wants it
    /// acknowledged with a response containing nothing. Sending an empty
    /// *string* instead would answer a question nobody asked.
    #[test]
    fn a_message_is_acknowledged_with_no_answer() {
        let (stream, server) = fake_greetd(vec![
            Greetd::AuthMessage {
                auth_message_type: AuthMessageType::Info,
                auth_message: "Password expires in 3 days".into(),
            },
            Greetd::Success,
        ]);
        let mut conversation = Conversation::new(GreetdBackend::over(stream), "apost");
        settle(&mut conversation);
        assert_eq!(
            conversation.state(),
            &State::Telling { text: "Password expires in 3 days".into(), error: false }
        );

        conversation.acknowledge();
        settle(&mut conversation);
        assert!(conversation.state().is_authenticated());

        let seen = server.join().expect("server thread");
        assert!(
            matches!(&seen[1], Request::PostAuthMessageResponse { response } if response.is_none()),
            "an info message must be answered with no response, got {:?}",
            seen[1]
        );
    }

    /// The message a user sees most often should read as "you typed it
    /// wrong", not as "the program broke" — the same substitution the
    /// lock screen makes, for the same reason.
    #[test]
    fn a_wrong_password_reads_plainly() {
        let (stream, _server) = fake_greetd(vec![Greetd::Error {
            error_type: ErrorType::AuthError,
            description: "Authentication failure".into(),
        }]);
        let mut conversation = Conversation::new(GreetdBackend::over(stream), "apost");
        settle(&mut conversation);
        assert_eq!(
            conversation.state(),
            &State::Failed { reason: "Incorrect password".into() }
        );
    }

    /// A broken PAM stack is not a wrong password, and saying so would
    /// send someone retyping a password that was right. This is exactly
    /// the mistake the lock screen made until its own account check was
    /// reported separately.
    #[test]
    fn a_service_error_is_not_blamed_on_the_password() {
        let (stream, _server) = fake_greetd(vec![Greetd::Error {
            error_type: ErrorType::Error,
            description: "PAM: module is not known".into(),
        }]);
        let mut conversation = Conversation::new(GreetdBackend::over(stream), "apost");
        settle(&mut conversation);
        match conversation.state() {
            State::Failed { reason } => {
                assert!(reason.contains("module is not known"), "{reason}");
                assert!(
                    !reason.to_lowercase().contains("incorrect password"),
                    "a service error must not be blamed on the password: {reason}"
                );
            }
            other => panic!("expected a failure, got {other:?}"),
        }
    }

    /// greetd going away must not hang the login screen, which would be
    /// a machine nobody can get into.
    ///
    /// Built without a server thread on purpose. The first version used
    /// `fake_greetd(vec![])` and then joined it, which hung in the
    /// test's own `accept` rather than in the code under test — a test
    /// that hangs while proving something does not hang is worse than no
    /// test. Here the peer is closed explicitly and nothing is waited
    /// on.
    #[test]
    fn a_greetd_that_disappears_reports_failure_rather_than_hanging() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("greetd.sock");
        let listener = UnixListener::bind(&path).expect("bind");
        let client = UnixStream::connect(&path).expect("connect");
        let server = listener.accept().expect("accept").0;

        // greetd's side goes away, as it would if the daemon died.
        drop(server);
        drop(listener);

        let mut conversation = Conversation::new(GreetdBackend::over(client), "apost");
        settle(&mut conversation);
        assert!(
            matches!(conversation.state(), State::Failed { .. }),
            "a dead login service must be reported, got {:?}",
            conversation.state()
        );
    }
}
