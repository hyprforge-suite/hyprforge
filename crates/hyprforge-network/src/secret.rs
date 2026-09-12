//! The one type a Wi-Fi passphrase is allowed to live in.
//!
//! A PSK is typed on a keyboard, which puts it under the rule that
//! already governs the lock screen: nothing derived from a keystroke
//! reaches a log, a panic message or a debug dump. The difference here is
//! that a network passphrase passes through more hands than a login
//! password does — it goes into a `HashMap` of D-Bus variants, gets
//! cloned into a settings dictionary, and any `tracing` call or
//! `#[derive(Debug)]` anywhere along that path would print it.
//!
//! So the value is not reachable by accident. There is no `Display`, the
//! `Debug` is hand-written to render a count, and the one accessor is
//! called [`Psk::expose`] so that `grep expose` finds every place the
//! plaintext is actually used.

use std::fmt;

/// A Wi-Fi pre-shared key, on its way to NetworkManager and nowhere else.
#[derive(Clone, PartialEq, Eq)]
pub struct Psk(String);

impl Psk {
    pub fn new(value: impl Into<String>) -> Self {
        Psk(value.into())
    }

    /// The plaintext. Named to be greppable: every caller is a place the
    /// passphrase leaves this type, and there should be exactly one.
    pub fn expose(&self) -> &str {
        &self.0
    }

    /// Characters, not bytes — this is only ever used to describe the
    /// value, never to validate it against a byte length.
    pub fn len_chars(&self) -> usize {
        self.0.chars().count()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Whether this could be a WPA-PSK passphrase at all: 8–63 ASCII
    /// characters, or exactly 64 hex digits for a raw key.
    ///
    /// Checked here so a too-short passphrase is refused by the screen
    /// rather than by NetworkManager, which reports it as a failed
    /// activation several seconds later and looks identical to a wrong
    /// password.
    pub fn is_plausible_wpa(&self) -> bool {
        let raw_key = self.0.len() == 64 && self.0.chars().all(|c| c.is_ascii_hexdigit());
        let passphrase = (8..=63).contains(&self.0.len());
        raw_key || passphrase
    }
}

/// Renders a count, never the value.
///
/// A keysym name *is* the character and a passphrase is no different: the
/// moment this prints `self.0`, every `tracing::debug!(?settings)` in the
/// crate writes a Wi-Fi password to the journal, and in an agent session
/// to the transcript as well.
impl fmt::Debug for Psk {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Psk(<{} chars>)", self.len_chars())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The property the whole type exists for. Written as a test rather
    /// than trusted to review because the failure is silent: a derived
    /// `Debug` would compile, pass every other test, and quietly publish
    /// the passphrase.
    #[test]
    fn a_psk_never_renders_its_own_value() {
        let psk = Psk::new("hunter2-correct-horse");
        let rendered = format!("{psk:?}");
        assert!(
            !rendered.contains("hunter2"),
            "the Debug impl leaked the passphrase: {rendered}"
        );
        assert_eq!(rendered, "Psk(<21 chars>)");
    }

    /// Nesting is where a derived `Debug` would bite — the passphrase is
    /// never printed on its own, it is printed as a field of something
    /// else.
    #[test]
    fn a_psk_nested_in_a_derived_debug_still_renders_a_count() {
        #[derive(Debug)]
        #[allow(dead_code)]
        struct Request {
            ssid: String,
            psk: Psk,
        }
        let rendered = format!(
            "{:?}",
            Request {
                ssid: "home".to_string(),
                psk: Psk::new("swordfish1"),
            }
        );
        assert!(!rendered.contains("swordfish"), "leaked: {rendered}");
        assert!(rendered.contains("<10 chars>"));
    }

    /// Counted in characters, so a non-ASCII passphrase reports what was
    /// typed rather than how many bytes it took.
    #[test]
    fn the_count_is_characters_not_bytes() {
        assert_eq!(Psk::new("pässwörd").len_chars(), 8);
    }

    #[test]
    fn a_passphrase_too_short_for_wpa_is_refused_before_networkmanager_sees_it() {
        assert!(!Psk::new("short").is_plausible_wpa());
        assert!(!Psk::new("").is_plausible_wpa());
        assert!(Psk::new("eightchr").is_plausible_wpa());
        assert!(Psk::new("a".repeat(63)).is_plausible_wpa());
    }

    /// 64 characters is too long to be a passphrase and is a raw key
    /// instead — but only if it is actually hex.
    #[test]
    fn a_sixty_four_character_value_is_a_key_only_when_it_is_hex() {
        assert!(Psk::new("a".repeat(64)).is_plausible_wpa());
        assert!(Psk::new("0123456789abcdef".repeat(4)).is_plausible_wpa());
        assert!(!Psk::new("z".repeat(64)).is_plausible_wpa());
    }
}
