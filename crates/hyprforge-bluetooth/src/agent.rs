//! `org.bluez.Agent1`, and its registration with `org.bluez.AgentManager1`.
//!
//! Pairing is the one BlueZ conversation that calls back *into* us:
//! `bluetoothd` holds a method call open — sometimes for as long as a
//! person takes to compare two screens — and our reply decides whether
//! the pairing continues. `RequestConfirmation` and `RequestAuthorization`
//! share one crucial detail: **an empty reply accepts, and an error
//! rejects.** There is no boolean to get backwards, only a return path to
//! remember to take.
//!
//! What this crate calls a decision — what to show, what the answer means
//! — lives in [`crate::pairing::PairingPrompt`], with no D-Bus in it. This
//! module is the marshalling around that: turning a `RequestConfirmation`
//! call into a [`PairingPrompt`], handing it out over a channel, and
//! turning the eventual answer back into the D-Bus reply BlueZ is waiting
//! on.
//!
//! # Why this does not reuse `bluez.rs`'s property-reading helpers
//!
//! `bluez.rs` says as much: its `bounded`, `classify` and `prop` helpers
//! read BlueZ's *whole* device property map for the list screen. They are
//! private to that module, and this module was asked not to touch it.
//! Rather than widen that privacy boundary, the one property this module
//! actually needs — a display name — is read directly over
//! `org.freedesktop.DBus.Properties`, and the address is derived from the
//! object path itself: BlueZ writes it as
//! `/org/bluez/hci0/dev_AA_BB_CC_DD_EE_FF`, so the address never needs a
//! round trip at all, and a device whose properties cannot be read for any
//! reason still gets a prompt.

use crate::pairing::{Passkey, PairingPrompt};
use crate::types::{Address, BluetoothError};
use std::future::Future;
use std::sync::Mutex;
use std::time::Duration;
use tokio::sync::{mpsc, oneshot};
use zbus::zvariant::ObjectPath;
use zbus::Connection;

/// Where this agent is served. Chosen, not assigned by BlueZ — anything
/// under `/org/bluez` is reserved for BlueZ's own objects.
pub const AGENT_PATH: &str = "/dev/hyprforge/BluetoothAgent";

/// `DisplayYesNo`: numeric comparison, Just Works, and a keyboard typing
/// what we display are all covered. `KeyboardOnly`/`KeyboardDisplay` would
/// mean accepting a passkey *typed at this end*, which nothing here does —
/// hence the outright rejections below for the PIN/passkey-entry calls
/// that capability would otherwise invite.
pub const CAPABILITY: &str = "DisplayYesNo";

/// How long a `RequestConfirmation`/`RequestAuthorization`/
/// `AuthorizeService` call waits for a person to answer.
///
/// Matches [`crate::bluez::PAIR_TIMEOUT`] deliberately: that is the budget
/// `bluez::pair()` already gives the *whole* conversation from our own
/// calling side, so waiting any longer here would mean bluetoothd is still
/// held open after the code that started the pairing has already given up
/// on it.
pub const ANSWER_TIMEOUT: Duration = crate::bluez::PAIR_TIMEOUT;

#[zbus::proxy(
    interface = "org.bluez.AgentManager1",
    default_service = "org.bluez",
    default_path = "/org/bluez"
)]
trait AgentManagerDbus {
    fn register_agent(&self, agent: &ObjectPath<'_>, capability: &str) -> zbus::Result<()>;
    fn unregister_agent(&self, agent: &ObjectPath<'_>) -> zbus::Result<()>;
}

/// Reading exactly one property off a device, rather than the whole map
/// `bluez.rs` fetches for the list screen — this module only ever wants a
/// name.
#[zbus::proxy(interface = "org.freedesktop.DBus.Properties", default_service = "org.bluez")]
trait PropertiesDbus {
    #[zbus(name = "Get")]
    fn get_property(&self, interface: &str, property: &str) -> zbus::Result<zbus::zvariant::OwnedValue>;
}

/// Bounds a call and turns a D-Bus failure into something this module's
/// callers can report.
///
/// A smaller cousin of `bluez::bounded` — that one also classifies
/// *which* failure it was (service missing vs. refused), which register
/// and unregister do not need: a machine with no BlueZ was already ruled
/// out by whatever opened the connection this module is handed.
async fn bounded<T>(
    limit: Duration,
    call: impl Future<Output = zbus::Result<T>>,
) -> Result<T, BluetoothError> {
    match tokio::time::timeout(limit, call).await {
        Err(_elapsed) => Err(BluetoothError::TimedOut(limit)),
        Ok(Ok(value)) => Ok(value),
        Ok(Err(e)) => Err(BluetoothError::Refused(e.to_string())),
    }
}

fn agent_path() -> ObjectPath<'static> {
    ObjectPath::try_from(AGENT_PATH).expect("AGENT_PATH is a valid object path, checked by a test")
}

/// The address half of a `Device1` object path.
///
/// BlueZ derives the path from the address itself
/// (`/org/bluez/hci0/dev_AA_BB_CC_DD_EE_FF`), so recovering it needs no
/// D-Bus call and therefore cannot fail the way a property read can — a
/// device this deep into a pairing conversation is, by definition, one
/// BlueZ already gave us a path for.
fn address_from_path(path: &ObjectPath<'_>) -> Address {
    let segment = path.as_str().rsplit('/').next().unwrap_or("");
    let hex = segment.strip_prefix("dev_").unwrap_or(segment);
    Address::new(hex.replace('_', ":"))
}

/// What to call a device in a prompt: its resolved name, or its address
/// when nothing could be read.
///
/// A pure function on purpose — the fallback is the property this module
/// exists to guarantee ("a device with no readable name still pairs"),
/// and it should be checkable without a bus to fail to answer on.
fn display_name(resolved: Option<String>, fallback: &Address) -> String {
    resolved.unwrap_or_else(|| fallback.to_string())
}

/// What a pending `RequestConfirmation`/`RequestAuthorization`/
/// `AuthorizeService` call is asking, and the one-shot way to answer it.
///
/// The [`oneshot::Sender<bool>`] is not exposed directly: reading `true`
/// as reject and `false` as accept is exactly the kind of thing that is
/// obvious in review and wrong at 2am, so [`PairingRequest::accept`] and
/// [`PairingRequest::reject`] are the only way to resolve one.
pub struct PairingRequest {
    pub prompt: PairingPrompt,
    answer: oneshot::Sender<bool>,
}

impl PairingRequest {
    fn new(prompt: PairingPrompt, answer: oneshot::Sender<bool>) -> Self {
        PairingRequest { prompt, answer }
    }

    /// Confirms the pairing. BlueZ sees an empty, successful reply.
    pub fn accept(self) {
        let _ = self.answer.send(true);
    }

    /// Refuses the pairing. BlueZ sees `org.bluez.Error.Rejected`.
    pub fn reject(self) {
        let _ = self.answer.send(false);
    }
}

/// `org.bluez.Error.*`, for the accept/reject calls.
///
/// `Rejected` is the user saying no; `Canceled` is everything that is not
/// a person answering — bluetoothd calling `Cancel()`, the answer timing
/// out, or nobody listening on the other end of the channel at all. BlueZ
/// treats both as "no", but a UI reading this back (or a person debugging
/// a log) should not be told a request was refused when it was actually
/// abandoned.
#[derive(Debug, zbus::DBusError)]
#[zbus(prefix = "org.bluez.Error")]
pub enum AgentError {
    Rejected(String),
    Canceled(String),
}

/// Hands [`PairingPrompt`]s out on a channel and waits for the answer,
/// independent of D-Bus entirely — so the property this module exists to
/// guarantee (timeout and "nobody is listening" both reject, never
/// accept) can be tested without a bus to talk to.
struct Prompter {
    requests: mpsc::UnboundedSender<PairingRequest>,
    /// The cancellation half of whichever request is currently
    /// outstanding, if any.
    ///
    /// A plain [`Mutex`], not a `tokio::sync::Mutex`: it is only ever held
    /// long enough to swap a value in or out, never across an `.await` —
    /// which matters here specifically, because `Cancel()` is dispatched
    /// by zbus *concurrently* with whichever `ask` call is in flight, and
    /// a lock held across that call's await would make `Cancel()` wait
    /// for the very thing it exists to interrupt.
    cancel: Mutex<Option<oneshot::Sender<()>>>,
}

impl Prompter {
    fn new(requests: mpsc::UnboundedSender<PairingRequest>) -> Self {
        Prompter {
            requests,
            cancel: Mutex::new(None),
        }
    }

    /// Sends `prompt` out and waits for an answer, a cancellation, or the
    /// timeout — whichever comes first.
    async fn ask(&self, prompt: PairingPrompt) -> Result<(), AgentError> {
        let (answer_tx, answer_rx) = oneshot::channel();
        let (cancel_tx, cancel_rx) = oneshot::channel();
        *self.cancel.lock().unwrap() = Some(cancel_tx);

        // Nobody is listening for prompts at all — reject immediately
        // rather than waiting out `ANSWER_TIMEOUT` for an answer that
        // structurally cannot arrive. A UI that starts later will simply
        // not see this request, which is correct: it was not running when
        // BlueZ asked.
        if self.requests.send(PairingRequest::new(prompt, answer_tx)).is_err() {
            *self.cancel.lock().unwrap() = None;
            return Err(AgentError::Canceled(
                "no pairing UI is listening for prompts".to_string(),
            ));
        }

        let accepted = tokio::select! {
            biased;
            _ = cancel_rx => None,
            outcome = tokio::time::timeout(ANSWER_TIMEOUT, answer_rx) => {
                match outcome {
                    // A real answer.
                    Ok(Ok(accepted)) => Some(accepted),
                    // The `PairingRequest` was dropped without being
                    // answered — the UI that received it went away.
                    Ok(Err(_)) => None,
                    // Nobody answered in time.
                    Err(_elapsed) => None,
                }
            }
        };
        *self.cancel.lock().unwrap() = None;

        match accepted {
            Some(true) => Ok(()),
            Some(false) => Err(AgentError::Rejected("the user declined".to_string())),
            None => Err(AgentError::Canceled(
                "the request was cancelled, dropped, or timed out".to_string(),
            )),
        }
    }

    /// Interrupts whichever `ask` call is currently outstanding, if any.
    ///
    /// Only one request is tracked at a time: BlueZ serialises pairing
    /// per adapter in practice, and `Cancel()` itself carries no device to
    /// disambiguate with, so "the most recent outstanding request" is the
    /// only answer this method could give even if more than one existed.
    fn cancel_pending(&self) {
        if let Some(sender) = self.cancel.lock().unwrap().take() {
            let _ = sender.send(());
        }
    }
}

/// The `org.bluez.Agent1` object itself.
pub struct PairingAgent {
    connection: Connection,
    prompter: Prompter,
}

impl PairingAgent {
    fn new(connection: Connection, requests: mpsc::UnboundedSender<PairingRequest>) -> Self {
        PairingAgent {
            connection,
            prompter: Prompter::new(requests),
        }
    }

    /// `Alias`, falling back to `Name`, over `Properties.Get` — the same
    /// fallback order BlueZ itself uses for `Alias`, read here rather than
    /// through `bluez.rs` because that module's helper for it is private.
    /// Either property being unreadable is not an error: pairing still
    /// works with only an address to show.
    async fn resolve(&self, device: &ObjectPath<'_>) -> (Address, String) {
        let address = address_from_path(device);

        // `device` borrows from the incoming method call; the builder
        // needs a path it can own, hence `OwnedObjectPath` rather than
        // passing the borrow straight through.
        let owned_path: zbus::zvariant::OwnedObjectPath = device.clone().into();
        let built = match PropertiesDbusProxy::builder(&self.connection).path(owned_path) {
            Ok(builder) => builder.build().await,
            Err(e) => Err(e),
        };
        let Ok(proxy) = built else {
            return (address.clone(), display_name(None, &address));
        };

        for property in ["Alias", "Name"] {
            if let Ok(value) = bounded(
                crate::bluez::TIMEOUT,
                proxy.get_property("org.bluez.Device1", property),
            )
            .await
            {
                if let Ok(name) = String::try_from(value) {
                    return (address.clone(), display_name(Some(name), &address));
                }
            }
        }
        (address.clone(), display_name(None, &address))
    }
}

#[zbus::interface(name = "org.bluez.Agent1")]
impl PairingAgent {
    /// bluetoothd is done with this agent. Any request still outstanding
    /// cannot be answered by a person any more, so it is withdrawn the
    /// same way `Cancel()` withdraws one.
    async fn release(&self) {
        self.prompter.cancel_pending();
    }

    async fn request_confirmation(&self, device: ObjectPath<'_>, passkey: u32) -> Result<(), AgentError> {
        let (address, name) = self.resolve(&device).await;
        self.prompter
            .ask(PairingPrompt::Confirm {
                device: address,
                name,
                passkey: Passkey::new(passkey),
            })
            .await
    }

    async fn request_authorization(&self, device: ObjectPath<'_>) -> Result<(), AgentError> {
        let (address, name) = self.resolve(&device).await;
        self.prompter
            .ask(PairingPrompt::Authorize { device: address, name })
            .await
    }

    /// Informational: BlueZ does not wait on an answer here, and neither
    /// do we — [`PairingPrompt::needs_an_answer`] says as much for this
    /// case. The prompt is still handed out, because a screen must show
    /// the digits somewhere, and repeated calls as `entered` climbs are
    /// exactly BlueZ letting that screen update.
    async fn display_passkey(&self, device: ObjectPath<'_>, passkey: u32, entered: u16) {
        let (address, name) = self.resolve(&device).await;
        let _ = self.prompter.requests.send(PairingRequest::new(
            PairingPrompt::Display {
                device: address,
                name,
                passkey: Passkey::new(passkey),
                entered,
            },
            oneshot::channel().0,
        ));
    }

    async fn authorize_service(&self, device: ObjectPath<'_>, _uuid: String) -> Result<(), AgentError> {
        let (address, name) = self.resolve(&device).await;
        self.prompter
            .ask(PairingPrompt::Authorize { device: address, name })
            .await
    }

    /// bluetoothd abandoned the request. Whatever prompt is on screen has
    /// to come down with it — an accept or reject reaching BlueZ after
    /// this point would answer a conversation that is already over.
    async fn cancel(&self) {
        self.prompter.cancel_pending();
    }

    /// Legacy PIN-code pairing. Rejected outright: entering one means
    /// `KeyboardOnly`/`KeyboardDisplay` capability, which this agent does
    /// not register as — see [`CAPABILITY`].
    async fn request_pin_code(&self, _device: ObjectPath<'_>) -> Result<String, AgentError> {
        Err(AgentError::Rejected(
            "legacy PIN-code pairing isn't supported".to_string(),
        ))
    }

    /// Typed passkey entry at this end. Same reasoning as
    /// [`Self::request_pin_code`].
    async fn request_passkey(&self, _device: ObjectPath<'_>) -> Result<u32, AgentError> {
        Err(AgentError::Rejected(
            "typed passkey entry isn't supported".to_string(),
        ))
    }

    /// Same reasoning as [`Self::request_pin_code`]: nothing here accepts
    /// a legacy PIN.
    async fn display_pin_code(&self, _device: ObjectPath<'_>, _pincode: String) -> Result<(), AgentError> {
        Err(AgentError::Rejected(
            "legacy PIN-code pairing isn't supported".to_string(),
        ))
    }
}

/// A registered [`PairingAgent`]. Dropping it unregisters where it can —
/// see the note on [`Drop`] below for the one case it cannot.
pub struct AgentHandle {
    connection: Connection,
    unregistered: bool,
}

impl AgentHandle {
    /// Unregisters explicitly, reporting a failure rather than swallowing
    /// it the way [`Drop`] has to.
    pub async fn unregister(mut self) -> Result<(), BluetoothError> {
        let manager = bounded(crate::bluez::TIMEOUT, AgentManagerDbusProxy::new(&self.connection)).await?;
        bounded(crate::bluez::TIMEOUT, manager.unregister_agent(&agent_path())).await?;
        let _ = self
            .connection
            .object_server()
            .remove::<PairingAgent, _>(AGENT_PATH)
            .await;
        self.unregistered = true;
        Ok(())
    }
}

impl Drop for AgentHandle {
    /// Best-effort only: `Drop` cannot `.await`, so this spawns the
    /// unregister call on whatever tokio runtime is current and does
    /// nothing when there is none.
    ///
    /// That is not a leak. BlueZ documents that an agent is dropped
    /// automatically when its owner disappears from the bus — the same
    /// property `hyprforge-tray`'s icons rely on — so a process exit with
    /// no runtime left to spawn on still clears the registration, just a
    /// little later, from BlueZ's side rather than ours.
    fn drop(&mut self) {
        if self.unregistered {
            return;
        }
        let connection = self.connection.clone();
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            handle.spawn(async move {
                if let Ok(manager) = AgentManagerDbusProxy::new(&connection).await {
                    let _ = manager.unregister_agent(&agent_path()).await;
                }
                let _ = connection
                    .object_server()
                    .remove::<PairingAgent, _>(AGENT_PATH)
                    .await;
            });
        }
    }
}

/// Serves a [`PairingAgent`] on `connection` and registers it with
/// `org.bluez.AgentManager1`.
///
/// Returns the [`mpsc::UnboundedReceiver`] of [`PairingRequest`]s
/// alongside the [`AgentHandle`], since a registered agent that hands its
/// prompts nowhere is not useful to a caller — the two are only apart in
/// their types, not in their lifetime.
pub async fn register(
    connection: &Connection,
) -> Result<(AgentHandle, mpsc::UnboundedReceiver<PairingRequest>), BluetoothError> {
    let (tx, rx) = mpsc::unbounded_channel();
    let agent = PairingAgent::new(connection.clone(), tx);

    bounded(crate::bluez::TIMEOUT, connection.object_server().at(AGENT_PATH, agent)).await?;

    let manager = bounded(crate::bluez::TIMEOUT, AgentManagerDbusProxy::new(connection)).await?;
    if let Err(e) = bounded(
        crate::bluez::TIMEOUT,
        manager.register_agent(&agent_path(), CAPABILITY),
    )
    .await
    {
        // The object is already served; undo that before reporting
        // failure, so a caller that gives up on `register` is not left
        // with a half-registered agent sitting on the bus.
        let _ = connection.object_server().remove::<PairingAgent, _>(AGENT_PATH).await;
        return Err(e);
    }

    // `RequestDefaultAgent` is deliberately **not** called.
    //
    // BlueZ's own documentation settles it: "every application can
    // register its own agent and for all actions triggered by that
    // application its agent is used". Hyprforge initiates its own
    // pairings, so this agent is already the one BlueZ asks — being
    // default buys nothing for that.
    //
    // What it would cost is somebody else's. The default agent handles
    // pairings *this* process did not start, including ones a remote
    // device initiates, and there is generally already an agent holding
    // it — blueman-applet on this machine. Taking it means Hyprforge
    // silently answers for pairings the user started somewhere else, and
    // stops answering the moment the Settings app is closed. A settings
    // screen should not quietly become the system's pairing handler for
    // as long as it happens to be open.
    //
    // If a Hyprforge session ever wants to own pairing system-wide, that
    // is a deliberate choice for a long-lived process to make, not a
    // side effect of opening a screen.
    //
    // `RequestDefaultAgent` is not on the proxy above at all, so this is
    // enforced by the compiler rather than by remembering. A test that
    // grepped this file for the call was the first attempt and was
    // worse: it matched its own source line.

    Ok((
        AgentHandle {
            connection: connection.clone(),
            unregistered: false,
        },
        rx,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn address() -> Address {
        Address::new("AA:BB:CC:DD:EE:FF")
    }

    fn confirm_prompt() -> PairingPrompt {
        PairingPrompt::Confirm {
            device: address(),
            name: "Headset".to_string(),
            passkey: Passkey::new(123456),
        }
    }

    /// The one detail this whole module exists to get right: reading the
    /// bool backwards turns a decline into an acceptance. `accept`/
    /// `reject` are the only way to resolve a request specifically so
    /// this cannot be gotten wrong at the call site.
    #[tokio::test]
    async fn accepting_resolves_to_true_and_rejecting_resolves_to_false() {
        let (answer_tx, answer_rx) = oneshot::channel();
        PairingRequest::new(confirm_prompt(), answer_tx).accept();
        assert_eq!(answer_rx.await, Ok(true));

        let (answer_tx, answer_rx) = oneshot::channel();
        PairingRequest::new(confirm_prompt(), answer_tx).reject();
        assert_eq!(answer_rx.await, Ok(false));
    }

    /// No UI is listening at all — the receiver of `PairingRequest`s was
    /// dropped. This must fail immediately, not sit out `ANSWER_TIMEOUT`
    /// for an answer that cannot possibly come.
    #[tokio::test(start_paused = true)]
    async fn a_request_with_no_receiver_rejects_immediately_rather_than_waiting_out_the_timeout() {
        let (tx, rx) = mpsc::unbounded_channel();
        drop(rx);
        let prompter = Prompter::new(tx);

        let before = tokio::time::Instant::now();
        let result = prompter.ask(confirm_prompt()).await;
        let elapsed = before.elapsed();

        assert!(matches!(result, Err(AgentError::Canceled(_))));
        assert!(
            elapsed < Duration::from_millis(50),
            "took {elapsed:?}, which means it waited instead of failing fast"
        );
    }

    /// The one bug that would actually be dangerous: a timeout that
    /// defaults to *accepting* would pair with anything that asked a
    /// question while nobody was at the screen to answer it.
    #[tokio::test(start_paused = true)]
    async fn a_timeout_rejects_rather_than_accepting() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let prompter = Prompter::new(tx);

        let ask = tokio::spawn(async move { prompter.ask(confirm_prompt()).await });

        // Receive the request and hold onto it without ever calling
        // `accept` or `reject` — this is the "nobody answered" case, not
        // the "nobody is listening at all" case above.
        let request = rx.recv().await.expect("the request was sent");

        tokio::time::advance(ANSWER_TIMEOUT + Duration::from_secs(1)).await;

        let result = ask.await.expect("the ask task did not panic");
        assert!(
            matches!(result, Err(AgentError::Canceled(_))),
            "a timeout must reject, never accept: {result:?}"
        );
        // Keep the request alive until here, so its drop is not what
        // resolves the `oneshot` — the timeout is what this test is
        // checking.
        drop(request);
    }

    /// `Cancel()` must be able to interrupt a request that is already
    /// waiting on an answer, and must not need to wait for that request's
    /// own timeout to do it.
    #[tokio::test(start_paused = true)]
    async fn cancelling_a_pending_request_rejects_it_without_waiting_for_the_timeout() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let prompter = std::sync::Arc::new(Prompter::new(tx));
        let asker = prompter.clone();

        let ask = tokio::spawn(async move { asker.ask(confirm_prompt()).await });
        let request = rx.recv().await.expect("the request was sent");

        prompter.cancel_pending();

        let result = ask.await.expect("the ask task did not panic");
        assert!(matches!(result, Err(AgentError::Canceled(_))));
        drop(request);
    }

    /// `DisplayYesNo` is the one capability this agent registers with —
    /// anything else would claim it can accept typed passkey entry, which
    /// it does not.
    #[test]
    fn the_registered_capability_is_exactly_display_yes_no() {
        assert_eq!(CAPABILITY, "DisplayYesNo");
    }

    /// The property this whole resolution path exists to guarantee: a
    /// device whose name could not be read still gets a prompt, and that
    /// prompt names it by address rather than showing nothing.
    #[test]
    fn a_device_with_no_readable_name_is_named_by_its_address_instead() {
        let addr = address();
        assert_eq!(display_name(None, &addr), addr.to_string());
        assert_eq!(display_name(Some("Real Name".to_string()), &addr), "Real Name");
    }

    /// The address is recovered from the object path with no D-Bus call —
    /// this is the mechanical check that the encoding BlueZ documents
    /// (`dev_AA_BB_CC_DD_EE_FF`, underscores for colons) is decoded the
    /// same way it was written.
    #[test]
    fn the_device_address_is_recovered_from_its_object_path() {
        let path = ObjectPath::try_from("/org/bluez/hci0/dev_AA_BB_CC_DD_EE_FF").unwrap();
        assert_eq!(address_from_path(&path).as_str(), "AA:BB:CC:DD:EE:FF");
    }

    /// `AGENT_PATH` is written by hand above; if it were ever mistyped
    /// into something D-Bus rejects, every `register` call would fail
    /// with an error that says nothing about why.
    #[test]
    fn the_agent_path_constant_is_a_valid_object_path() {
        assert!(ObjectPath::try_from(AGENT_PATH).is_ok());
    }
}
