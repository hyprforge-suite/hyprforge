//! Turning a real key press into the name Hyprland binds against.
//!
//! Hyprland matches binds on **xkb keysym names**, which are not what iced
//! reports and not what's printed on the key: Return rather than Enter,
//! `left` rather than ArrowLeft, `XF86AudioRaiseVolume` rather than
//! AudioVolumeUp. Typing those from memory is exactly the kind of thing the
//! old free-text key field asked users to do.
//!
//! What capture can't do, and why the manual field stays: the compositor
//! consumes a chord it has already bound before any client sees it, so
//! pressing an in-use shortcut here produces nothing at all. Capture is the
//! fast path for free chords, not a replacement for being able to type a key
//! name.

use hyprforge_shortcuts::Modifier;
use iced::keyboard::key::{Code, Named, Physical};
use iced::keyboard::{Key, Modifiers};

/// The modifiers held during a capture, as this crate models them.
///
/// Only the four a person actually binds. CAPS and MOD2/3/5 exist in
/// [`Modifier`] because Hyprland's modmask has them and an imported bind may
/// use them, but nobody *presses* them deliberately.
pub fn modifiers_from(modifiers: Modifiers) -> Vec<Modifier> {
    let mut mods = Vec::new();
    if modifiers.shift() {
        mods.push(Modifier::Shift);
    }
    if modifiers.control() {
        mods.push(Modifier::Ctrl);
    }
    if modifiers.alt() {
        mods.push(Modifier::Alt);
    }
    if modifiers.logo() {
        mods.push(Modifier::Super);
    }
    mods
}

/// The Hyprland key name for a press, or `None` if this press shouldn't end
/// a capture.
///
/// A bare modifier returns `None` on purpose: holding Super to build
/// `SUPER + Q` fires a key press for Super itself first, and ending the
/// capture there would record the modifier as the key.
pub fn key_name(key: &Key, physical: &Physical) -> Option<String> {
    match key {
        Key::Named(named) => named_key(*named),
        Key::Character(c) => character_key(c),
        // Media keys on some layouts arrive with no logical key at all;
        // the physical scancode still identifies them.
        Key::Unidentified => physical_key(physical),
    }
    // A layout that produces no logical name can still be identified
    // physically — worth trying before giving up on the press.
    .or_else(|| physical_key(physical))
}

/// Letters uppercase, everything else as typed.
///
/// Hyprland matches letters case-insensitively but named keys exactly, and
/// uppercase is the form every config in the wild writes.
fn character_key(c: &str) -> Option<String> {
    let c = c.trim();
    if c.is_empty() {
        return None;
    }
    Some(if c.chars().all(|c| c.is_ascii_alphabetic()) {
        c.to_ascii_uppercase()
    } else {
        c.to_string()
    })
}

fn named_key(named: Named) -> Option<String> {
    let name = match named {
        // Bare modifiers never end a capture.
        Named::Shift | Named::Control | Named::Alt | Named::Super | Named::Meta | Named::Hyper => {
            return None
        }
        Named::Enter => "Return",
        Named::Tab => "Tab",
        Named::Space => "space",
        Named::Backspace => "BackSpace",
        Named::Delete => "Delete",
        Named::Insert => "Insert",
        Named::Escape => "Escape",
        Named::Home => "Home",
        Named::End => "End",
        Named::PageUp => "Prior",
        Named::PageDown => "Next",
        Named::ArrowLeft => "left",
        Named::ArrowRight => "right",
        Named::ArrowUp => "up",
        Named::ArrowDown => "down",
        Named::PrintScreen => "Print",
        Named::Pause => "Pause",
        Named::CapsLock => "Caps_Lock",
        Named::NumLock => "Num_Lock",
        Named::ScrollLock => "Scroll_Lock",
        Named::ContextMenu => "Menu",
        // The XF86 family — the reason capture is worth having at all.
        Named::AudioVolumeUp => "XF86AudioRaiseVolume",
        Named::AudioVolumeDown => "XF86AudioLowerVolume",
        Named::AudioVolumeMute => "XF86AudioMute",
        Named::MicrophoneVolumeMute => "XF86AudioMicMute",
        Named::MediaPlayPause => "XF86AudioPlay",
        Named::MediaPause => "XF86AudioPause",
        Named::MediaStop => "XF86AudioStop",
        Named::MediaTrackNext => "XF86AudioNext",
        Named::MediaTrackPrevious => "XF86AudioPrev",
        Named::BrightnessUp => "XF86MonBrightnessUp",
        Named::BrightnessDown => "XF86MonBrightnessDown",
        Named::LaunchMail => "XF86Mail",
        Named::LaunchWebBrowser => "XF86HomePage",
        Named::LaunchMediaPlayer => "XF86AudioMedia",
        Named::Power => "XF86PowerOff",
        Named::Eject => "XF86Eject",
        Named::WakeUp => "XF86WakeUp",
        Named::F1 => "F1",
        Named::F2 => "F2",
        Named::F3 => "F3",
        Named::F4 => "F4",
        Named::F5 => "F5",
        Named::F6 => "F6",
        Named::F7 => "F7",
        Named::F8 => "F8",
        Named::F9 => "F9",
        Named::F10 => "F10",
        Named::F11 => "F11",
        Named::F12 => "F12",
        _ => return None,
    };
    Some(name.to_string())
}

/// The scancode route, for keys the layout gives no logical name to.
fn physical_key(physical: &Physical) -> Option<String> {
    let Physical::Code(code) = physical else {
        return None;
    };
    let name = match code {
        Code::AudioVolumeUp => "XF86AudioRaiseVolume",
        Code::AudioVolumeDown => "XF86AudioLowerVolume",
        Code::AudioVolumeMute => "XF86AudioMute",
        Code::MediaPlayPause => "XF86AudioPlay",
        Code::MediaTrackNext => "XF86AudioNext",
        Code::MediaTrackPrevious => "XF86AudioPrev",
        Code::MediaStop => "XF86AudioStop",
        _ => return None,
    };
    Some(name.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unknown() -> Physical {
        Physical::Unidentified(iced::keyboard::key::NativeCode::Unidentified)
    }

    #[test]
    fn a_letter_is_uppercased() {
        assert_eq!(key_name(&Key::Character("q".into()), &unknown()).as_deref(), Some("Q"));
    }

    #[test]
    fn a_digit_is_left_alone() {
        assert_eq!(key_name(&Key::Character("1".into()), &unknown()).as_deref(), Some("1"));
    }

    /// The names Hyprland actually wants, which are not the names iced uses.
    #[test]
    fn named_keys_use_xkb_names() {
        for (named, expected) in [
            (Named::Enter, "Return"),
            (Named::ArrowLeft, "left"),
            (Named::PageUp, "Prior"),
            (Named::Backspace, "BackSpace"),
            (Named::Space, "space"),
        ] {
            assert_eq!(key_name(&Key::Named(named), &unknown()).as_deref(), Some(expected));
        }
    }

    /// The keys nobody can spell from memory — the whole point of capture.
    #[test]
    fn media_keys_map_to_their_xf86_names() {
        assert_eq!(
            key_name(&Key::Named(Named::AudioVolumeUp), &unknown()).as_deref(),
            Some("XF86AudioRaiseVolume")
        );
        // Even with no logical key at all, the scancode identifies it.
        assert_eq!(
            key_name(&Key::Unidentified, &Physical::Code(Code::AudioVolumeMute)).as_deref(),
            Some("XF86AudioMute")
        );
    }

    /// Holding Super to build `SUPER + Q` fires a press for Super first —
    /// ending the capture there would record the modifier as the key.
    #[test]
    fn a_bare_modifier_does_not_end_a_capture() {
        for named in [Named::Shift, Named::Control, Named::Alt, Named::Super] {
            assert_eq!(key_name(&Key::Named(named), &unknown()), None, "{named:?}");
        }
    }

    #[test]
    fn only_the_four_real_modifiers_are_captured() {
        let mods = modifiers_from(Modifiers::SHIFT | Modifiers::LOGO);
        assert_eq!(mods, vec![Modifier::Shift, Modifier::Super]);
        assert!(modifiers_from(Modifiers::default()).is_empty());
    }
}
