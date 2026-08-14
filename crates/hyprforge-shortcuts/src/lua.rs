//! Rendering values as Lua source.
//!
//! One implementation, used by both directions: [`crate::codegen`] writing a
//! shortcut out, and [`crate::import`] writing a captured argument back. They
//! had a copy each, and the copies disagreed — the import one didn't handle
//! tables, which is how a config full of `hl.dsp.focus({ direction = ... })`
//! came back as `hl.dsp.focus()` and was rejected wholesale by Hyprland.

use crate::model::ParamValue;
use std::collections::BTreeMap;

/// Re-exported rather than reimplemented: every crate that generates Lua
/// needs the same quoting, and the three copies this project used to have
/// all shared the same bug. See [`hyprforge_core::lua`] for what makes it
/// subtle.
pub use hyprforge_core::lua::lua_string;

/// A table key, bare when it can be and bracketed when it can't.
///
/// The spaces inside the brackets are load-bearing: `[[[key]]]` reads as a
/// long-bracket string, not an indexed key.
pub fn lua_key(k: &str) -> String {
    let is_identifier = !k.is_empty()
        && !k.starts_with(|c: char| c.is_ascii_digit())
        && k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        && !KEYWORDS.contains(&k);
    if is_identifier {
        k.to_string()
    } else {
        format!("[ {} ]", lua_string(k))
    }
}

const KEYWORDS: [&str; 22] = [
    "and", "break", "do", "else", "elseif", "end", "false", "for", "function", "goto", "if", "in",
    "local", "nil", "not", "or", "repeat", "return", "then", "true", "until", "while",
];

pub fn render_value(v: &ParamValue) -> String {
    match v {
        ParamValue::Bool(b) => b.to_string(),
        ParamValue::Int(i) => i.to_string(),
        ParamValue::Float(f) => f.to_string(),
        ParamValue::Str(s) => lua_string(s),
    }
}

/// `{ k = v, ... }`, or `{}` when empty, from already-rendered values.
///
/// Both table shapes this crate emits come through here — a dispatcher's
/// typed params and `hl.bind`'s options table — so the two can't drift on
/// spacing or on what an empty table looks like.
pub fn render_entries(entries: &[(&str, String)]) -> String {
    if entries.is_empty() {
        return "{}".to_string();
    }
    let rendered: Vec<String> =
        entries.iter().map(|(k, v)| format!("{} = {}", lua_key(k), v)).collect();
    format!("{{ {} }}", rendered.join(", "))
}

/// `{ k = v, ... }` from typed param values.
pub fn render_table(params: &BTreeMap<String, ParamValue>) -> String {
    let entries: Vec<(&str, String)> =
        params.iter().map(|(k, v)| (k.as_str(), render_value(v))).collect();
    render_entries(&entries)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The quoting itself is covered in `hyprforge-core::lua`, including
    /// against a real Lua parser. What matters here is only that this
    /// module's own renderers reach it.
    #[test]
    fn values_are_quoted_through_the_shared_helper() {
        assert_eq!(render_value(&ParamValue::Str("echo ]".into())), "[=[echo ]]=]");
    }

    #[test]
    fn identifiers_are_bare_and_everything_else_is_bracketed() {
        assert_eq!(lua_key("direction"), "direction");
        assert_eq!(lua_key("on_current_monitor"), "on_current_monitor");
        assert_eq!(lua_key("end"), "[ [[end]] ]");
        assert_eq!(lua_key("2"), "[ [[2]] ]");
        assert_eq!(lua_key("special:magic"), "[ [[special:magic]] ]");
    }

    #[test]
    fn a_table_renders_its_keys_in_a_stable_order() {
        let mut params = BTreeMap::new();
        params.insert("workspace".to_string(), ParamValue::Int(3));
        params.insert("follow".to_string(), ParamValue::Bool(true));
        assert_eq!(render_table(&params), "{ follow = true, workspace = 3 }");
    }

    #[test]
    fn an_empty_table_is_still_a_table() {
        assert_eq!(render_table(&BTreeMap::new()), "{}");
    }
}
