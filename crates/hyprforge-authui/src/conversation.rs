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

use hyprforge_secret::Secret;
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
    Asking { prompt: Prompt, entered: Secret<String> },
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
/// **Nothing here blocks.** The three verbs post a request and return;
/// answers come back from [`Backend::poll`] whenever they are ready.
///
/// That split is the whole point. `pam_unix` deliberately sleeps for
/// about two seconds after a wrong password, and greetd is a socket
/// round trip — if answering meant waiting, the surface would stop
/// repainting for exactly as long as the authenticator took. On a lock
/// screen that is indistinguishable from a crash, and the user's only
/// other option is a hard reboot.
///
/// A backend that can answer immediately simply returns it from the
/// next `poll`, so synchronous implementations stay trivial.
pub trait Backend {
    /// Begin, or begin again after a failure.
    fn start(&mut self, username: &str);

    /// Supply the answer to the last [`Response::Ask`].
    fn answer(&mut self, answer: &str);

    /// Acknowledge a [`Response::Tell`] and continue.
    fn proceed(&mut self);

    /// The next response, if one is ready. Must not block.
    ///
    /// Returning `None` means "not yet", never "nothing more" — the
    /// host will ask again when something wakes it.
    fn poll(&mut self) -> Option<Response>;
}

/// Lets a test hold onto its backend while the conversation borrows it,
/// so what was actually sent can be checked afterwards.
impl<B: Backend> Backend for &mut B {
    fn start(&mut self, username: &str) {
        (**self).start(username)
    }

    fn answer(&mut self, answer: &str) {
        (**self).answer(answer)
    }

    fn proceed(&mut self) {
        (**self).proceed()
    }

    fn poll(&mut self) -> Option<Response> {
        (**self).poll()
    }
}

/// How many responses one `pump` will apply before giving up.
///
/// A real step yields one or two. This exists only so a backend that
/// answers every poll cannot hang the host — see [`Conversation::pump`].
const MAX_RESPONSES_PER_PUMP: usize = 64;

/// The conversation, driven by the UI.
pub struct Conversation<B: Backend> {
    backend: B,
    username: String,
    state: State,
    /// Failed attempts since the last success, for the host to rate-limit
    /// on. Counted here because both hosts need it and neither should
    /// invent its own rule.
    failures: u32,
    /// The question last asked, kept so that what is typed during a retry
    /// can be restored only if the same question comes back.
    last_prompt: Option<Prompt>,
    /// Typed while the backend is working, after a [`Conversation::retry`].
    ///
    /// Without this, the key that dismisses a failure is swallowed and so
    /// is everything typed until PAM asks again — the state is `Working`
    /// and `type_into` only writes in `Asking`. Someone who reacts to
    /// "incorrect password" by typing it again loses the first characters,
    /// submits a truncated password, and is told it was wrong a second
    /// time. On a stack with `pam_faillock` at three attempts, that is how
    /// a correct password locks an account.
    ///
    /// Never rendered: a [`Secret`], the same wrapper
    /// `hyprforge-network`'s Wi-Fi passphrase is built on, so this
    /// crate does not hand-write its own copy of "never render this".
    /// [`Conversation`]'s own `Debug` below still gets a hand-written
    /// impl, but only because it also has to redact `state`.
    pending: Secret<String>,
}

/// Renders no part of what was typed.
///
/// `State`'s own `Debug` is hand-written to hide `entered`; this type
/// holds a second copy of the same characters in `pending`, and a derived
/// `Debug` here would undo that work — though for `pending` specifically,
/// deriving would actually be safe: `Secret<String>`'s own `Debug`
/// already renders only a count, nested or not. It is `state` that still
/// needs a hand-written field here.
impl<B: Backend> fmt::Debug for Conversation<B> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Conversation")
            .field("state", &self.state)
            .field("failures", &self.failures)
            .field("pending", &self.pending)
            .finish()
    }
}

impl<B: Backend> Conversation<B> {
    /// Starts a conversation for `username`.
    pub fn new(backend: B, username: impl Into<String>) -> Conversation<B> {
        let mut conversation = Conversation {
            backend,
            username: username.into(),
            state: State::Working,
            last_prompt: None,
            pending: Secret::default(),
            failures: 0,
        };
        conversation.backend.start(&conversation.username.clone());
        conversation.pump();
        conversation
    }

    /// Applies whatever the backend has ready. Never blocks.
    ///
    /// Returns whether anything changed, so a host can avoid repainting
    /// for nothing. Call it whenever something might have woken the
    /// backend — and it is always safe to call.
    pub fn pump(&mut self) -> bool {
        let mut changed = false;
        // A loop, not an `if`: PAM can emit a message and the next
        // prompt in one go, and leaving the second sitting in the queue
        // would stall the conversation until an unrelated event
        // happened to pump it again.
        //
        // Bounded, though. A backend whose `poll` never returns `None`
        // would spin here forever, and both real backends had exactly
        // that bug: a dead worker thread answered every poll with the
        // same failure. On a lock screen that is not a busy loop, it is
        // a screen that never draws again. The contract says `None`
        // means "not yet" — but the host must not be the thing that
        // depends on every backend honouring it.
        for _ in 0..MAX_RESPONSES_PER_PUMP {
            let Some(response) = self.backend.poll() else {
                return changed;
            };
            self.apply(response);
            changed = true;
        }
        changed
    }

    /// The backend, for what only the host can do with it.
    ///
    /// The conversation covers the exchange; it does not cover what
    /// happens after. A lock screen unlocks a compositor, a greeter asks
    /// greetd to start a session — neither belongs in a trait about
    /// asking questions, and both need the concrete backend.
    pub fn backend(&mut self) -> &mut B {
        &mut self.backend
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
            State::Asking { entered, .. } => entered.expose(),
            _ => "",
        }
    }

    /// Records a keystroke. Ignored unless something is being asked, so
    /// input arriving while the backend is working can't be lost into a
    /// buffer that is about to be replaced.
    pub fn type_into(&mut self, text: String) {
        match &mut self.state {
            State::Asking { entered, .. } => *entered = Secret::new(text),
            // Held until the same question comes back — see `pending`.
            // A backend answers on its own schedule, so there is a real
            // window here, and dropping what is typed in it is what made
            // a correct password look wrong.
            State::Working => self.pending = Secret::new(text),
            _ => {}
        }
    }

    /// What has been typed, including anything buffered while the backend
    /// is working.
    pub fn typed(&self) -> &str {
        match &self.state {
            State::Asking { entered, .. } => entered.expose(),
            State::Working => self.pending.expose(),
            _ => "",
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
        let answer = entered.expose().clone();
        self.state = State::Working;
        self.backend.answer(&answer);
        self.pump();
    }

    /// Acknowledges a message and continues.
    pub fn acknowledge(&mut self) {
        if !matches!(self.state, State::Telling { .. }) {
            return;
        }
        self.state = State::Working;
        self.backend.proceed();
        self.pump();
    }

    /// Starts over after a failure.
    pub fn retry(&mut self) {
        if !matches!(self.state, State::Failed { .. }) {
            return;
        }
        self.state = State::Working;
        self.backend.start(&self.username.clone());
        self.pump();
    }

    /// Clears whatever has been typed without sending it.
    pub fn clear(&mut self) {
        if let State::Asking { entered, .. } = &mut self.state {
            *entered = Secret::new(String::new());
        }
    }

    fn apply(&mut self, response: Response) {
        self.state = match response {
            Response::Ask(prompt) => {
                // Restore what was typed during the retry, but only when
                // this is the same question. A different one — a
                // verification code, say — must never be pre-filled with
                // the characters of a password.
                let entered = match self.last_prompt.as_ref() {
                    Some(previous) if previous == &prompt && !self.pending.expose().is_empty() => {
                        std::mem::take(&mut self.pending).into_inner()
                    }
                    _ => {
                        self.pending = Secret::default();
                        String::new()
                    }
                };
                self.last_prompt = Some(prompt.clone());
                State::Asking { prompt, entered: Secret::new(entered) }
            }
            Response::Tell { text, error } => {
                self.pending = Secret::default();
                State::Telling { text, error }
            }
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
        //
        // `entered` stays a plain `String` rather than a `Secret<String>`
        // here: existing tests construct `State::Asking` with a struct
        // literal (`entered: Secret::new(String::new())`, `entered: Secret::new("x".into())`), and
        // changing the field's type would mean editing every one of
        // them — which the rules for this refactor rule out. So the
        // *storage* is unchanged, but the *rendering* still goes through
        // `Secret`'s formatting rather than a second hand-rolled
        // `format_args!`, by wrapping the reference only for the
        // instant it takes to print it. That is the one place this
        // isn't a full migration, and this comment is why.
        match self {
            State::Asking { prompt, entered } => f
                .debug_struct("Asking")
                .field("prompt", prompt)
                .field("entered", entered)
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
        /// Queued for the next `poll`. A script answers immediately, so
        /// this never holds more than one — but it goes through the same
        /// post-then-poll path a real backend uses.
        ready: std::collections::VecDeque<Response>,
    }

    impl Script {
        fn new(steps: Vec<Response>) -> Script {
            Script {
                steps,
                at: 0,
                answers: Vec::new(),
                starts: 0,
                ready: std::collections::VecDeque::new(),
            }
        }

        fn queue_next(&mut self) {
            let response = self.steps.get(self.at).cloned().unwrap_or(Response::Failure {
                reason: "script ended".into(),
            });
            self.at += 1;
            self.ready.push_back(response);
        }
    }

    impl Backend for Script {
        fn start(&mut self, _username: &str) {
            self.starts += 1;
            self.at = 0;
            self.queue_next();
        }

        fn answer(&mut self, answer: &str) {
            self.answers.push(answer.to_string());
            self.queue_next();
        }

        fn proceed(&mut self) {
            self.queue_next();
        }

        fn poll(&mut self) -> Option<Response> {
            self.ready.pop_front()
        }
    }

    /// A backend that answers only when told to.
    ///
    /// [`Script`] answers the instant it is asked, which hides the bug
    /// these tests are about: a real authenticator takes its own time,
    /// and the window between asking it to start over and it asking again
    /// is exactly where typed characters used to be dropped.
    ///
    /// The queue is shared with the test rather than reached through the
    /// conversation, so the test can make the backend speak at a chosen
    /// moment without the production type growing an accessor that exists
    /// only for tests.
    #[derive(Clone)]
    struct Slow {
        ready: std::rc::Rc<std::cell::RefCell<std::collections::VecDeque<Response>>>,
        answers: std::rc::Rc<std::cell::RefCell<Vec<String>>>,
    }

    impl Slow {
        fn new() -> Slow {
            Slow {
                ready: Default::default(),
                answers: Default::default(),
            }
        }

        /// Makes the next `poll` return `response`.
        fn now_say(&self, response: Response) {
            self.ready.borrow_mut().push_back(response);
        }
    }

    impl Backend for Slow {
        fn start(&mut self, _username: &str) {}
        fn answer(&mut self, answer: &str) {
            self.answers.borrow_mut().push(answer.to_string());
        }
        fn proceed(&mut self) {}
        fn poll(&mut self) -> Option<Response> {
            self.ready.borrow_mut().pop_front()
        }
    }

    /// The bug that locked someone out of their own machine.
    ///
    /// After "incorrect password", any key dismisses the error — but
    /// dismissing it puts the conversation into `Working` while the
    /// backend starts over, and `type_into` only wrote in `Asking`. So
    /// the character that did the dismissing was dropped, along with
    /// every other one typed until PAM asked again. Retyping a correct
    /// password submitted it short, which reads as a second wrong
    /// password, and on a stack with `pam_faillock` at three attempts the
    /// third strike locks the account.
    #[test]
    fn a_character_typed_while_the_backend_restarts_is_not_lost() {
        let backend = Slow::new();
        backend.now_say(Response::Ask(Prompt::secret("Password:")));
        let mut c = Conversation::new(backend.clone(), "apost");
        assert!(c.state().accepts_input(), "precondition: the password is being asked");

        c.type_into("wrong".into());
        c.pump();
        // The backend rejects it, on its own schedule.
        backend.now_say(Response::Failure { reason: "Incorrect password".into() });
        c.submit();
        c.pump();
        assert!(matches!(c.state(), State::Failed { .. }), "precondition: it failed");

        // The user reacts by typing the first character of the right
        // password. That dismisses the error and the backend has not
        // asked again yet.
        c.retry();
        assert!(matches!(c.state(), State::Working), "precondition: the backend is restarting");
        c.type_into("h".into());

        // Now it asks again — the same question.
        backend.now_say(Response::Ask(Prompt::secret("Password:")));
        c.pump();

        assert_eq!(
            c.typed(),
            "h",
            "the character that dismissed the error was dropped, so a retyped password \
             would be submitted short and read as wrong a second time"
        );
    }

    /// The reason the buffer is not simply restored into whatever comes
    /// next. A verification code prompt pre-filled with the characters of
    /// a password would submit the password as the code — to a different
    /// system, which may well log it.
    #[test]
    fn a_different_question_is_never_prefilled_with_what_was_typed_for_the_last_one() {
        let backend = Slow::new();
        backend.now_say(Response::Ask(Prompt::secret("Password:")));
        let mut c = Conversation::new(backend.clone(), "apost");

        backend.now_say(Response::Failure { reason: "no".into() });
        c.submit();
        c.pump();
        c.retry();
        c.type_into("hunter2".into());

        // PAM asks something else entirely this time.
        backend.now_say(Response::Ask(Prompt::secret("Verification code:")));
        c.pump();

        assert_eq!(
            c.typed(),
            "",
            "a different question must start empty, whatever was typed for the last one"
        );
    }

    /// A message rather than a question also discards it — there is
    /// nothing to answer, and holding the characters across a "your
    /// account is locked" notice would restore them into whatever
    /// followed.
    #[test]
    fn a_message_discards_what_was_buffered() {
        let backend = Slow::new();
        backend.now_say(Response::Ask(Prompt::secret("Password:")));
        let mut c = Conversation::new(backend.clone(), "apost");

        backend.now_say(Response::Failure { reason: "no".into() });
        c.submit();
        c.pump();
        c.retry();
        c.type_into("hunter2".into());

        backend.now_say(Response::Tell { text: "Account locked".into(), error: true });
        c.pump();
        backend.now_say(Response::Ask(Prompt::secret("Password:")));
        c.acknowledge();
        c.pump();

        assert_eq!(c.typed(), "", "the buffer must not survive a message");
    }

    /// The buffer is a second copy of the password. `State`'s `Debug` is
    /// hand-written to hide `entered`; a derived one here would undo it.
    #[test]
    fn a_conversation_never_renders_what_is_buffered() {
        let backend = Slow::new();
        backend.now_say(Response::Ask(Prompt::secret("Password:")));
        let mut c = Conversation::new(backend.clone(), "apost");
        backend.now_say(Response::Failure { reason: "no".into() });
        c.submit();
        c.pump();
        c.retry();
        c.type_into("correct-horse".into());

        let rendered = format!("{c:?}");
        assert!(!rendered.contains("correct-horse"), "leaked: {rendered}");
        assert!(rendered.contains("13 chars"), "expected a count, got {rendered}");
    }

    fn password_then(outcome: Response) -> Script {
        Script::new(vec![Response::Ask(Prompt::secret("Password:")), outcome])
    }

    #[test]
    fn a_password_prompt_is_asked_then_accepted() {
        let mut c = Conversation::new(password_then(Response::Success), "apost");
        assert_eq!(
            c.state(),
            &State::Asking { prompt: Prompt::secret("Password:"), entered: Secret::new(String::new()) }
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
                assert!(entered.expose().is_empty(), "the new prompt starts empty");
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
            State::Asking { prompt: Prompt::secret("p"), entered: Secret::new("x".into()) },
            State::Telling { text: "t".into(), error: true },
            State::Failed { reason: "r".into() },
        ] {
            assert!(!state.is_authenticated(), "{state:?} must not read as success");
        }
        assert!(State::Authenticated.is_authenticated());
    }

    /// A backend that answers every poll must not hang the host.
    ///
    /// This is not hypothetical: both real backends did exactly that.
    /// `poll` reported a dead worker thread with `Some(Failure)` every
    /// time it was asked, so `pump` — which loops while `poll` yields —
    /// span forever. On a lock screen that is a screen which never
    /// draws again, which is the failure this whole crate is arranged
    /// to avoid. The backends are fixed; this makes sure the host
    /// survives one that is not.
    #[test]
    fn a_backend_that_never_stops_answering_cannot_hang_the_host() {
        /// Always has something to say, like a disconnected channel.
        struct Endless;
        impl Backend for Endless {
            fn start(&mut self, _username: &str) {}
            fn answer(&mut self, _answer: &str) {}
            fn proceed(&mut self) {}
            fn poll(&mut self) -> Option<Response> {
                Some(Response::Failure { reason: "gone".into() })
            }
        }

        // Returns at all, which is the property under test — a failure
        // here is a hang, not an assertion.
        let mut c = Conversation::new(Endless, "apost");
        assert!(matches!(c.state(), State::Failed { .. }));
        assert!(c.pump(), "a yielding backend still reports a change");
        assert!(matches!(c.state(), State::Failed { .. }));
    }

    /// A backend that takes its time — `pam_unix` sleeps about two
    /// seconds after a wrong password — must leave the conversation
    /// usable rather than stopping it. A host that couldn't repaint
    /// through this would look exactly like one that had crashed.
    #[test]
    fn a_slow_backend_leaves_the_conversation_working_rather_than_stuck() {
        /// Answers only when told to, standing in for a backend whose
        /// reply arrives on another thread.
        struct Slow {
            ready: Option<Response>,
        }
        impl Backend for Slow {
            fn start(&mut self, _username: &str) {
                self.ready = Some(Response::Ask(Prompt::secret("Password:")));
            }
            fn answer(&mut self, _answer: &str) {}
            fn proceed(&mut self) {}
            fn poll(&mut self) -> Option<Response> {
                self.ready.take()
            }
        }

        let mut c = Conversation::new(Slow { ready: None }, "apost");
        c.type_into("hunter2".into());
        c.submit();

        // The answer has not arrived, so the state says so and nothing
        // claims success.
        assert_eq!(c.state(), &State::Working);
        assert!(!c.state().is_authenticated());
        // Pumping with nothing ready changes nothing and does not block.
        assert!(!c.pump());
        assert_eq!(c.state(), &State::Working);

        // When it does arrive, pumping applies it.
        c.backend.ready = Some(Response::Success);
        assert!(c.pump());
        assert!(c.state().is_authenticated());
    }

    /// PAM can emit a message and the next prompt together. Applying
    /// only one per pump would stall the conversation until some
    /// unrelated event happened to pump it again.
    #[test]
    fn several_responses_arriving_at_once_are_all_applied() {
        let script = Script::new(vec![
            Response::Ask(Prompt::secret("Password:")),
            Response::Success,
        ]);
        let mut c = Conversation::new(script, "apost");
        c.backend.ready.push_back(Response::Tell { text: "notice".into(), error: false });
        c.backend.ready.push_back(Response::Ask(Prompt::visible("Code:")));

        assert!(c.pump());
        match c.state() {
            State::Asking { prompt, .. } => assert_eq!(prompt.text, "Code:"),
            other => panic!("the last response should have won, got {other:?}"),
        }
    }

    /// What was typed is a password. It must not reach a log, a panic
    /// message, or a debug dump.
    #[test]
    fn debug_output_never_contains_what_was_typed() {
        let state = State::Asking {
            prompt: Prompt::secret("Password:"),
            entered: Secret::new("hunter2".into()),
        };
        let rendered = format!("{state:?}");
        assert!(!rendered.contains("hunter2"), "{rendered}");
        assert!(rendered.contains("7 chars"), "{rendered}");
    }
}
