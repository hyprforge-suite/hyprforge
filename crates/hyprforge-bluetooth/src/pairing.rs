//! What pairing asks the user, with no D-Bus in it.
//!
//! Pairing is the one part of Bluetooth that is a *conversation*: BlueZ
//! calls into us and waits, sometimes for as long as a person takes to
//! look at two screens and decide they match. The shapes that
//! conversation can take are here, so a screen can be built and tested
//! against them without a radio.

use crate::types::Address;

/// What to put in front of the user, and what their answer means.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PairingPrompt {
    /// Numeric comparison: both ends show the same six digits and the
    /// user confirms they match. The common case for phones, speakers
    /// and anything with a screen.
    Confirm {
        device: Address,
        name: String,
        passkey: Passkey,
    },
    /// We show the digits and the user types them on the other device —
    /// how a Bluetooth keyboard pairs, since it has keys and no screen.
    /// There is nothing to answer here; it completes on its own.
    Display {
        device: Address,
        name: String,
        passkey: Passkey,
        /// How many digits the far end has taken so far. BlueZ updates
        /// this as they type.
        entered: u16,
    },
    /// "Just Works": no digits at all, because one end has no way to show
    /// or enter them. The user is only saying they meant to do this.
    ///
    /// Worth naming as its own case rather than a `Confirm` with no
    /// passkey: there is genuinely nothing to compare, so a screen must
    /// not invite the user to check something that does not exist.
    Authorize { device: Address, name: String },
}

impl PairingPrompt {
    pub fn device(&self) -> &Address {
        match self {
            PairingPrompt::Confirm { device, .. }
            | PairingPrompt::Display { device, .. }
            | PairingPrompt::Authorize { device, .. } => device,
        }
    }

    pub fn name(&self) -> &str {
        match self {
            PairingPrompt::Confirm { name, .. }
            | PairingPrompt::Display { name, .. }
            | PairingPrompt::Authorize { name, .. } => name,
        }
    }

    /// Whether this prompt is waiting on the user to answer.
    ///
    /// [`PairingPrompt::Display`] is not: it is an instruction, and the
    /// pairing completes when the far end finishes typing. Offering
    /// Accept and Reject buttons there would be offering buttons that do
    /// nothing.
    pub fn needs_an_answer(&self) -> bool {
        !matches!(self, PairingPrompt::Display { .. })
    }
}

/// A six-digit pairing code.
///
/// A type rather than a `u32`, because the spec says it is *always* six
/// digits, zero-padded — and a passkey of `1234` displayed as "1234" next
/// to a device showing "001234" is a user deciding, correctly, that they
/// do not match. Formatting it in one place is the only way that stays
/// true everywhere it is shown.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Passkey(u32);

impl Passkey {
    pub fn new(value: u32) -> Self {
        Passkey(value)
    }

    pub fn as_u32(&self) -> u32 {
        self.0
    }
}

impl std::fmt::Display for Passkey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:06}", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The spec says six digits, zero-padded. A device showing "001234"
    /// beside a screen showing "1234" is a user correctly deciding the
    /// codes do not match and cancelling a pairing that was fine.
    #[test]
    fn a_passkey_is_always_six_digits() {
        assert_eq!(Passkey::new(1234).to_string(), "001234");
        assert_eq!(Passkey::new(0).to_string(), "000000");
        assert_eq!(Passkey::new(999999).to_string(), "999999");
    }

    /// A "Just Works" pairing has nothing to compare. Showing it as a
    /// confirmation invites the user to check a number that does not
    /// exist.
    #[test]
    fn a_just_works_pairing_is_its_own_prompt_not_a_confirm_without_digits() {
        let authorize = PairingPrompt::Authorize {
            device: Address::new("AA:BB:CC:DD:EE:FF"),
            name: "Speaker".to_string(),
        };
        assert!(authorize.needs_an_answer());
        assert!(!matches!(authorize, PairingPrompt::Confirm { .. }));
    }

    /// A displayed passkey is an instruction, not a question — the far
    /// end answers it by typing. Accept and Reject buttons there would do
    /// nothing at all.
    #[test]
    fn a_displayed_passkey_is_not_waiting_on_an_answer_from_this_end() {
        let display = PairingPrompt::Display {
            device: Address::new("AA:BB:CC:DD:EE:FF"),
            name: "Keyboard".to_string(),
            passkey: Passkey::new(42),
            entered: 0,
        };
        assert!(!display.needs_an_answer());
    }

    #[test]
    fn every_prompt_can_name_the_device_it_is_about() {
        let addr = Address::new("AA:BB:CC:DD:EE:FF");
        for prompt in [
            PairingPrompt::Confirm {
                device: addr.clone(),
                name: "Phone".to_string(),
                passkey: Passkey::new(1),
            },
            PairingPrompt::Authorize {
                device: addr.clone(),
                name: "Phone".to_string(),
            },
        ] {
            assert_eq!(prompt.device(), &addr);
            assert_eq!(prompt.name(), "Phone");
        }
    }
}
