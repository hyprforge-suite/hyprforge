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
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, Instant};

/// Generous but finite: real configs are a few hundred lines, not a
/// computation. This exists purely so a pathological or accidentally
/// -infinite config can't hang an import — see [`evaluate`].
const MAX_INSTRUCTIONS: u32 = 5_000_000;
/// How often the hook runs. Small enough that, once the limit has tripped,
/// code that caught the error gets at most this many instructions before
/// the next one is raised — see [`Limit`].
const HOOK_EVERY: u32 = 1_000;
/// The same bound in wall-clock time, for a config that stays under the
/// instruction count by spending its time somewhere instructions are not
/// counted (a C function asked to do a lot of work per call).
const DEADLINE: Duration = Duration::from_secs(5);
const MEMORY_LIMIT_BYTES: usize = 64 * 1024 * 1024;
/// The longest string the pattern functions will search. Lua's matcher
/// backtracks in C, where the instruction hook never runs, and its cost
/// is polynomial in the subject — `(".-"):rep(3) .. "b"` against a
/// megabyte of `a` never returns. Nothing a hand-written config matches
/// against (a key name, a monitor description) comes close to this.
///
/// This bounds the subject, not the pattern, so it narrows the problem
/// rather than closing it: a pattern of many backtracking items against a
/// subject under the cap can still run for a long time without the hook
/// seeing it. Closing that needs the evaluation in a process that can be
/// killed; the caller's own timeout is the backstop meanwhile.
const MAX_PATTERN_SUBJECT: usize = 4 * 1024;

/// The instruction and time limits, shared by the hook and by the
/// wrappers that stop a config catching them.
///
/// Raising an error from the hook is only half a limit: `pcall`, `xpcall`
/// and `coroutine.resume` all catch errors, so
/// `while true do pcall(function() while true do end end) end` caught the
/// limit once per pass and ran forever. Once tripped, the limit stays
/// tripped — every later hook raises again — and the catching functions
/// are wrapped to re-raise it the moment they return (see
/// [`install_limit_wrappers`]).
struct Limit {
    hooks: Cell<u32>,
    started: Instant,
    tripped: Cell<Option<&'static str>>,
}

impl Limit {
    fn new() -> Self {
        Limit { hooks: Cell::new(0), started: Instant::now(), tripped: Cell::new(None) }
    }

    /// Counts one hook call, trips the limit if it has been reached, and
    /// raises if it is tripped — now or at any earlier point.
    fn tick(&self) -> mlua::Result<()> {
        if self.tripped.get().is_none() {
            let n = self.hooks.get().saturating_add(1);
            self.hooks.set(n);
            if n >= MAX_INSTRUCTIONS / HOOK_EVERY {
                self.tripped.set(Some("import sandbox instruction limit exceeded"));
            } else if self.started.elapsed() > DEADLINE {
                self.tripped.set(Some("import sandbox time limit exceeded"));
            }
        }
        self.check()
    }

    fn check(&self) -> mlua::Result<()> {
        match self.tripped.get() {
            Some(reason) => Err(mlua::Error::RuntimeError(reason.to_string())),
            None => Ok(()),
        }
    }
}

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
    /// Chunk name -> the file it was loaded from, so a call is attributed
    /// to the file its own frame belongs to (see [`record_call`]).
    chunks: HashMap<String, PathBuf>,
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
    let mut inner = match Rc::try_unwrap(state) {
        Ok(cell) => cell.into_inner(),
        // Shouldn't happen once `lua` (and every closure it owned) has
        // dropped — falls back to cloning rather than panicking a GUI
        // action over an Rc-accounting edge case.
        Err(rc) => rc.borrow().clone(),
    };
    forget_shared_lines(&mut inner.calls);
    ImportResult {
        calls: inner.calls,
        failures: inner.failures,
    }
}

/// A line is offered for removal only if exactly one call in the whole
/// evaluation came from it.
///
/// `for i = 1, 9 do hl.bind(...) end` is nine calls from one line, and so
/// is `hl.bind(a, ...) hl.bind(b, ...)`. Removing that line because some
/// of them were imported deletes the ones that were not, so none of them
/// names it.
fn forget_shared_lines(calls: &mut [RecordedCall]) {
    let mut counts: HashMap<(PathBuf, usize), usize> = HashMap::new();
    for call in calls.iter() {
        if let Some(line) = call.line {
            *counts.entry((call.source_path.clone(), line)).or_default() += 1;
        }
    }
    for call in calls.iter_mut() {
        if let Some(line) = call.line {
            if counts.get(&(call.source_path.clone(), line)).copied().unwrap_or(0) > 1 {
                call.line = None;
            }
        }
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
    let limit = Rc::new(Limit::new());
    // The *global* hook, not `set_hook`: mlua keys a per-thread hook on the
    // thread it was set for, and a coroutine the config creates inherits
    // the hook function but finds no entry for itself — so mlua removes
    // the hook from it, and a loop inside `coroutine.wrap` ran unlimited.
    let for_hook = limit.clone();
    lua.set_global_hook(
        HookTriggers::new().every_nth_instruction(HOOK_EVERY),
        move |_lua, _debug| -> mlua::Result<VmState> {
            for_hook.tick()?;
            Ok(VmState::Continue)
        },
    )?;
    install_limit_wrappers(lua, &limit)?;

    install_hl(lua, &state)?;
    install_require(lua, hypr_config_dir.to_path_buf(), state, limit)?;
    Ok(())
}

/// Wraps everything a config could catch the limit with, so that it is
/// raised again the moment the catching function returns, and the pattern
/// functions, so their subject is bounded (see [`MAX_PATTERN_SUBJECT`]).
///
/// Written in Lua so the wrappers are ordinary Lua functions: a `pcall`
/// reimplemented in Rust would make every protected call unyieldable,
/// which the real one is not. The originals are captured as upvalues
/// here, before any config code runs, so nothing the config does to the
/// globals afterwards reaches them.
fn install_limit_wrappers(lua: &Lua, limit: &Rc<Limit>) -> mlua::Result<()> {
    let for_check = limit.clone();
    let check = lua.create_function(move |_, ()| for_check.check())?;
    let check_subject = lua.create_function(|_, subject: Value| match subject {
        Value::String(s) if s.as_bytes().len() > MAX_PATTERN_SUBJECT => Err(mlua::Error::RuntimeError(format!(
            "import sandbox refuses to pattern-match a string over {MAX_PATTERN_SUBJECT} bytes"
        ))),
        _ => Ok(()),
    })?;
    lua.load(
        r#"
        local check, check_subject = ...
        local pack, unpack = table.pack, table.unpack
        local function settle(r) check() return unpack(r, 1, r.n) end
        local raw_pcall, raw_xpcall = pcall, xpcall
        pcall = function(...) return settle(pack(raw_pcall(...))) end
        xpcall = function(...) return settle(pack(raw_xpcall(...))) end
        local raw_resume, raw_wrap = coroutine.resume, coroutine.wrap
        coroutine.resume = function(...) return settle(pack(raw_resume(...))) end
        coroutine.wrap = function(f)
            local g = raw_wrap(f)
            return function(...) return settle(pack(g(...))) end
        end
        for _, name in ipairs({ "find", "match", "gmatch", "gsub" }) do
            local raw = string[name]
            string[name] = function(s, ...) check_subject(s) return raw(s, ...) end
        end
        "#,
    )
    .set_name("=hyprforge sandbox")
    .call::<()>((check, check_subject))
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
    //
    // The file comes from that frame too, never from whichever file is
    // currently running: a helper defined in `helpers.lua` and called from
    // `hyprland.lua` executes at a line of `helpers.lua`, and pairing that
    // line with the running file once named an unrelated line of
    // `hyprland.lua` for removal.
    //
    // And the line is only kept for a call made from a file's main chunk.
    // A call inside a function body — or reached by a tail call, which
    // replaces the frame that made it, or through a C function such as
    // `pcall` — is not one a single line removal can safely undo.
    let tail_called = lua.inspect_stack(0, |dbg| dbg.is_tail_call()).unwrap_or(true);
    let (line, chunk) = lua
        .inspect_stack(1, |dbg| {
            let source = dbg.source();
            let line = (source.what == "main").then(|| dbg.current_line()).flatten();
            (line, source.source.map(|s| s.into_owned()))
        })
        .unwrap_or((None, None));
    let line = if tail_called { None } else { line };
    let mut s = state.borrow_mut();
    let source_path = chunk
        .and_then(|name| s.chunks.get(&name).cloned())
        .or_else(|| s.source_stack.last().cloned())
        .unwrap_or_default();
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
fn install_require(lua: &Lua, hypr_dir: PathBuf, state: SharedState, limit: Rc<Limit>) -> mlua::Result<()> {
    let require_fn = lua.create_function(move |lua, module: String| -> mlua::Result<Value> {
        // A required file's failure is caught here and reported for that
        // file alone, which is right for a syntax error and wrong for the
        // limit: that has to stop the requirer too.
        limit.check()?;
        let path = hypr_dir.join(format!("{module}.lua"));
        {
            let s = state.borrow();
            if s.required.contains(&path) {
                return Ok(Value::Nil);
            }
        }
        state.borrow_mut().required.insert(path.clone());
        run_file(lua, &path, &state);
        limit.check()?;
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
    // `@` marks the name as a file path, which is also what makes Lua's
    // own error messages read `/path/to/file.lua:3:` rather than quoting it
    // as a string. The name is how [`record_call`] maps a frame back to
    // its file.
    let chunk_name = format!("@{}", path.display());
    {
        let mut s = state.borrow_mut();
        s.chunks.insert(chunk_name.clone(), path.to_path_buf());
        s.source_stack.push(path.to_path_buf());
    }
    let result = lua.load(&contents).set_name(chunk_name).exec();
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

    /// `for i = 1, 9 do hl.bind(...) end` is nine calls from one line.
    /// Removing that line for a ticked subset deletes the unticked binds
    /// with it, so a line any other call also came from has no line at all.
    #[test]
    fn a_line_that_made_several_calls_is_never_offered_for_removal() {
        let dir = dir_with(&[(
            "hyprland.lua",
            "for i = 1, 3 do hl.bind(\"SUPER + \" .. i, hl.dsp.window.close()) end\n\
             hl.bind(\"SUPER + Q\", hl.dsp.window.close())\n",
        )]);
        let result = evaluate(dir.path());
        assert!(result.failures.is_empty(), "unexpected failures: {:?}", result.failures);
        assert_eq!(result.calls.len(), 4);
        for call in &result.calls[..3] {
            assert_eq!(call.line, None, "a looped call must not name its line");
        }
        assert_eq!(result.calls[3].line, Some(2), "a call alone on its line still does");
    }

    /// A helper defined in another file: the call executes in the helper's
    /// file at the helper's line. Attributing it to whichever file was
    /// running *and* the helper's line named an unrelated line of
    /// `hyprland.lua` for removal.
    #[test]
    fn a_call_inside_a_function_is_attributed_to_its_own_file_and_never_to_a_line() {
        let dir = dir_with(&[
            (
                "hyprland.lua",
                "hl.config({ general = { gaps_in = 5 } })\nrequire(\"helpers\")\nquit_bind(\"SUPER + Q\")\n",
            ),
            (
                "helpers.lua",
                "function quit_bind(key) hl.bind(key, hl.dsp.window.close()) end\n",
            ),
        ]);
        let result = evaluate(dir.path());
        assert!(result.failures.is_empty(), "unexpected failures: {:?}", result.failures);
        let bind = result.calls.iter().find(|c| c.kind == "bind").expect("the bind is recorded");
        assert_eq!(bind.source_path, dir.path().join("helpers.lua"));
        assert_eq!(bind.line, None);
    }

    /// The same, for a function defined and called in one file: the call
    /// ran once, but its line is inside a body that could run again.
    #[test]
    fn a_call_inside_a_local_function_names_no_line() {
        let dir = dir_with(&[(
            "hyprland.lua",
            "local function b(k)\n  hl.bind(k, hl.dsp.window.close())\nend\nb(\"SUPER + Q\")\n",
        )]);
        let result = evaluate(dir.path());
        assert_eq!(result.calls.len(), 1);
        assert_eq!(result.calls[0].line, None);
    }

    /// The instruction limit is an error, and `pcall` catches errors. A
    /// limit that a config can catch and carry on from is no limit.
    #[test]
    fn a_pcall_loop_cannot_outlive_the_instruction_limit() {
        let started = std::time::Instant::now();
        let dir = dir_with(&[(
            "hyprland.lua",
            "while true do pcall(function() while true do end end) end",
        )]);
        let result = evaluate(dir.path());
        assert_eq!(result.failures.len(), 1, "{:?}", result.failures);
        assert!(started.elapsed() < std::time::Duration::from_secs(30));
    }

    #[test]
    fn an_xpcall_or_coroutine_loop_cannot_outlive_the_instruction_limit() {
        for body in [
            "while true do xpcall(function() while true do end end, function(e) return e end) end",
            "while true do coroutine.resume(coroutine.create(function() while true do end end)) end",
            "while true do pcall(coroutine.wrap(function() while true do end end)) end",
        ] {
            let started = std::time::Instant::now();
            let dir = dir_with(&[("hyprland.lua", body)]);
            let result = evaluate(dir.path());
            assert_eq!(result.failures.len(), 1, "{body}: {:?}", result.failures);
            assert!(started.elapsed() < std::time::Duration::from_secs(30), "{body}");
        }
    }

    /// Pattern matching runs in C, where the instruction hook never fires,
    /// and backtracking is polynomial in the subject. A subject no
    /// hand-written config would ever match against is refused.
    #[test]
    fn a_huge_pattern_subject_is_refused_rather_than_matched() {
        let dir = dir_with(&[(
            "hyprland.lua",
            "local s = string.rep(\"a\", 1000000)\nlocal _ = s:find(\".-.-.-b\")\n",
        )]);
        let started = std::time::Instant::now();
        let result = evaluate(dir.path());
        assert_eq!(result.failures.len(), 1, "{:?}", result.failures);
        assert!(started.elapsed() < std::time::Duration::from_secs(30));
    }

    /// The wrappers must not change what an ordinary config sees.
    #[test]
    fn pcall_and_string_functions_behave_as_usual_below_the_limits() {
        let dir = dir_with(&[(
            "hyprland.lua",
            r#"
            local ok, err = pcall(error, "boom", 0)
            assert(ok == false and err == "boom")
            local a, b, c = pcall(function() return 1, nil, 3 end)
            assert(a == true and b == 1 and c == nil)
            local co = coroutine.wrap(function(x) coroutine.yield(x + 1) end)
            assert(co(1) == 2)
            assert(("SUPER + Q"):match("^(%w+)") == "SUPER")
            assert(string.gsub("a-b", "-", "+") == "a+b")
            hl.bind("SUPER + Q", hl.dsp.window.close())
            "#,
        )]);
        let result = evaluate(dir.path());
        assert!(result.failures.is_empty(), "unexpected failures: {:?}", result.failures);
        assert_eq!(result.calls.len(), 1);
    }

    /// A Lua array reaches the importers as a JSON array, not as an
    /// object keyed "1" and "2" — the shape `hyprforge-core` reads a
    /// vec2 (`shadow.offset = {2, 3}`) from. Checked here because it is
    /// mlua's choice, not this crate's.
    #[test]
    fn a_lua_array_is_recorded_as_a_json_array() {
        let dir = dir_with(&[(
            "hyprland.lua",
            r#"hl.config({ decoration = { shadow = { offset = { 2, 3.5 } } } })"#,
        )]);
        let result = evaluate(dir.path());
        assert!(result.failures.is_empty(), "unexpected failures: {:?}", result.failures);
        let offset = &result.calls[0].args[0]["decoration"]["shadow"]["offset"];
        assert_eq!(offset, &serde_json::json!([2, 3.5]));
    }

}
