//! One colour type, and one parser for it.
//!
//! `rgba(rrggbbaa)` is already the universal spelling across this
//! workspace — the catalogue defaults, every stored `Value::Text`, the
//! lock screen's theme file. What was missing was a single
//! implementation: four grew independently, with three different ideas
//! about what to do with input that doesn't parse.
//!
//! **Leniency is not a mode here.** `parse` is strict and there is no
//! forgiving variant, because a fallback is a decision about what the
//! user should see and it belongs at the place that decides — a lock
//! screen writes `Color::parse(s).unwrap_or(Color::BLACK)` and the
//! reason ("a lock screen with a bad colour must be plain, not absent")
//! stays visible next to the code that depends on it, instead of being
//! a property hidden inside a helper.
//!
//! [`Display`](std::fmt::Display) is the only way to turn one back into
//! a string, so no caller can invent a fifth spelling.

use std::fmt;

/// A colour, 8 bits per channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("expected rgba(rrggbbaa) or rgb(rrggbb) in hex, got {0:?}")]
pub struct ColorError(pub String);

impl Color {
    pub const BLACK: Color = Color { r: 0, g: 0, b: 0, a: 0xff };

    pub const fn rgba(r: u8, g: u8, b: u8, a: u8) -> Color {
        Color { r, g, b, a }
    }

    /// Parses `rgba(rrggbbaa)` or `rgb(rrggbb)`, strictly.
    ///
    /// The digit count is tied to the prefix: `rgba()` takes eight and
    /// `rgb()` six. Accepting eight digits inside `rgb(` would draw a
    /// confident swatch beside a value Hyprland is about to refuse.
    pub fn parse(s: &str) -> Result<Color, ColorError> {
        let bad = || ColorError(s.to_string());
        let (body, want) = match s.strip_prefix("rgba(") {
            Some(rest) => (rest.strip_suffix(')').ok_or_else(bad)?, 8),
            None => (
                s.strip_prefix("rgb(")
                    .ok_or_else(bad)?
                    .strip_suffix(')')
                    .ok_or_else(bad)?,
                6,
            ),
        };
        if body.len() != want || !body.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(bad());
        }
        let byte = |i: usize| u8::from_str_radix(&body[i..i + 2], 16).map_err(|_| bad());
        Ok(Color {
            r: byte(0)?,
            g: byte(2)?,
            b: byte(4)?,
            a: if want == 8 { byte(6)? } else { 0xff },
        })
    }

    /// Reads `hyprctl`'s gradient spelling, which is `AARRGGBB` — the
    /// alpha leading rather than trailing. Copying that string into a
    /// config would be a different colour.
    pub fn from_argb_hex(s: &str) -> Option<Color> {
        if s.len() != 8 || !s.chars().all(|c| c.is_ascii_hexdigit()) {
            return None;
        }
        let byte = |i: usize| u8::from_str_radix(&s[i..i + 2], 16).ok();
        Some(Color { a: byte(0)?, r: byte(2)?, g: byte(4)?, b: byte(6)? })
    }

    /// Reads `hyprctl`'s plain-colour spelling, a packed `0xAARRGGBB`
    /// integer. `rgba(112233ff)` comes back as `4279312947`.
    pub fn from_argb_u32(v: u32) -> Color {
        Color {
            a: (v >> 24) as u8,
            r: (v >> 16) as u8,
            g: (v >> 8) as u8,
            b: v as u8,
        }
    }

    /// Packed `0xAARRGGBB`, which is what a Wayland `Argb8888` shm
    /// buffer wants.
    pub fn to_argb(self) -> u32 {
        (self.a as u32) << 24 | (self.r as u32) << 16 | (self.g as u32) << 8 | self.b as u32
    }
}

impl fmt::Display for Color {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "rgba({:02x}{:02x}{:02x}{:02x})", self.r, self.g, self.b, self.a)
    }
}

impl serde::Serialize for Color {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_string())
    }
}

impl<'de> serde::Deserialize<'de> for Color {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Color, D::Error> {
        let raw = String::deserialize(d)?;
        Color::parse(&raw).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_two_accepted_spellings_parse() {
        assert_eq!(Color::parse("rgba(bd93f9ff)").unwrap(), Color::rgba(0xbd, 0x93, 0xf9, 0xff));
        assert_eq!(Color::parse("rgb(282a36)").unwrap(), Color::rgba(0x28, 0x2a, 0x36, 0xff));
        assert_eq!(Color::parse("rgba(00000080)").unwrap(), Color::rgba(0, 0, 0, 0x80));
    }

    /// The digit count is tied to the prefix. Being more lenient would
    /// accept values Hyprland itself refuses, which is worse than
    /// refusing them here: the user would see a confident swatch beside
    /// a setting that will not apply.
    #[test]
    fn the_digit_count_must_match_the_prefix() {
        assert!(Color::parse("rgb(bd93f9ff)").is_err(), "eight digits in rgb(");
        assert!(Color::parse("rgba(282a36)").is_err(), "six digits in rgba(");
    }

    /// Every shape the old parsers disagreed about. One of them treated
    /// these as opaque black, another as `None`; now there is one answer
    /// and the caller decides what to do with it.
    #[test]
    fn anything_else_is_refused_rather_than_guessed() {
        for bad in [
            "",
            "nonsense",
            "rgba(",
            "rgba(zzzzzzzz)",
            "0xffbd93f9",
            "rgba(bd93f9ff",
            "bd93f9ff",
            "rgba(bd93f9ff) ",
        ] {
            assert!(Color::parse(bad).is_err(), "{bad:?} should not parse");
        }
    }

    /// `hyprctl` puts the alpha first; the config format puts it last.
    /// Getting this backwards is a different colour, not a near miss.
    #[test]
    fn hyprctl_spellings_move_the_alpha_to_the_end() {
        assert_eq!(Color::from_argb_hex("ffbd93f9").unwrap().to_string(), "rgba(bd93f9ff)");
        assert_eq!(Color::from_argb_u32(4279312947).to_string(), "rgba(112233ff)");
    }

    #[test]
    fn a_colour_round_trips_through_its_string_form() {
        for original in ["rgba(bd93f9ff)", "rgba(00000000)", "rgba(ffffffff)"] {
            let parsed = Color::parse(original).unwrap();
            assert_eq!(parsed.to_string(), original);
            assert_eq!(Color::parse(&parsed.to_string()).unwrap(), parsed);
        }
    }

    /// What the shm buffer is handed. The lock screen draws with this,
    /// so a byte-order slip here is a visibly wrong lock screen.
    #[test]
    fn packing_for_a_wayland_buffer_is_argb() {
        assert_eq!(Color::parse("rgba(bd93f9ff)").unwrap().to_argb(), 0xffbd93f9);
        assert_eq!(Color::BLACK.to_argb(), 0xff00_0000);
    }

    #[test]
    fn colours_survive_serde_as_the_string_form() {
        #[derive(serde::Serialize, serde::Deserialize, PartialEq, Debug)]
        struct Holder {
            accent: Color,
        }
        let held = Holder { accent: Color::parse("rgba(bd93f9ff)").unwrap() };
        let text = toml::to_string(&held).unwrap();
        assert!(text.contains(r#"accent = "rgba(bd93f9ff)""#), "{text}");
        assert_eq!(toml::from_str::<Holder>(&text).unwrap(), held);
    }
}
