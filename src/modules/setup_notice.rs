//! The banner shown when the user's Hyprland config can't take a
//! `require()` line.
//!
//! Lives here rather than in `hyprforge-ui` because it is the one widget
//! that knows what Hyprland is. Keeping it in the shared crate would
//! mean a calculator depended on Lua config machinery to draw a button.

use hyprforge_core::lua_setup::HyprConfig;
use hyprforge_ui::theme::{spacing, FontScale};
use hyprforge_ui::widgets::{meta_text, scaled_text, section};
use iced::widget::column;
use iced::Element;

/// The banner for the two config shapes a require line can't serve.
///
/// `None` for a Lua config, which needs no notice at all. `subject` names
/// what this module generates — "rules", "binds", "monitor settings" — and
/// is the only thing that differed between the three copies of this text
/// the Settings modules each carried.
///
/// Never blocks editing: a module still saves to its own TOML and activates
/// once setup is done (vision pillar #3: no dead ends).
pub fn setup_notice<'a, Message: 'a>(
    config: &HyprConfig,
    subject: &str,
    scale: FontScale,
) -> Option<Element<'a, Message>> {
    
    let body = match config {
        HyprConfig::Lua(_) => return None,
        HyprConfig::Missing => column![scaled_text(
            "No Hyprland config found, and Hyprforge couldn't create one \
             automatically — see the error above.",
            13.0,
            scale,
        )],
        HyprConfig::ConfOnly(path) => column![
            scaled_text(
                format!(
                    "You're using Hyprland's hyprland.conf format. Hyprforge \
                     generates Lua {subject} and sources them with require(), which \
                     only the Lua config supports — so it won't modify your .conf."
                ),
                13.0,
                scale,
            ),
            meta_text(path.display().to_string(), 12.0, scale),
            scaled_text(
                "Hyprland switched its config language from hyprlang to Lua \
                 in 0.55. To use this module, port your settings into a \
                 hyprland.lua; Hyprforge will pick it up automatically on \
                 next launch. Your .conf is left untouched either way.",
                13.0,
                scale,
            ),
        ],
    };
    Some(section("Setup required", scale, body.spacing(spacing::SM)))
}
