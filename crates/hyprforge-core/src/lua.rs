//! Quoting values as Lua source.
//!
//! Shared because every crate that generates Lua needs exactly this and the
//! subtlety is not obvious: `hyprforge-shortcuts`, `hyprforge-windowrules`
//! and `hyprforge-displayd` each had their own copy, all three carrying the
//! same wrong test for when the fallback level is needed. Whichever crate
//! writes the file, the failure is the same — Hyprland rejects the *whole*
//! file, not the line that's wrong.

/// Quotes with a Lua long bracket, which needs no escaping at all — the
/// alternative is escaping quotes and backslashes in commands like
/// `sh -c "echo \"hi\""` and in RE2 window-match patterns, and getting that
/// subtly wrong writes a broken config.
///
/// Two rules of Lua's long-bracket syntax that are easy to get wrong:
///
/// 1. The level has to be chosen so the closing bracket can't appear early.
///    "Does the content contain `]]`?" is not that test — a value merely
///    *ending* in `]` is enough (a `foo[0-9]` class regex, a command ending
///    in `]`), because its trailing `]` fuses with the closing `]]` into
///    `]]]` and Lua terminates the string one character early. Verified
///    against lua5.4: `[[echo ]]]` is a syntax error.
/// 2. A newline immediately after the opening bracket is dropped by the
///    parser, so a value that starts with one has to be opened with an
///    extra newline to survive the round trip.
///
/// And one it cannot express at all: inside a long bracket Lua normalises
/// every `\r\n`, `\n\r` and lone `\r` to `\n`, so a value carrying a
/// carriage return comes back as a different string. Those are written as
/// an escaped short string instead (see `escaped_string`).
pub fn lua_string(s: &str) -> String {
    if s.contains('\r') {
        return escaped_string(s);
    }
    let level = (0..).find(|level| fits_at_level(s, *level)).unwrap_or(0);
    let eq = "=".repeat(level);
    // Rule 2: the parser eats a newline directly after `[[`, so give it one
    // to eat that isn't the value's own.
    let lead = if s.starts_with('\n') { "\n" } else { "" };
    format!("[{eq}[{lead}{s}]{eq}]")
}

/// `s` as a double-quoted Lua string, every byte that needs it escaped.
///
/// Control characters go out as three-digit decimal escapes: Lua's `\ddd`
/// takes up to three digits, so a shorter `\0` followed by a digit in the
/// value would read as a different byte.
fn escaped_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            c if (c as u32) < 0x20 || c as u32 == 0x7f => out.push_str(&format!("\\{:03}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// `s` bare if Lua reads it as the same finite number, quoted otherwise.
///
/// For a field that takes either a number or an expression string — a
/// window rule's `move`, a gesture's argument. "Does Rust parse it as an
/// `f64`" is not that test: Rust accepts `nan`, `inf` and `infinity`,
/// which bare are Lua globals reading as `nil`, and `+5`, which Lua has no
/// syntax for — one such value made Hyprland refuse the whole file.
pub fn lua_scalar(s: &str) -> String {
    let numeric = !s.starts_with('+')
        && s.bytes().all(|b| b.is_ascii_digit() || matches!(b, b'.' | b'-' | b'e' | b'E' | b'+'))
        && s.parse::<f64>().is_ok_and(f64::is_finite);
    if numeric {
        s.to_string()
    } else {
        lua_string(s)
    }
}

/// Whether `s` can be quoted at this bracket level without terminating early.
///
/// The closing delimiter is `]` + `level` `=`s + `]`. A premature match can
/// begin anywhere in `s` and run into the delimiter that follows it, so the
/// test appends all but the last character of the delimiter and looks for a
/// match in the whole thing — which catches both a delimiter sitting inside
/// the content and a content whose tail overlaps the start of it.
fn fits_at_level(s: &str, level: usize) -> bool {
    let close = format!("]{}]", "=".repeat(level));
    let overlap = &close[..close.len() - 1];
    !format!("{s}{overlap}").contains(&close)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_string_needs_no_escaping_inside_long_brackets() {
        assert_eq!(lua_string(r#"sh -c "echo \"hi\"""#), r#"[[sh -c "echo \"hi\""]]"#);
    }

    #[test]
    fn a_string_containing_the_bracket_falls_back_a_level() {
        assert_eq!(lua_string("a]]b"), "[=[a]]b]=]");
    }

    /// The case that made a whole generated file unloadable: a value merely
    /// *ending* in `]` fuses with the closing bracket, so `[[echo ]]]`
    /// terminates a character early. lua5.4 rejects it.
    #[test]
    fn a_string_ending_in_a_bracket_falls_back_a_level() {
        assert_eq!(lua_string("echo ]"), "[=[echo ]]=]");
        // The shape this actually reaches production as: a window-match
        // regex on a class like `firefox[0-9]`.
        assert_eq!(lua_string("^firefox[0-9]"), "[=[^firefox[0-9]]=]");
    }

    /// The level is chosen against the delimiter that level actually uses,
    /// not against `]]` alone: `]=]` is harmless at level 0 and fatal at
    /// level 1, so a string carrying both hazards has to go deeper still.
    #[test]
    fn the_level_is_chosen_per_string_rather_than_assumed() {
        assert_eq!(lua_string("a]=]b"), "[[a]=]b]]");
        assert_eq!(lua_string("a]]b]=]c"), "[==[a]]b]=]c]==]");
    }

    /// A long bracket hands back `\r` and `\r\n` as `\n` — Lua normalises
    /// line endings inside one — so a value carrying a carriage return
    /// cannot be quoted that way and still be itself.
    #[test]
    fn a_carriage_return_is_never_put_in_long_brackets() {
        for s in ["a\rb", "a\r\nb", "\r"] {
            let quoted = lua_string(s);
            assert!(!quoted.starts_with('['), "{s:?} quoted as {quoted}");
            assert!(!quoted.contains('\r'), "{s:?} quoted as {quoted:?}");
        }
        assert_eq!(lua_string("a\rb"), "\"a\\rb\"");
    }

    /// A value is written bare only if Lua reads it as that same number.
    /// Rust's `f64` parser accepts `nan`, `inf` and `infinity` — bare, those
    /// are Lua globals that read as `nil` — and `+5`, which is no Lua at
    /// all and takes the whole generated file down.
    #[test]
    fn only_a_finite_number_lua_can_read_is_written_bare() {
        for bare in ["5", "-5", "0.5", ".5", "5.", "1e3", "-2.5E-1"] {
            assert_eq!(lua_scalar(bare), bare, "{bare}");
        }
        for quoted in ["nan", "NaN", "inf", "-inf", "infinity", "+5", "1e999", "50%", "cursor_x"] {
            assert!(lua_scalar(quoted).starts_with('['), "{quoted} written bare");
        }
    }

    /// Lua drops a newline directly after the opening bracket, so a value
    /// starting with one has to be given a spare to eat.
    #[test]
    fn a_leading_newline_survives() {
        assert_eq!(lua_string("\nhello"), "[[\n\nhello]]");
    }

    /// Every quoted string has to parse back as itself. This is the property
    /// the individual cases above are examples of — a regression here writes
    /// a config Hyprland refuses in full.
    #[test]
    fn every_awkward_string_round_trips_through_a_real_lua_parser() {
        // Belt-and-braces on top of the explicit cases above; a machine
        // without a Lua binary shouldn't fail the suite for it.
        if lua_eval_string("[[probe]]").as_deref() != Some("probe") {
            return;
        }
        for s in [
            "plain",
            "echo ]",
            "a]]b",
            "a]=]b",
            "]",
            "]]",
            "]=]",
            "[[",
            "\nhello",
            "\n",
            r#"sh -c "echo \"hi\"""#,
            "${arr[1]}",
            "^firefox[0-9]",
            "grep -o '[a-z]'",
            "desc:BOE 0x0BC9",
            "a\rb",
            "a\r\nb",
            "\r",
            "tab\there \\ \"q\" \x01",
        ] {
            let quoted = lua_string(s);
            assert_eq!(
                lua_eval_string(&quoted).as_deref(),
                Some(s),
                "{s:?} quoted as {quoted} did not parse back as itself"
            );
        }
    }

    /// Parses `quoted` with the system Lua and returns the string it
    /// evaluates to. `None` means no usable `lua` binary; a syntax error
    /// comes back as a distinctive value rather than `None`, so a broken
    /// quoting can never be mistaken for an absent interpreter.
    fn lua_eval_string(quoted: &str) -> Option<String> {
        use std::process::Command;
        let script = format!("io.write({quoted})");
        let evaluated = crate::command::output(
            Command::new("lua").arg("-e").arg(&script),
            crate::command::TIMEOUT,
        );
        let out = match evaluated {
            Ok(out) => out,
            Err(_) => return None,
        };
        if !out.status.success() {
            return Some(format!(
                "SYNTAX ERROR: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            ));
        }
        Some(String::from_utf8_lossy(&out.stdout).into_owned())
    }
}
