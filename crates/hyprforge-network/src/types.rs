//! What a network looks like to this crate, independent of D-Bus.
//!
//! Kept free of `zbus` types so the model can be built and asserted on
//! without a bus, which is what makes the mock backend worth having.

use std::fmt;

/// A network name, as bytes.
///
/// 802.11 says an SSID is up to 32 **octets**, not characters, and
/// NetworkManager reports it as `ay` for exactly that reason: it is
/// allowed to be invalid UTF-8, and some access points genuinely are.
/// Storing it as a `String` would mean deciding what a non-UTF-8 name
/// becomes at the point it is read, and then never being able to connect
/// to it, because the bytes handed back would no longer be the bytes that
/// came in.
#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Ssid(Vec<u8>);

impl Ssid {
    pub fn new(bytes: impl Into<Vec<u8>>) -> Self {
        Ssid(bytes.into())
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    /// Empty is how a hidden network reports itself before its name is
    /// known — a real state, not a missing value.
    pub fn is_hidden(&self) -> bool {
        self.0.is_empty() || self.0.iter().all(|&b| b == 0)
    }

    /// What to put on screen. Lossy, and deliberately one-way: this is
    /// never parsed back into an `Ssid`, because doing so would replace
    /// undecodable bytes with U+FFFD and then fail to connect.
    pub fn to_display_string(&self) -> String {
        if self.is_hidden() {
            return "(hidden network)".to_string();
        }
        String::from_utf8_lossy(&self.0).into_owned()
    }
}

impl fmt::Display for Ssid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_display_string())
    }
}

impl fmt::Debug for Ssid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Ssid({:?})", self.to_display_string())
    }
}

/// The `NM80211ApFlags` / `NM80211ApSecurityFlags` bits this crate reads.
/// Named here rather than inline so the classification below can be read
/// against NetworkManager's own documentation.
pub mod ap_flags {
    /// `NM_802_11_AP_FLAGS_PRIVACY` — the AP requires *some* key.
    pub const PRIVACY: u32 = 0x1;
    pub const KEY_MGMT_PSK: u32 = 0x100;
    pub const KEY_MGMT_802_1X: u32 = 0x200;
    pub const KEY_MGMT_SAE: u32 = 0x400;
    pub const KEY_MGMT_OWE: u32 = 0x800;
    pub const KEY_MGMT_OWE_TM: u32 = 0x1000;
    pub const KEY_MGMT_EAP_SUITE_B_192: u32 = 0x2000;
}

/// How a network expects to be joined.
///
/// Ordered by what this module can actually do: everything up to
/// [`Security::Wpa3Personal`] is connectable here, and
/// [`Security::Enterprise`] deliberately is not — 802.1X needs
/// certificates, identities and a EAP method chooser, which is its own
/// screen and not a checkbox on this one. It is still *listed*, because a
/// network that is missing from the list is a bug report and a network
/// that says "not supported yet" is an answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Security {
    Open,
    /// Opportunistic Wireless Encryption: no passphrase, but encrypted.
    Owe,
    Wep,
    Wpa2Personal,
    Wpa3Personal,
    Enterprise,
}

impl Security {
    /// Classifies an access point from the three flag words
    /// NetworkManager publishes.
    ///
    /// Order matters and is not arbitrary: an AP commonly advertises more
    /// than one of these at once (WPA2/WPA3 transition mode sets both PSK
    /// and SAE), so this reports the *strongest* thing on offer, which is
    /// what NetworkManager will negotiate.
    pub fn classify(flags: u32, wpa_flags: u32, rsn_flags: u32) -> Security {
        use ap_flags::*;
        let both = wpa_flags | rsn_flags;

        if both & (KEY_MGMT_802_1X | KEY_MGMT_EAP_SUITE_B_192) != 0 {
            return Security::Enterprise;
        }
        if rsn_flags & KEY_MGMT_SAE != 0 {
            return Security::Wpa3Personal;
        }
        if both & KEY_MGMT_PSK != 0 {
            return Security::Wpa2Personal;
        }
        if both & (KEY_MGMT_OWE | KEY_MGMT_OWE_TM) != 0 {
            return Security::Owe;
        }
        // Privacy with no WPA or RSN at all is the only thing WEP looks
        // like on this interface. There is no WEP flag to test.
        if flags & PRIVACY != 0 && both == 0 {
            return Security::Wep;
        }
        Security::Open
    }

    /// Whether joining this network means typing something.
    pub fn needs_passphrase(self) -> bool {
        matches!(self, Security::Wep | Security::Wpa2Personal | Security::Wpa3Personal)
    }

    /// `802-11-wireless-security.key-mgmt`, or `None` for a network that
    /// needs no security setting at all.
    pub fn key_mgmt(self) -> Option<&'static str> {
        match self {
            Security::Open => None,
            Security::Owe => Some("owe"),
            Security::Wep => Some("none"),
            Security::Wpa2Personal => Some("wpa-psk"),
            // "sae" is WPA3-only. A transition-mode AP classifies as
            // Wpa3Personal here and "sae" is right for it: NetworkManager
            // falls back to PSK itself if the AP turns out not to support
            // SAE, which is not something this crate can tell in advance.
            Security::Wpa3Personal => Some("sae"),
            Security::Enterprise => Some("wpa-eap"),
        }
    }

    /// Why a network is listed but cannot be joined from here.
    pub fn unsupported_reason(self) -> Option<&'static str> {
        match self {
            Security::Enterprise => Some(
                "This network uses enterprise (802.1X) security, which needs a certificate \
                 and sign-in details. Hyprforge cannot join it yet — use nmcli or nmtui.",
            ),
            _ => None,
        }
    }
}

/// One access point as the list shows it.
#[derive(Debug, Clone, PartialEq)]
pub struct AccessPoint {
    pub ssid: Ssid,
    /// BSSID. Two APs can share an SSID, so this is what identifies one.
    pub bssid: String,
    /// Signal quality in percent, as NetworkManager reports it.
    pub strength: u8,
    pub frequency_mhz: u32,
    pub security: Security,
}

impl AccessPoint {
    /// 2.4 vs 5/6GHz, for the band label beside a network name. Two APs
    /// with one SSID on different bands are otherwise indistinguishable
    /// in the list.
    pub fn band(&self) -> &'static str {
        match self.frequency_mhz {
            0..=2499 => "2.4 GHz",
            2500..=5924 => "5 GHz",
            _ => "6 GHz",
        }
    }
}

/// Whether the radio is on, and whether it is ours to turn on.
///
/// `HardwareOff` is a rocker switch or an Fn key, and no amount of D-Bus
/// will change it — telling the user to flip it is the only useful
/// answer, and is not the same message as "Wi-Fi is off".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RadioState {
    On,
    Off,
    HardwareOff,
}

/// What went wrong, in terms a screen can show without saying "check the
/// logs" (vision pillar #3).
///
/// [`NetworkError::Unavailable`] is separate from an empty network list on
/// purpose, and is the whole reason this enum is not `anyhow::Error`: a
/// NetworkManager that is not running must never render as "there are no
/// networks nearby".
#[derive(Debug, thiserror::Error)]
pub enum NetworkError {
    #[error("NetworkManager isn't running, so networks can't be listed or joined. Start it with `systemctl start NetworkManager`.")]
    Unavailable,
    #[error("This machine has no Wi-Fi adapter that NetworkManager can see.")]
    NoWifiDevice,
    #[error("NetworkManager didn't answer within {0:?}.")]
    TimedOut(std::time::Duration),
    #[error("The password for this network wasn't accepted.")]
    BadPassphrase,
    #[error("{0}")]
    Refused(String),
}

#[cfg(test)]
mod tests {
    use super::*;
    use ap_flags::*;

    #[test]
    fn an_open_network_needs_no_passphrase_and_no_security_setting() {
        let s = Security::classify(0, 0, 0);
        assert_eq!(s, Security::Open);
        assert!(!s.needs_passphrase());
        assert_eq!(s.key_mgmt(), None);
    }

    /// WPA2/WPA3 transition mode advertises both, and reporting it as
    /// WPA2 would offer the weaker of the two things the AP is willing to
    /// do.
    #[test]
    fn a_transition_mode_network_is_reported_as_the_stronger_of_the_two() {
        let s = Security::classify(PRIVACY, KEY_MGMT_PSK, KEY_MGMT_PSK | KEY_MGMT_SAE);
        assert_eq!(s, Security::Wpa3Personal);
        assert_eq!(s.key_mgmt(), Some("sae"));
    }

    #[test]
    fn a_plain_wpa2_network_asks_for_a_passphrase() {
        let s = Security::classify(PRIVACY, KEY_MGMT_PSK, KEY_MGMT_PSK);
        assert_eq!(s, Security::Wpa2Personal);
        assert!(s.needs_passphrase());
        assert_eq!(s.key_mgmt(), Some("wpa-psk"));
    }

    /// Enterprise outranks PSK even when both are advertised: offering a
    /// passphrase box for a network that wants a certificate is a dead
    /// end that looks like a wrong password.
    #[test]
    fn an_enterprise_network_is_listed_but_says_why_it_cannot_be_joined() {
        let s = Security::classify(PRIVACY, KEY_MGMT_802_1X, KEY_MGMT_802_1X | KEY_MGMT_PSK);
        assert_eq!(s, Security::Enterprise);
        assert!(!s.needs_passphrase());
        assert!(s.unsupported_reason().is_some());
    }

    /// OWE is encrypted and passphrase-less, so treating it as Open would
    /// be right by accident; it still needs its own key-mgmt to connect.
    #[test]
    fn an_owe_network_is_joined_without_a_passphrase_but_is_not_open() {
        let s = Security::classify(PRIVACY, 0, KEY_MGMT_OWE);
        assert_eq!(s, Security::Owe);
        assert!(!s.needs_passphrase());
        assert_eq!(s.key_mgmt(), Some("owe"));
    }

    /// There is no WEP bit. Privacy with neither WPA nor RSN behind it is
    /// the only signature it has.
    #[test]
    fn privacy_with_no_wpa_or_rsn_behind_it_is_wep() {
        assert_eq!(Security::classify(PRIVACY, 0, 0), Security::Wep);
    }

    #[test]
    fn an_ssid_that_is_not_utf8_still_lists_and_keeps_its_bytes() {
        let raw = vec![b'c', b'a', b'f', 0xE9];
        let ssid = Ssid::new(raw.clone());
        assert_eq!(ssid.as_bytes(), &raw[..], "the bytes to connect with are kept");
        assert!(ssid.to_display_string().contains("caf"));
        assert!(!ssid.is_hidden());
    }

    /// A hidden network reports an empty or all-zero SSID. It is a real
    /// access point with an unknown name, not an absent one.
    #[test]
    fn a_hidden_network_is_named_rather_than_shown_blank() {
        assert!(Ssid::new(Vec::new()).is_hidden());
        assert!(Ssid::new(vec![0, 0, 0]).is_hidden());
        assert_eq!(Ssid::new(Vec::new()).to_display_string(), "(hidden network)");
    }

    #[test]
    fn the_band_label_separates_two_radios_sharing_one_name() {
        let ap = |mhz| AccessPoint {
            ssid: Ssid::new("home"),
            bssid: "00:11:22:33:44:55".to_string(),
            strength: 70,
            frequency_mhz: mhz,
            security: Security::Wpa2Personal,
        };
        assert_eq!(ap(2412).band(), "2.4 GHz");
        assert_eq!(ap(5180).band(), "5 GHz");
        assert_eq!(ap(6175).band(), "6 GHz");
    }

    /// The distinction this crate exists to keep: a daemon that is not
    /// running is not an empty network list.
    #[test]
    fn an_absent_networkmanager_says_so_rather_than_reporting_no_networks() {
        let message = NetworkError::Unavailable.to_string();
        assert!(message.contains("isn't running"));
        assert!(
            message.contains("systemctl"),
            "an error with no way out of it is the dead end pillar 3 forbids"
        );
    }
}
