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

/// A value that must never reach a log, a panic message, or a debug
/// dump — only its length may be rendered.
///
/// `T` is almost always a `String`: a typed password, a Wi-Fi
/// passphrase. [`Secret::new`] wraps it, [`Secret::expose`] is the one
/// sanctioned way back out, and `Debug` is implemented once, here,
/// instead of by hand at every call site that holds one.
#[derive(Clone, PartialEq, Eq, Default)]
pub struct Secret<T>(T);

impl<T> Secret<T> {
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
    pub fn into_inner(self) -> T {
        self.0
    }
}

/// How to count a secret's length without looking at what it is made of.
///
/// A password is counted in characters, not bytes, for the same reason
/// [`crate::chars`] is: it is only ever used to describe the value, never
/// to validate it against a byte length, and a non-ASCII password should
/// report what was typed.
pub trait SecretLen {
    fn secret_char_count(&self) -> usize;
}

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
impl<T> From<T> for Secret<T> {
    fn from(value: T) -> Self {
        Secret::new(value)
    }
}

impl<T: SecretLen> fmt::Debug for Secret<T> {
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
