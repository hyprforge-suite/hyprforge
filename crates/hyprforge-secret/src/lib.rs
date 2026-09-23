//! One implementation of "never render this".
//!
//! A keysym name *is* the character it names, and the same is true of a
//! typed password and a Wi-Fi passphrase: the moment a `Debug` impl, a
//! `tracing::debug!(?x)`, or a panic message prints the value, it is on
//! disk in a barely-encoded form, and in an agent session it reaches the
//! session transcript through tool output. That rule got written by hand
//! three separate times in this workspace — the lock screen's typed
//! password, the Wi-Fi passphrase, and the clipboard's remembered content
//! — and each of those was a fresh chance to get it wrong: to forget a
//! field, to `#[derive(Debug)]` over it later, to print the value "just
//! this once" for a bug report.
//!
//! [`Secret<T>`] is the one implementation for the two of those that are
//! a genuine "hide this, entirely" case — a password and a passphrase,
//! where the *value* itself is the whole risk and nothing about it is
//! worth describing. Its `Debug` renders a character count and nothing
//! else, and [`Secret::expose`] is the single accessor: `grep expose` in
//! any crate that depends on this one finds every place a secret leaves
//! the wrapper.
//!
//! The clipboard is a different shape of the same problem and is
//! deliberately *not* built on `Secret<T>` — see
//! `hyprforge-clipboard`'s `types.rs` for why, and [`chars`]/[`bytes`]
//! for the smaller piece of this crate it does share: the formatting of
//! "a redacted count", used by a `Debug` impl that describes rather than
//! hides.

use std::fmt;
use zeroize::Zeroize;

/// A value that must never reach a log, a panic message, or a debug
/// dump — only its length may be rendered.
///
/// `T` is almost always a `String`: a typed password, a Wi-Fi
/// passphrase. [`Secret::new`] wraps it, [`Secret::expose`] is the one
/// sanctioned way back out, and `Debug` is implemented once, here,
/// instead of by hand at every call site that holds one.
#[derive(Clone, PartialEq, Eq, Default)]
pub struct Secret<T: Zeroize>(T);

/// Erased when it goes out of scope.
///
/// This is the half `Debug` cannot do. Hiding the value from a log stops
/// it being *written down*; it does nothing about the copy sitting in
/// the process's heap until that page is reused, which a core dump or a
/// swapped-out page can still carry.
///
/// It matters more here than it looks, because of how the value is
/// accumulated. A typed password is not appended to in place — the host
/// hands the whole string over on every keystroke and the old `Secret`
/// is dropped — so entering eight characters allocates eight strings,
/// each holding a prefix of the password, and before this each of them
/// was freed with its contents intact. Zeroing on drop clears every one
/// of them at the moment it stops being used.
///
/// What it still does not cover, and the README says so: PAM and greetd
/// keep copies of their own once the answer is handed over, and nothing
/// on this side can reach those.
impl<T: Zeroize> Drop for Secret<T> {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

impl<T: Zeroize> Secret<T> {
    /// Wraps a value so it can only leave through [`Secret::expose`] or
    /// [`Secret::into_inner`]. Always safe: wrapping does not expose
    /// anything, which is why `Secret<T>: From<T>` also exists as a
    /// shorthand for this.
    pub fn new(value: T) -> Self {
        Secret(value)
    }

    /// The value. Named to be greppable: every caller is a place the
    /// secret leaves this type, and there should be exactly one per use
    /// site — the place it is actually sent to PAM, greetd, or
    /// NetworkManager.
    pub fn expose(&self) -> &T {
        &self.0
    }

    /// Unwraps by value, for the rare case a secret needs to move
    /// (`std::mem::take` and hand it to something that owns it next).
    /// Still not a way to see the value — the caller has to `expose()`
    /// it separately, same as anyone else.
    /// Requires `Default` because this type erases itself on drop, and
    /// Rust will not let a field move out of something with a `Drop`
    /// impl — so the value is swapped for a default one, which is then
    /// what gets erased. The caller owns the real value afterwards and
    /// owns the job of clearing it, exactly as they would have anyway.
    pub fn into_inner(mut self) -> T
    where
        T: Default,
    {
        std::mem::take(&mut self.0)
    }
}

/// How to count a secret's length without looking at what it is made of.
///
/// A password is counted in characters, not bytes, for the same reason
/// [`crate::chars`] is: it is only ever used to describe the value, never
/// to validate it against a byte length, and a non-ASCII password should
/// report what was typed.
///
/// Sealed: this exists to let `Secret<T>`'s `Debug` count characters
/// without caring what `T` is made of, not as an extension point for
/// callers outside this crate. A public trait with one method is a
/// promise about its *whole* shape — adding a second method later would
/// break every external implementer — and sealing is what keeps that
/// door shut while still letting `SecretLen` appear in a public bound
/// (`Secret<T>`'s `Debug` impl requires it).
pub trait SecretLen: sealed::Sealed {
    /// How many characters this value would render as, if it were ever
    /// allowed to render at all.
    fn secret_char_count(&self) -> usize;
}

mod sealed {
    pub trait Sealed {}
}

impl sealed::Sealed for String {}
impl sealed::Sealed for str {}
impl<T: sealed::Sealed + ?Sized> sealed::Sealed for &T {}

impl SecretLen for String {
    fn secret_char_count(&self) -> usize {
        self.chars().count()
    }
}

impl SecretLen for str {
    fn secret_char_count(&self) -> usize {
        self.chars().count()
    }
}

impl<T: SecretLen + ?Sized> SecretLen for &T {
    fn secret_char_count(&self) -> usize {
        (**self).secret_char_count()
    }
}

/// Renders `<N chars>`, never the value — the whole point of the type.
///
/// This is the one place that decision is made. Every caller that holds
/// a `Secret<T>`, including one nested inside a `#[derive(Debug)]`
/// struct, gets this rendering for free and cannot accidentally derive
/// past it.
/// Wrapping is always safe, so `.into()` is allowed rather than making
/// every call site spell out `Secret::new`.
///
/// Deliberately one-directional: there is no `From<Secret<T>> for T`,
/// because taking a value back *out* is the operation that has to stay
/// greppable, and that is what [`Secret::expose`] is for.
impl<T: Zeroize> From<T> for Secret<T> {
    fn from(value: T) -> Self {
        Secret::new(value)
    }
}

impl<T: SecretLen + Zeroize> fmt::Debug for Secret<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", chars(self.0.secret_char_count()))
    }
}

/// A count with a unit, rendered as `<N unit>` — `<28 chars>`,
/// `<2048 bytes>`.
///
/// This is the shared piece for a type that *describes* its content
/// rather than hiding it entirely (`hyprforge-clipboard`'s `Content`,
/// which says `Text(<28 chars>)` or `Image(image/png, <2048 bytes>)`
/// rather than nothing at all). [`Secret<T>`]'s own `Debug` is built out
/// of the same formatting, so there is exactly one implementation of "a
/// redacted count looks like this" in the workspace, even though hiding
/// entirely and describing-with-a-count are different policies.
pub struct RedactedCount {
    count: usize,
    unit: &'static str,
}

impl fmt::Debug for RedactedCount {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "<{} {}>", self.count, self.unit)
    }
}

/// A character count, for text: `<N chars>`.
pub fn chars(count: usize) -> RedactedCount {
    RedactedCount { count, unit: "chars" }
}

/// A byte count, for anything not meaningfully measured in characters
/// (image bytes, for instance): `<N bytes>`.
pub fn bytes(count: usize) -> RedactedCount {
    RedactedCount { count, unit: "bytes" }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_secret_never_renders_its_own_value() {
        let secret = Secret::new("hunter2-correct-horse".to_string());
        let rendered = format!("{secret:?}");
        assert!(!rendered.contains("hunter2"), "leaked: {rendered}");
        assert_eq!(rendered, "<21 chars>");
    }

    /// The case that actually bites: a secret is never printed on its
    /// own, it is printed as a field of something else, usually a
    /// `#[derive(Debug)]` struct that was written without thinking about
    /// what it holds.
    #[test]
    fn a_secret_nested_inside_a_derived_debug_still_renders_only_a_count() {
        #[derive(Debug)]
        #[allow(dead_code)]
        struct Request {
            username: String,
            password: Secret<String>,
        }
        let rendered = format!(
            "{:?}",
            Request {
                username: "apost".to_string(),
                password: Secret::new("swordfish1".to_string()),
            }
        );
        assert!(!rendered.contains("swordfish"), "leaked: {rendered}");
        assert!(rendered.contains("<10 chars>"), "got {rendered}");
    }

    #[test]
    fn the_count_is_characters_not_bytes() {
        let secret = Secret::new("pässwörd".to_string());
        assert_eq!(format!("{secret:?}"), "<8 chars>");
    }

    #[test]
    fn expose_is_the_only_way_back_to_the_value() {
        let secret = Secret::new("hunter2".to_string());
        assert_eq!(secret.expose(), "hunter2");
    }

    #[test]
    fn a_redacted_count_renders_a_count_and_a_unit_never_a_value() {
        assert_eq!(format!("{:?}", chars(28)), "<28 chars>");
        assert_eq!(format!("{:?}", bytes(2048)), "<2048 bytes>");
    }
}

#[cfg(test)]
mod erasing {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Counts how many times it was erased, so the *wiring* can be
    /// tested without reading freed memory.
    ///
    /// Reading the heap after a free to prove the bytes are gone is
    /// undefined behaviour, and a test built on it reports whatever the
    /// allocator happened to do — which is the sort of measurement
    /// CLAUDE.md's "check the instrument" rule is about. What this crate
    /// can actually get wrong is the wiring: forgetting the `Drop`,
    /// bounding it so it silently does not apply, or moving the value
    /// out before it runs. That is what these check. Whether the write
    /// survives the optimiser is `zeroize`'s own guarantee and its own
    /// test suite, which is the reason for depending on it rather than
    /// writing the loop here.
    /// `Option`, so the empty husk `into_inner` leaves behind counts
    /// against nothing and erasing it cannot be mistaken for erasing
    /// the value that was taken out. That is also what `Default` gives
    /// for free, which is why it is derived.
    #[derive(Clone, Default)]
    struct Counted(Option<&'static AtomicUsize>);

    impl Zeroize for Counted {
        fn zeroize(&mut self) {
            if let Some(counter) = self.0 {
                counter.fetch_add(1, Ordering::SeqCst);
            }
        }
    }

    static DROPPED: AtomicUsize = AtomicUsize::new(0);
    static TAKEN: AtomicUsize = AtomicUsize::new(0);

    #[test]
    fn a_secret_erases_itself_when_it_goes_out_of_scope() {
        DROPPED.store(0, Ordering::SeqCst);
        {
            let _held = Secret::new(Counted(Some(&DROPPED)));
            assert_eq!(DROPPED.load(Ordering::SeqCst), 0, "not while it is still in use");
        }
        assert_eq!(DROPPED.load(Ordering::SeqCst), 1);
    }

    /// The case that makes this worth having. A typed password is
    /// replaced wholesale on every keystroke rather than appended to, so
    /// each intermediate has to be erased as it is displaced — eight
    /// characters, eight strings, each holding a prefix.
    #[test]
    fn every_displaced_value_is_erased_not_only_the_last() {
        DROPPED.store(0, Ordering::SeqCst);
        let mut entered = Secret::new(Counted(Some(&DROPPED)));
        for _ in 0..8 {
            entered = Secret::new(Counted(Some(&DROPPED)));
        }
        assert_eq!(DROPPED.load(Ordering::SeqCst), 8, "one per displaced value");
        drop(entered);
        assert_eq!(DROPPED.load(Ordering::SeqCst), 9);
    }

    /// `into_inner` hands the value to the caller, so what is erased is
    /// the default left behind — not the value, which the caller now
    /// owns and is responsible for.
    #[test]
    fn taking_the_value_out_does_not_erase_the_value_that_was_taken() {
        TAKEN.store(0, Ordering::SeqCst);
        let held = Secret::new(Counted(Some(&TAKEN)));
        let out = held.into_inner();
        // Nothing was counted: what the wrapper erased was the empty
        // husk left in its place, and the real value went to the
        // caller untouched.
        assert_eq!(TAKEN.load(Ordering::SeqCst), 0, "the value that was taken must not be erased");
        // The real value came out, and nothing erased it on the way —
        // it is the caller's now, and clearing it is theirs too.
        assert!(out.0.is_some(), "and it is the real one");
    }

    /// A real password, through the type the auth stack actually uses —
    /// so a change that made `String` stop satisfying the bound would
    /// fail here rather than somewhere far away.
    #[test]
    fn a_string_password_is_what_this_is_for() {
        let password = Secret::new("hunter2".to_string());
        assert_eq!(password.expose(), "hunter2");
        assert!(!format!("{password:?}").contains("hunter2"));
    }
}
