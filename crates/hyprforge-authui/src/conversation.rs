//! Asking who you are, without caring who answers.
//!
//! Authentication is a **conversation**, not a password box. PAM emits a
//! sequence of prompts — `Password:`, a fingerprint request, a 2FA
//! challenge — and greetd's protocol is a proxy for exactly that: it
//! forwards PAM's messages and carries the replies back. Both sides speak
//! the same shape.
//!
//! That is what lets one screen serve both. If this modelled "a password
//! field", the two hosts would diverge the moment anything but a password
//! was involved — a fingerprint reader on the lock screen and a password
//! box on the greeter is precisely the mismatch this crate exists to
//! prevent.
//!
//! The state machine is deliberately small and total, because the failure
//! modes are unusually unforgiving: a lock screen that gets stuck is a
//! machine you cannot get into, and one that reports success wrongly is
//! worse.

use std::fmt;

/// A question the authenticator is asking.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Prompt {
    pub text: String,
    /// Whether what the user types should be hidden. PAM distinguishes
    /// these, and so does greetd — getting it backwards either shows a
    /// password on screen or hides an answer that isn't secret.
    pub secret: bool,
}

impl Prompt {
    pub fn secret(text: impl Into<String>) -> Prompt {
        Prompt { text: text.into(), secret: true }
    }

    pub fn visible(text: impl Into<String>) -> Prompt {
        Prompt { text: text.into(), secret: false }
    }
}

/// What a backend says when asked to continue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Response {
    /// Ask the user this.
    Ask(Prompt),
    /// Tell the user something; no answer wanted. PAM's "info" and
    /// "error" messages — password expiry warnings, account notices.
    Tell { text: String, error: bool },
    /// Authentication succeeded.
    Success,
    /// This attempt failed. The conversation restarts.
    Failure { reason: String },
}

/// Where a conversation currently is.
///
/// `Debug` is written by hand below rather than derived: what the user
/// has typed is a password, and a derived one would put it in every log
/// line and panic message.
#[derive(Clone, PartialEq, Eq)]
pub enum State {
    /// Waiting for the user to answer `prompt`.
    Asking { prompt: Prompt, entered: String },
    /// Waiting on the backend.
    Working,
    /// Showing a message before continuing.
    Telling { text: String, error: bool },
    /// Done. The host acts on this — unlock the session, or start one.
    Authenticated,
    /// The last attempt failed. Shown until the user starts typing again.
    Failed { reason: String },
}

impl State {
    /// Whether the user can type right now.
    pub fn accepts_input(&self) -> bool {
        matches!(self, State::Asking { .. })
    }

    /// Whether the host should act — unlock, or launch a session.
    ///
    /// The only place that returns true, so a host cannot accidentally
    /// treat any other state as success.
    pub fn is_authenticated(&self) -> bool {
        matches!(self, State::Authenticated)
    }
}

/// Whatever actually decides: PAM on a lock screen, greetd on a greeter.
///
/// Deliberately synchronous and single-stepped. Both real backends are
/// request/response, and a host that pumps one step at a time can keep
/// the UI drawing between them — which matters on a lock screen, where a
/// frozen surface is indistinguishable from a crashed one.
pub trait Backend {
    /// Begin, or begin again after a failure. Returns the first thing to
    /// do.
    fn start(&mut self, username: &str) -> Response;

    /// Supply the answer to the last [`Response::Ask`].
    fn answer(&mut self, answer: &str) -> Response;

    /// Acknowledge a [`Response::Tell`] and continue.
    fn proceed(&mut self) -> Response;
}

/// The conversation, driven by the UI.
pub struct Conversation<B: Backend> {
    backend: B,
    username: String,
    state: State,
    /// Failed attempts since the last success, for the host to rate-limit
    /// on. Counted here because both hosts need it and neither should
    /// invent its own rule.
    failures: u32,
}

impl<B: Backend> Conversation<B> {
    /// Starts a conversation for `username`.
    pub fn new(backend: B, username: impl Into<String>) -> Conversation<B> {
        let mut conversation = Conversation {
            backend,
            username: username.into(),
            state: State::Working,
            failures: 0,
        };
        let first = conversation.backend.start(&conversation.username.clone());
        conversation.apply(first);
        conversation
    }

    pub fn state(&self) -> &State {
        &self.state
    }

    pub fn username(&self) -> &str {
        &self.username
    }

    pub fn failures(&self) -> u32 {
        self.failures
    }

    /// What the user has typed so far, if anything is being asked.
    pub fn entered(&self) -> &str {
        match &self.state {
            State::Asking { entered, .. } => entered,
            _ => "",
        }
    }

    /// Records a keystroke. Ignored unless something is being asked, so
    /// input arriving while the backend is working can't be lost into a
    /// buffer that is about to be replaced.
    pub fn type_into(&mut self, text: String) {
        if let State::Asking { entered, .. } = &mut self.state {
            *entered = text;
        }
    }

    /// Submits the current answer.
    ///
    /// Does nothing unless something is being asked — a stray Enter while
    /// the backend is working must not send an empty answer.
    pub fn submit(&mut self) {
        let State::Asking { entered, .. } = &self.state else {
            return;
        };
        let answer = entered.clone();
        self.state = State::Working;
        let next = self.backend.answer(&answer);
        self.apply(next);
    }

    /// Acknowledges a message and continues.
    pub fn acknowledge(&mut self) {
        if !matches!(self.state, State::Telling { .. }) {
            return;
        }
        self.state = State::Working;
        let next = self.backend.proceed();
        self.apply(next);
    }

    /// Starts over after a failure.
    pub fn retry(&mut self) {
        if !matches!(self.state, State::Failed { .. }) {
            return;
        }
        self.state = State::Working;
        let first = self.backend.start(&self.username.clone());
        self.apply(first);
    }

    /// Clears whatever has been typed without sending it.
    pub fn clear(&mut self) {
        if let State::Asking { entered, .. } = &mut self.state {
            entered.clear();
        }
    }

    fn apply(&mut self, response: Response) {
        self.state = match response {
            Response::Ask(prompt) => State::Asking { prompt, entered: String::new() },
            Response::Tell { text, error } => State::Telling { text, error },
            Response::Success => {
                self.failures = 0;
                State::Authenticated
            }
            Response::Failure { reason } => {
                self.failures += 1;
                State::Failed { reason }
            }
        };
    }
}

impl fmt::Debug for State {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // What was typed never reaches a log, a panic message or a
        // debug dump. It is a password.
        match self {
            State::Asking { prompt, entered } => f
                .debug_struct("Asking")
                .field("prompt", prompt)
                .field("entered", &format_args!("<{} chars>", entered.len()))
                .finish(),
            State::Working => f.write_str("Working"),
            State::Telling { text, error } => f
                .debug_struct("Telling")
                .field("text", text)
                .field("error", error)
                .finish(),
            State::Authenticated => f.write_str("Authenticated"),
            State::Failed { reason } => f.debug_struct("Failed").field("reason", reason).finish(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A backend that plays a fixed script, so the state machine can be
    /// driven through shapes real backends produce.
    struct Script {
        steps: Vec<Response>,
        at: usize,
        answers: Vec<String>,
        starts: u32,
    }

    impl Script {
        fn new(steps: Vec<Response>) -> Script {
            Script { steps, at: 0, answers: Vec::new(), starts: 0 }
        }

        fn next(&mut self) -> Response {
            let response = self.steps.get(self.at).cloned().unwrap_or(Response::Failure {
                reason: "script ended".into(),
            });
            self.at += 1;
            response
        }
    }

    impl Backend for Script {
        fn start(&mut self, _username: &str) -> Response {
            self.starts += 1;
            self.at = 0;
            self.next()
        }

        fn answer(&mut self, answer: &str) -> Response {
            self.answers.push(answer.to_string());
            self.next()
        }

        fn proceed(&mut self) -> Response {
            self.next()
        }
    }

    fn password_then(outcome: Response) -> Script {
        Script::new(vec![Response::Ask(Prompt::secret("Password:")), outcome])
    }

    #[test]
    fn a_password_prompt_is_asked_then_accepted() {
        let mut c = Conversation::new(password_then(Response::Success), "apost");
        assert_eq!(
            c.state(),
            &State::Asking { prompt: Prompt::secret("Password:"), entered: String::new() }
        );
        assert!(c.state().accepts_input());

        c.type_into("hunter2".into());
        c.submit();
        assert!(c.state().is_authenticated());
        assert_eq!(c.failures(), 0);
    }

    #[test]
    fn the_typed_answer_reaches_the_backend_verbatim() {
        let mut backend = password_then(Response::Success);
        let mut c = Conversation::new(&mut backend, "apost");
        c.type_into("  spaces and $ymbols ".into());
        c.submit();
        assert_eq!(backend.answers, vec!["  spaces and $ymbols "]);
    }

    /// The reason this isn't modelled as a password box: PAM and greetd
    /// both ask several questions, and not all of them are secret.
    #[test]
    fn a_multi_step_conversation_asks_each_question_in_turn() {
        let script = Script::new(vec![
            Response::Ask(Prompt::secret("Password:")),
            Response::Ask(Prompt::visible("Verification code:")),
            Response::Success,
        ]);
        let mut c = Conversation::new(script, "apost");

        assert!(matches!(c.state(), State::Asking { prompt, .. } if prompt.secret));
        c.type_into("hunter2".into());
        c.submit();

        match c.state() {
            State::Asking { prompt, entered } => {
                assert_eq!(prompt.text, "Verification code:");
                assert!(!prompt.secret, "a 2FA code is not hidden input");
                assert!(entered.is_empty(), "the new prompt starts empty");
            }
            other => panic!("expected a second question, got {other:?}"),
        }
        c.type_into("123456".into());
        c.submit();
        assert!(c.state().is_authenticated());
    }

    #[test]
    fn a_message_is_shown_and_acknowledged_before_continuing() {
        let script = Script::new(vec![
            Response::Tell { text: "Password expires in 3 days".into(), error: false },
            Response::Ask(Prompt::secret("Password:")),
            Response::Success,
        ]);
        let mut c = Conversation::new(script, "apost");
        assert_eq!(
            c.state(),
            &State::Telling { text: "Password expires in 3 days".into(), error: false }
        );
        assert!(!c.state().accepts_input(), "there's nothing to answer");
        c.acknowledge();
        assert!(c.state().accepts_input());
    }

    #[test]
    fn a_failure_is_counted_and_can_be_retried() {
        let mut c = Conversation::new(
            password_then(Response::Failure { reason: "Incorrect".into() }),
            "apost",
        );
        c.type_into("wrong".into());
        c.submit();
        assert_eq!(c.state(), &State::Failed { reason: "Incorrect".into() });
        assert_eq!(c.failures(), 1);
        assert!(!c.state().is_authenticated());

        c.retry();
        assert!(c.state().accepts_input(), "a retry asks again");
        c.type_into("wrong again".into());
        c.submit();
        assert_eq!(c.failures(), 2, "failures accumulate across attempts");
    }

    /// A lock screen that gets stuck is a machine you can't get into, so
    /// every input in every state has to be defined.
    #[test]
    fn input_outside_a_question_is_ignored_rather_than_stuck() {
        let mut c = Conversation::new(
            password_then(Response::Failure { reason: "no".into() }),
            "apost",
        );
        c.type_into("wrong".into());
        c.submit();

        // Failed: typing and submitting must do nothing at all.
        c.type_into("more".into());
        c.submit();
        assert_eq!(c.entered(), "");
        assert_eq!(c.failures(), 1, "a stray submit must not count as an attempt");

        // And acknowledging something that isn't a message does nothing.
        c.acknowledge();
        assert!(matches!(c.state(), State::Failed { .. }));
    }

    /// A stray Enter while the backend is working must not send an empty
    /// answer and burn an attempt.
    #[test]
    fn submitting_while_working_sends_nothing() {
        let mut backend = password_then(Response::Success);
        let mut c = Conversation::new(&mut backend, "apost");
        c.type_into("hunter2".into());
        c.submit();
        assert!(c.state().is_authenticated());

        c.submit();
        assert_eq!(backend.answers.len(), 1, "only the real answer was sent");
    }

    #[test]
    fn clearing_discards_what_was_typed_without_sending_it() {
        let mut backend = password_then(Response::Success);
        let mut c = Conversation::new(&mut backend, "apost");
        c.type_into("typo".into());
        c.clear();
        assert_eq!(c.entered(), "");
        assert!(backend.answers.is_empty());
    }

    /// Only one state means success, so a host can't mistake anything
    /// else for it.
    #[test]
    fn only_authenticated_counts_as_success() {
        for state in [
            State::Working,
            State::Asking { prompt: Prompt::secret("p"), entered: "x".into() },
            State::Telling { text: "t".into(), error: true },
            State::Failed { reason: "r".into() },
        ] {
            assert!(!state.is_authenticated(), "{state:?} must not read as success");
        }
        assert!(State::Authenticated.is_authenticated());
    }

    /// What was typed is a password. It must not reach a log, a panic
    /// message, or a debug dump.
    #[test]
    fn debug_output_never_contains_what_was_typed() {
        let state = State::Asking {
            prompt: Prompt::secret("Password:"),
            entered: "hunter2".into(),
        };
        let rendered = format!("{state:?}");
        assert!(!rendered.contains("hunter2"), "{rendered}");
        assert!(rendered.contains("7 chars"), "{rendered}");
    }
}

impl<B: Backend> Backend for &mut B {
    fn start(&mut self, username: &str) -> Response {
        (**self).start(username)
    }

    fn answer(&mut self, answer: &str) -> Response {
        (**self).answer(answer)
    }

    fn proceed(&mut self) -> Response {
        (**self).proceed()
    }
}
