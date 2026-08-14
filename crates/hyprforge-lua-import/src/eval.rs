//! Evaluates a user's `hyprland.lua` (and everything it `require()`s) in a
//! sandboxed Lua state, with `hl.*` stubbed to record calls instead of
//! acting on them.
//!
//! This is the whole point of embedding a real interpreter rather than
//! text-parsing: a hand-written `hl.bind(mainMod .. " + Q", ...)` cannot be
//! read as a value from source text — `mainMod` is a variable, and only
//! evaluating the script (with the user's own variables, in their own
//! order) resolves it. See `sandbox.rs` for the proxy mechanism that makes
//! `hl.dsp.window.close()`-style chained dispatcher calls recordable
//! without knowing every dispatcher name ahead of time.

use crate::record::{ImportResult, RecordedCall};
use mlua::{HookTriggers, Lua, LuaSerdeExt, MetaMethod, Table, Value, Variadic, VmState};
use std::cell::RefCell;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::rc::Rc;

/// Generous but finite: real configs are a few hundred lines, not a
/// computation. This exists purely so a pathological or accidentally
/// -infinite config can't hang an import — see [`evaluate`].
const MAX_INSTRUCTIONS: u32 = 5_000_000;
const MEMORY_LIMIT_BYTES: usize = 64 * 1024 * 1024;

#[derive(Default, Clone)]
struct EvalState {
    calls: Vec<RecordedCall>,
    failures: Vec<(PathBuf, String)>,
    /// Top is whichever file is currently executing — `require()` pushes
    /// before running the required file and pops after, so a call
    /// recorded partway through a nested require is still attributed to
    /// the right file.
    source_stack: Vec<PathBuf>,
    /// Mirrors Lua's own `package.loaded` memoization: a file required
    /// twice (directly or via two different requirers) only runs once.
    required: HashSet<PathBuf>,
}

type SharedState = Rc<RefCell<EvalState>>;

/// Evaluates `<hypr_config_dir>/hyprland.lua` and returns everything it
/// recorded. Never panics and never returns `Err` — anything that goes
/// wrong (a missing file, a construct the sandbox doesn't support, hitting
/// a safety limit) is reported per-file in [`ImportResult::failures`]
/// instead, so one broken `require()` doesn't take the rest of an
/// otherwise-importable config down with it.
pub fn evaluate(hypr_config_dir: &Path) -> ImportResult {
    let state: SharedState = Rc::new(RefCell::new(EvalState::default()));
    {
        let lua = Lua::new();
        match configure_sandbox(&lua, hypr_config_dir, state.clone()) {
            Ok(()) => {
                let entry = hypr_config_dir.join("hyprland.lua");
                run_file(&lua, &entry, &state);
            }
            Err(e) => state.borrow_mut().failures.push((
                hypr_config_dir.to_path_buf(),
                format!("failed to set up the import sandbox: {e}"),
            )),
        }
        // `lua` drops here, releasing every closure's clone of `state` —
        // required before `finish` can reclaim sole ownership.
    }
    finish(state)
}

fn finish(state: SharedState) -> ImportResult {
    let inner = match Rc::try_unwrap(state) {
        Ok(cell) => cell.into_inner(),
        // Shouldn't happen once `lua` (and every closure it owned) has
        // dropped — falls back to cloning rather than panicking a GUI
        // action over an Rc-accounting edge case.
        Err(rc) => rc.borrow().clone(),
    };
    ImportResult {
        calls: inner.calls,
        failures: inner.failures,
    }
}

fn configure_sandbox(lua: &Lua, hypr_config_dir: &Path, state: SharedState) -> mlua::Result<()> {
    // `Lua::new()` (not `unsafe_new`) keeps bytecode loading and FFI out,
    // but its "safe" stdlib still includes `os`/`io`/`package` — confirmed
    // by an earlier version of this sandbox actually running
    // `os.execute("echo hi")` from a test config. Stripping them here is
    // what makes "no io/os surface" true rather than assumed.
    for dangerous in ["os", "io", "package", "dofile", "loadfile", "load"] {
        lua.globals().set(dangerous, Value::Nil)?;
    }

    // The limits below bound the two remaining ways a hand-written config
    // could misbehave during import — runaway allocation and runaway
    // looping — neither of which a config doing anything remotely normal
    // should ever approach.
    let _ = lua.set_memory_limit(MEMORY_LIMIT_BYTES);
    lua.set_hook(
        HookTriggers::new().every_nth_instruction(MAX_INSTRUCTIONS),
        |_lua, _debug| -> mlua::Result<VmState> {
            Err(mlua::Error::RuntimeError(
                "import sandbox instruction limit exceeded".to_string(),
            ))
        },
    )?;

    install_hl(lua, &state)?;
    install_require(lua, hypr_config_dir.to_path_buf(), state)?;
    Ok(())
}

fn install_hl(lua: &Lua, state: &SharedState) -> mlua::Result<()> {
    let hl = lua.create_table()?;
    for kind in ["bind", "window_rule", "workspace_rule", "monitor"] {
        let kind = kind.to_string();
        let state = state.clone();
        let kind_for_closure = kind.clone();
        hl.set(
            kind.as_str(),
            lua.create_function(move |lua, args: Variadic<Value>| {
                record_call(lua, &state, &kind_for_closure, args)
            })?,
        )?;
    }
    hl.set("dsp", make_dispatch_proxy(lua, String::new())?)?;

    // Everything else a real config calls — `hl.config`, `hl.env`,
    // `hl.animation`, `hl.on`, ... — has to exist too, even though nothing
    // imports it. Lua aborts the whole chunk on the first call to a nil
    // field, so a single unstubbed `hl.config({...})` on line 8 would sink
    // the binds and window rules further down the same file. Unknown names
    // are recorded like any other call (harmless: every adapter filters by
    // `kind`) and return an inert proxy, so a config that keeps using the
    // result — `hl.curve(...)` stored and indexed later — still evaluates.
    let meta = lua.create_table()?;
    let state = state.clone();
    meta.set(
        MetaMethod::Index.name(),
        lua.create_function(move |lua, (_t, key): (Table, String)| {
            let state = state.clone();
            lua.create_function(move |lua, args: Variadic<Value>| {
                record_call(lua, &state, &key, args)?;
                make_inert_proxy(lua)
            })
        })?,
    )?;
    hl.set_metatable(Some(meta))?;

    lua.globals().set("hl", hl)?;
    Ok(())
}

/// A value that tolerates whatever a config does with it next — indexing
/// it or calling it both yield the proxy again. Returned from stubs for
/// `hl.*` names nothing imports, where the point is only that evaluation
/// survives long enough to reach the calls that *are* importable.
fn make_inert_proxy(lua: &Lua) -> mlua::Result<Table> {
    let t = lua.create_table()?;
    let meta = lua.create_table()?;
    let for_index = t.clone();
    meta.set(
        MetaMethod::Index.name(),
        lua.create_function(move |_, (_t, _key): (Table, Value)| Ok(for_index.clone()))?,
    )?;
    let for_call = t.clone();
    meta.set(
        MetaMethod::Call.name(),
        lua.create_function(move |_, (_t, _args): (Table, Variadic<Value>)| Ok(for_call.clone()))?,
    )?;
    t.set_metatable(Some(meta))?;
    Ok(t)
}

fn record_call(
    lua: &Lua,
    state: &SharedState,
    kind: &str,
    args: Variadic<Value>,
) -> mlua::Result<Value> {
    let json_args: Vec<serde_json::Value> = args
        .iter()
        .map(|v| lua.from_value(v.clone()).unwrap_or(serde_json::Value::Null))
        .collect();
    // Level 1: the Lua code that called this Rust function — level 0 would
    // be this function itself, which (being a C function, from Lua's
    // perspective) has no line of its own.
    let line = lua.inspect_stack(1, |dbg| dbg.current_line()).flatten();
    let mut s = state.borrow_mut();
    let source_path = s.source_stack.last().cloned().unwrap_or_default();
    s.calls.push(RecordedCall {
        kind: kind.to_string(),
        source_path,
        line,
        args: json_args,
    });
    Ok(Value::Nil)
}

/// `hl.dsp.window.close()`-style chained dispatcher calls: every `.field`
/// access returns another proxy remembering the accumulated path, and
/// calling one is terminal — it returns a plain marker table
/// (`__hyprforge_dispatch` / `__hyprforge_args`), not another proxy, since
/// a dispatcher call is never itself indexed further in real configs.
/// `hl.bind`'s [`record_call`] captures that marker as one of its own
/// JSON-converted arguments, which is what lets the shortcuts adapter
/// later recover a real dispatcher path — something `hyprctl binds -j`
/// can never report (every Lua-defined bind shows there as opaque
/// `__lua`).
fn make_dispatch_proxy(lua: &Lua, path: String) -> mlua::Result<Table> {
    let t = lua.create_table()?;
    let meta = lua.create_table()?;

    let index_path = path.clone();
    meta.set(
        MetaMethod::Index.name(),
        lua.create_function(move |lua, (_t, key): (Table, String)| {
            let next = if index_path.is_empty() {
                key
            } else {
                format!("{index_path}.{key}")
            };
            make_dispatch_proxy(lua, next)
        })?,
    )?;

    let call_path = path;
    meta.set(
        MetaMethod::Call.name(),
        lua.create_function(move |lua, (_self, args): (Table, Variadic<Value>)| {
            let marker = lua.create_table()?;
            marker.set("__hyprforge_dispatch", call_path.clone())?;
            let arg_table = lua.create_table()?;
            for (i, v) in args.iter().enumerate() {
                let json: serde_json::Value = lua.from_value(v.clone()).unwrap_or(serde_json::Value::Null);
                arg_table.set(i + 1, lua.to_value(&json)?)?;
            }
            marker.set("__hyprforge_args", arg_table)?;
            Ok(marker)
        })?,
    )?;

    t.set_metatable(Some(meta))?;
    Ok(t)
}

/// Overrides Lua's own `require`. Hyprland's real `package.path`/searcher
/// setup for `require("hyprforge/window-rules")`-style paths isn't
/// documented, so rather than trying to replicate it, this implements the
/// one convention every file in this project actually needs: a module
/// path maps to `<hypr_config_dir>/<path>.lua`. Anything the user's own
/// config requires that doesn't fit that shape fails gracefully into
/// `failures` for that one file rather than the whole import.
fn install_require(lua: &Lua, hypr_dir: PathBuf, state: SharedState) -> mlua::Result<()> {
    let require_fn = lua.create_function(move |lua, module: String| -> mlua::Result<Value> {
        let path = hypr_dir.join(format!("{module}.lua"));
        {
            let s = state.borrow();
            if s.required.contains(&path) {
                return Ok(Value::Nil);
            }
        }
        state.borrow_mut().required.insert(path.clone());
        run_file(lua, &path, &state);
        Ok(Value::Nil)
    })?;
    lua.globals().set("require", require_fn)?;
    Ok(())
}

fn run_file(lua: &Lua, path: &Path, state: &SharedState) {
    let contents = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) => {
            state
                .borrow_mut()
                .failures
                .push((path.to_path_buf(), format!("could not read: {e}")));
            return;
        }
    };
    state.borrow_mut().source_stack.push(path.to_path_buf());
    let result = lua.load(&contents).set_name(path.display().to_string()).exec();
    state.borrow_mut().source_stack.pop();
    if let Err(e) = result {
        state
            .borrow_mut()
            .failures
            .push((path.to_path_buf(), e.to_string()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir_with(files: &[(&str, &str)]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        for (name, contents) in files {
            let path = dir.path().join(name);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).unwrap();
            }
            std::fs::write(path, contents).unwrap();
        }
        dir
    }

    #[test]
    fn a_plain_literal_window_rule_is_recorded() {
        let dir = dir_with(&[(
            "hyprland.lua",
            r#"hl.window_rule({ class = "^discord$", float = true })"#,
        )]);
        let result = evaluate(dir.path());
        assert!(result.failures.is_empty(), "unexpected failures: {:?}", result.failures);
        assert_eq!(result.calls.len(), 1);
        assert_eq!(result.calls[0].kind, "window_rule");
        assert_eq!(
            result.calls[0].args[0]["class"],
            serde_json::json!("^discord$")
        );
    }

    /// The line a caller needs to safely remove a single-line call after
    /// import — see `RecordedCall::line`'s doc comment.
    #[test]
    fn a_single_line_calls_line_is_captured() {
        let dir = dir_with(&[(
            "hyprland.lua",
            "-- a comment first\nhl.bind(\"SUPER + Q\", hl.dsp.window.close())\n",
        )]);
        let result = evaluate(dir.path());
        assert_eq!(result.calls[0].line, Some(2));
    }

    /// A multi-line call's `line` is where it *starts*, not where it ends
    /// — a caller checking "does this line alone both start with `hl.`
    /// and end with `)`" will correctly see this one doesn't qualify for
    /// single-line removal.
    #[test]
    fn a_multi_line_calls_line_is_its_start_not_its_end() {
        let dir = dir_with(&[(
            "hyprland.lua",
            "hl.window_rule({\n  class = \"^discord$\",\n})\n",
        )]);
        let result = evaluate(dir.path());
        assert_eq!(result.calls[0].line, Some(1));
    }

    /// The case that motivates this whole feature: `mainMod` is a
    /// variable, not a literal, and only a real interpreter resolves it —
    /// no text parser could get this right.
    #[test]
    fn a_bind_using_a_mod_key_variable_resolves_to_the_real_chord() {
        let dir = dir_with(&[(
            "hyprland.lua",
            r#"
            local mainMod = "SUPER"
            hl.bind(mainMod .. " + Q", hl.dsp.window.close())
            "#,
        )]);
        let result = evaluate(dir.path());
        assert!(result.failures.is_empty(), "unexpected failures: {:?}", result.failures);
        assert_eq!(result.calls.len(), 1);
        assert_eq!(result.calls[0].kind, "bind");
        assert_eq!(result.calls[0].args[0], serde_json::json!("SUPER + Q"));
        assert_eq!(
            result.calls[0].args[1]["__hyprforge_dispatch"],
            serde_json::json!("window.close")
        );
    }

    #[test]
    fn a_dispatcher_argument_is_captured() {
        let dir = dir_with(&[(
            "hyprland.lua",
            r#"hl.bind("SUPER + Return", hl.dsp.exec_cmd("ghostty"))"#,
        )]);
        let result = evaluate(dir.path());
        assert_eq!(
            result.calls[0].args[1]["__hyprforge_args"][0],
            serde_json::json!("ghostty")
        );
    }

    #[test]
    fn a_required_file_is_evaluated_and_calls_are_attributed_to_it() {
        let dir = dir_with(&[
            ("hyprland.lua", "require(\"binds\")\n"),
            (
                "binds.lua",
                r#"hl.bind("SUPER + C", hl.dsp.window.close())"#,
            ),
        ]);
        let result = evaluate(dir.path());
        assert!(result.failures.is_empty(), "unexpected failures: {:?}", result.failures);
        assert_eq!(result.calls.len(), 1);
        assert_eq!(result.calls[0].source_path, dir.path().join("binds.lua"));
    }

    #[test]
    fn a_missing_required_file_is_a_failure_not_a_panic() {
        let dir = dir_with(&[("hyprland.lua", "require(\"does-not-exist\")\n")]);
        let result = evaluate(dir.path());
        assert!(result.calls.is_empty());
        assert_eq!(result.failures.len(), 1);
        assert_eq!(result.failures[0].0, dir.path().join("does-not-exist.lua"));
    }

    #[test]
    fn a_syntax_error_is_a_failure_not_a_panic() {
        let dir = dir_with(&[("hyprland.lua", "this is not valid lua (((")]);
        let result = evaluate(dir.path());
        assert!(result.calls.is_empty());
        assert_eq!(result.failures.len(), 1);
    }

    /// `os`/`io` must be genuinely inaccessible, not just "not expected to
    /// be used" — an earlier version of this sandbox let `os.execute`
    /// actually run a real shell command during what's supposed to be a
    /// read-only import.
    #[test]
    fn os_and_io_are_not_reachable() {
        let dir = dir_with(&[(
            "hyprland.lua",
            "os.execute(\"true\")",
        )]);
        let result = evaluate(dir.path());
        assert!(result.calls.is_empty());
        assert_eq!(result.failures.len(), 1);
        assert!(
            result.failures[0].1.contains("nil"),
            "expected a nil-global error, got: {:?}",
            result.failures[0].1
        );
    }

    /// The chunk still aborts on the first unsupported construct (Lua has
    /// no implicit per-statement recovery) — but that's reported as a
    /// failure for this one file, not a panic, and a sibling `require()`d
    /// file with no such construct still imports normally.
    #[test]
    fn an_unsupported_construct_fails_only_its_own_file() {
        let dir = dir_with(&[
            (
                "hyprland.lua",
                "require(\"good\")\nrequire(\"bad\")\n",
            ),
            (
                "good.lua",
                r#"hl.bind("SUPER + C", hl.dsp.window.close())"#,
            ),
            ("bad.lua", "os.execute(\"true\")\nhl.bind(\"SUPER + D\", hl.dsp.window.close())"),
        ]);
        let result = evaluate(dir.path());
        // Lua aborts the whole chunk on the first runtime error (there's no
        // implicit per-statement recovery), so only the first bind lands —
        // this pins that behavior rather than assuming otherwise.
        assert_eq!(result.calls.len(), 1);
        assert_eq!(result.failures.len(), 1);
    }

    #[test]
    fn an_infinite_loop_is_stopped_by_the_instruction_limit_not_a_hang() {
        let dir = dir_with(&[("hyprland.lua", "while true do end")]);
        let result = evaluate(dir.path());
        assert!(result.calls.is_empty());
        assert_eq!(result.failures.len(), 1);
        assert!(
            result.failures[0].1.contains("instruction limit"),
            "got: {:?}",
            result.failures[0].1
        );
    }

    /// The shape of a real hand-written config: importable calls sit below
    /// a pile of `hl.*` calls nothing imports. Those must not abort the
    /// chunk — before they were stubbed, an `hl.config` on line 8 meant
    /// zero shortcuts imported from an otherwise fine file.
    #[test]
    fn an_hl_function_nothing_imports_does_not_sink_the_rest_of_the_file() {
        let dir = dir_with(&[(
            "hyprland.lua",
            r#"
            hl.config({ debug = { disable_logs = false } })
            hl.env("XCURSOR_SIZE", "24")
            hl.on("hyprland.start", function() hl.exec_cmd("waybar") end)
            local curve = hl.curve("quick", { type = "bezier" })
            hl.animation({ leaf = "global", bezier = curve.name })
            hl.bind("SUPER + Q", hl.dsp.window.close())
            "#,
        )]);
        let result = evaluate(dir.path());
        assert!(result.failures.is_empty(), "unexpected failures: {:?}", result.failures);
        let bind = result
            .calls
            .iter()
            .find(|c| c.kind == "bind")
            .expect("the bind below the unimported calls should still be recorded");
        assert_eq!(bind.args[0], serde_json::json!("SUPER + Q"));
    }

    #[test]
    fn requiring_the_same_file_twice_only_evaluates_it_once() {
        let dir = dir_with(&[
            ("hyprland.lua", "require(\"binds\")\nrequire(\"binds\")\n"),
            (
                "binds.lua",
                r#"hl.bind("SUPER + C", hl.dsp.window.close())"#,
            ),
        ]);
        let result = evaluate(dir.path());
        assert_eq!(result.calls.len(), 1);
    }
}
