//! Compiling a fragment of Lua without running it.
//!
//! The Shortcuts editor has a raw escape hatch — a dispatcher path and a Lua
//! argument expression, spliced verbatim into the generated file. Nothing
//! else checks it, and a syntax error there doesn't cost the user one broken
//! shortcut: Hyprland refuses the whole file, so every *other* shortcut stops
//! working too. Catching it in the editor turns that into a message beside
//! the field.
//!
//! This lives here rather than in `hyprforge-shortcuts` for the same reason
//! `import.rs` takes plain JSON: only this crate depends on `mlua`, and
//! keeping it that way is what stops a Lua VM being linked into every domain
//! crate. See `hyprforge-shortcuts/src/import.rs` for the full argument.
//!
//! Window rules deliberately don't use this, and that isn't an oversight:
//! every window-rule field reaches the file through `lua_string` quoting and
//! is valid by construction. Shortcuts is the only place a user's own text
//! is spliced in as Lua *source*.

use mlua::Lua;

/// `Ok(())` if `src` compiles as Lua, or the parser's complaint if it
/// doesn't.
///
/// Compiles only — the chunk is never run, so a call to a function that
/// doesn't exist in this VM (every `hl.*` in a generated line) is fine.
/// That's the intended scope: this catches unbalanced brackets, a stray
/// quote, a missing comma. It cannot tell you a dispatcher name is wrong,
/// and doesn't claim to.
pub fn check_syntax(src: &str) -> Result<(), String> {
    let lua = Lua::new();
    lua.load(src)
        .set_name("shortcut")
        .into_function()
        .map(|_| ())
        .map_err(|e| first_line(&e.to_string()))
}

/// Lua's error text carries a traceback and the chunk name; the first line
/// is the part a user can act on.
fn first_line(message: &str) -> String {
    let line = message.lines().next().unwrap_or(message).trim();
    // `[string "shortcut"]:1: unexpected symbol` — the prefix is noise to
    // anyone who isn't holding the chunk we just invented.
    match line.split_once("]:") {
        Some((_, rest)) => rest.trim_start_matches(|c: char| c.is_ascii_digit() || c == ':')
            .trim()
            .to_string(),
        None => line.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_well_formed_line_compiles() {
        assert!(check_syntax("hl.bind([[SUPER + Q]], hl.dsp.window.close(), {})").is_ok());
    }

    /// The point of the check: undefined globals are a runtime concern, and
    /// every generated line is made of them. Compiling must not care.
    #[test]
    fn undefined_globals_are_not_a_syntax_error() {
        assert!(check_syntax("this_does_not_exist({ a = 1 })").is_ok());
    }

    #[test]
    fn an_unbalanced_argument_is_reported() {
        let err = check_syntax("hl.dsp.focus({ direction = [[left]] ").unwrap_err();
        assert!(!err.is_empty());
        // The chunk name is an implementation detail of this function.
        assert!(!err.contains("[string"), "leaked chunk name: {err}");
    }

    #[test]
    fn a_stray_bracket_is_reported() {
        assert!(check_syntax("hl.dsp.exec_cmd([[echo ]]])").is_err());
    }

    /// Nothing is executed, so a chunk that would do damage if run is still
    /// only ever parsed.
    #[test]
    fn the_chunk_is_never_run() {
        let path = std::env::temp_dir().join("hyprforge-syntax-check-must-not-exist");
        let _ = std::fs::remove_file(&path);
        let src = format!("local f = io.open([[{}]], [[w]]) f:write([[x]])", path.display());
        assert!(check_syntax(&src).is_ok(), "it should compile");
        assert!(!path.exists(), "compiling must not run the chunk");
    }
}
