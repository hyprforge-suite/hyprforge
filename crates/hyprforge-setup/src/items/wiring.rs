//! The `require()` lines that join Hyprforge's generated files to the
//! user's `hyprland.lua`.
//!
//! The same opening move the Settings app makes as each page opens —
//! `hyprforge_core::lua_setup::bootstrap` — run for every module at once,
//! so a user who installs and never opens Settings still has binds that
//! load. Placement is each module's own decision (keybinds last, window
//! rules first); this only runs it.

use super::{Applied, Cx};
use crate::record::Change;
use crate::state::State;
use crate::Env;
use hyprforge_core::lua_setup::{self, HyprConfig, ModuleSetup, Placement, SetupPlan};
use std::path::PathBuf;

/// One module that owns a `require()`d file.
struct Module {
    name: &'static str,
    require_line: &'static str,
    placement: Placement,
    lua_path: fn(&Env) -> PathBuf,
    /// The module's "nothing configured" output: what the require line
    /// points at until something is saved. `require()` of a missing file
    /// is an error on the next reload, so it is written eagerly.
    empty: fn() -> String,
}

fn modules() -> [Module; 6] {
    [
        Module {
            name: "keybinds",
            require_line: hyprforge_shortcuts::setup::REQUIRE_LINE,
            placement: hyprforge_shortcuts::setup::PLACEMENT,
            lua_path: Env::keybinds_lua,
            empty: || hyprforge_shortcuts::codegen::generate(&[]),
        },
        Module {
            name: "window rules",
            require_line: hyprforge_windowrules::setup::REQUIRE_LINE,
            placement: hyprforge_windowrules::setup::PLACEMENT,
            lua_path: Env::window_rules_lua,
            empty: || hyprforge_windowrules::codegen::generate(&[], &[]),
        },
        Module {
            name: "input",
            require_line: hyprforge_input::setup::REQUIRE_LINE,
            placement: hyprforge_input::setup::PLACEMENT,
            lua_path: Env::input_lua,
            empty: || hyprforge_input::apply::generate(&Default::default()),
        },
        Module {
            name: "system",
            require_line: hyprforge_system::setup::REQUIRE_LINE,
            placement: hyprforge_system::setup::PLACEMENT,
            lua_path: Env::system_lua,
            empty: || hyprforge_system::apply::generate(&Default::default()),
        },
        Module {
            name: "session",
            require_line: hyprforge_session::setup::REQUIRE_LINE,
            placement: hyprforge_session::setup::PLACEMENT,
            lua_path: Env::session_lua,
            empty: || hyprforge_session::apply::generate(&Default::default(), &[]),
        },
        Module {
            name: "appearance",
            require_line: hyprforge_appearance::setup::REQUIRE_LINE,
            placement: hyprforge_appearance::setup::PLACEMENT,
            lua_path: Env::appearance_lua,
            empty: || hyprforge_appearance::apply::generate(&Default::default(), None),
        },
    ]
}

/// The modules whose line is missing from `contents`, or whose generated
/// file does not exist yet.
fn missing<'m>(env: &Env, contents: &str, modules: &'m [Module]) -> Vec<&'m Module> {
    modules
        .iter()
        .filter(|m| {
            lua_setup::detect(contents, m.require_line, m.placement) != SetupPlan::AlreadyPresent
                || !(m.lua_path)(env).exists()
        })
        .collect()
}

pub(super) fn check(cx: &Cx<'_>) -> State {
    let modules = modules();
    match lua_setup::discover(&cx.env.hypr_dir()) {
        HyprConfig::ConfOnly(path) => State::Unavailable {
            why: format!(
                "{} is the older hyprland.conf format; Hyprforge's generated files need \
                 hyprland.lua (Hyprland 0.55 and later)",
                path.display()
            ),
        },
        HyprConfig::Missing => State::Todo {
            what: format!(
                "Create {} requiring Hyprforge's modules",
                cx.env.hyprland_lua().display()
            ),
        },
        HyprConfig::Lua(path) => match std::fs::read_to_string(&path) {
            Err(e) => State::Unknown { why: format!("couldn't read {}: {e}", path.display()) },
            Ok(contents) => {
                let missing = missing(cx.env, &contents, &modules);
                if missing.is_empty() {
                    State::Done
                } else {
                    let names: Vec<&str> = missing.iter().map(|m| m.name).collect();
                    State::Todo {
                        what: format!("Add require lines to hyprland.lua for {}", names.join(", ")),
                    }
                }
            }
        },
    }
}

pub(super) fn apply(cx: &Cx<'_>) -> Result<Applied, String> {
    let env = cx.env;
    let path = env.hyprland_lua();
    let created_config = matches!(lua_setup::discover(&env.hypr_dir()), HyprConfig::Missing);
    let before = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(format!("couldn't read {}: {e}", path.display())),
    };
    let mut inserted = Vec::new();
    for module in modules() {
        let had_line = lua_setup::detect(&before, module.require_line, module.placement)
            == SetupPlan::AlreadyPresent;
        let state = lua_setup::bootstrap(
            &env.hypr_dir(),
            &path,
            ModuleSetup {
                require_line: module.require_line,
                placement: module.placement,
                generated: ((module.lua_path)(env), (module.empty)()),
            },
        );
        if let Some(error) = state.error {
            // A failed apply records nothing in setup.toml, so the lines
            // already inserted would be unexplained; say so here instead.
            return Err(if inserted.is_empty() {
                error
            } else {
                format!("{error} (after adding {} line(s), which stay)", inserted.len())
            });
        }
        if !had_line {
            inserted.push(module.require_line.to_string());
        }
    }
    Ok(Applied { change: Change::Wiring { inserted, created_config }, note: None })
}

/// Removes exactly the lines setup inserted, wherever they are now. The
/// marker comments around them stay: they are harmless, and the next
/// install puts its lines back between them.
/// `lines` without any Hyprforge-managed require block that is now empty:
/// an opening marker followed directly by its closing one. Repeated until
/// none is left, because an earlier version could nest one block inside
/// the other and an undo of both left the pairs inside each other.
fn without_empty_blocks(mut lines: Vec<&str>) -> Vec<&str> {
    let is_start = |l: &str| l.trim().starts_with("-- Hyprforge-managed requires (");
    let is_end = |l: &str| l.trim() == "-- end Hyprforge-managed requires";
    while let Some(i) = lines.windows(2).position(|w| is_start(w[0]) && is_end(w[1])) {
        lines.drain(i..i + 2);
    }
    lines
}

pub(super) fn undo(cx: &Cx<'_>, inserted: &[String]) -> Result<Option<String>, String> {
    let path = cx.env.hyprland_lua();
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("couldn't read {}: {e}", path.display())),
    };
    let kept: Vec<&str> =
        text.lines().filter(|l| !inserted.iter().any(|ours| l.trim() == ours.trim())).collect();
    let kept = without_empty_blocks(kept);
    let mut out = kept.join("\n");
    if text.ends_with('\n') {
        out.push('\n');
    }
    if out != text {
        hyprforge_paths::write_atomic(&path, &out)
            .map_err(|e| format!("couldn't write {}: {e}", path.display()))?;
    }
    Ok(None)
}

#[cfg(test)]
mod empty_block_tests {
    use super::without_empty_blocks;

    /// What undo left behind before: two empty pairs inside each other.
    /// Both go, and nothing of the user's is touched.
    #[test]
    fn undo_leaves_no_empty_marker_blocks_behind() {
        let lines = vec![
            "hl.bind(x)",
            "-- Hyprforge-managed requires (evaluated last) — do not edit by hand.",
            "-- Hyprforge-managed requires (evaluated first) — do not edit by hand.",
            "-- end Hyprforge-managed requires",
            "-- end Hyprforge-managed requires",
            "-- mine",
        ];
        assert_eq!(without_empty_blocks(lines), ["hl.bind(x)", "-- mine"]);
        let kept = vec![
            "-- Hyprforge-managed requires (evaluated last) — do not edit by hand.",
            "require(\"theirs\")",
            "-- end Hyprforge-managed requires",
        ];
        assert_eq!(without_empty_blocks(kept.clone()), kept, "a block still holding a line stays");
    }
}
