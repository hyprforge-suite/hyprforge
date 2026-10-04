//! "Show in folder" from other apps opens Files.
//!
//! Browsers and editors call `org.freedesktop.FileManager1`, and the
//! session bus starts whichever service file names it. A user's own file
//! in `~/.local/share/dbus-1/services/` is read before `/usr/share`'s,
//! which is how this wins over `nemo.FileManager1.service` there. The
//! installer writes the same file for a source install; this is the same
//! claim for a package install, and the same `ReloadConfig` after it.

use super::{Applied, Cx};
use crate::record::Change;
use crate::state::State;
use std::path::Path;

const NAME: &str = "org.freedesktop.FileManager1";

/// The file, exactly as the installer writes it.
fn wanted(binary: &Path) -> String {
    format!("[D-BUS Service]\nName={NAME}\nExec={} --dbus-service\n", binary.display())
}

/// Whether `text` already sends the name to an installed Files: its
/// `Exec=` runs a `hyprforge-files` that exists, with `--dbus-service`.
/// Not a byte comparison — the installer and a package put Files in
/// different places, and either is a correct answer.
fn points_at_files(text: &str, installed: &Path) -> bool {
    let names_us = text.lines().any(|l| l.trim() == format!("Name={NAME}"));
    let exec = text.lines().find_map(|l| l.trim().strip_prefix("Exec="));
    let runs_files = exec.is_some_and(|exec| {
        let mut words = exec.split_whitespace();
        let program = words.next().map(Path::new);
        program.is_some_and(|p| {
            p == installed
                || (p.file_name().is_some_and(|n| n == "hyprforge-files") && p.exists())
        }) && words.any(|w| w == "--dbus-service")
    });
    names_us && runs_files
}

fn read(cx: &Cx<'_>) -> Result<Option<String>, String> {
    let path = cx.env.file_manager1_service();
    match std::fs::read_to_string(&path) {
        Ok(text) => Ok(Some(text)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("couldn't read {}: {e}", path.display())),
    }
}

pub(super) fn check(cx: &Cx<'_>) -> State {
    // `Item::check` has already made sure there is one.
    let installed = cx.sys.find_binary("hyprforge-files").unwrap_or_default();
    match read(cx) {
        Err(e) => State::Unknown { why: e },
        Ok(Some(text)) if points_at_files(&text, &installed) => State::Done,
        Ok(Some(_)) => State::Todo {
            what: "Replace your own FileManager1 service with one that starts Files".into(),
        },
        Ok(None) => State::Todo { what: "Claim org.freedesktop.FileManager1 for Files".into() },
    }
}

fn reload(cx: &Cx<'_>) -> Option<String> {
    cx.sys.reload_session_bus().err().map(|e| {
        format!("saved, but the session bus didn't reload ({e}) — log out and back in to apply")
    })
}

pub(super) fn apply(cx: &Cx<'_>) -> Result<Applied, String> {
    let binary = cx
        .sys
        .find_binary("hyprforge-files")
        .ok_or_else(|| "hyprforge-files isn't installed".to_string())?;
    let previous = read(cx)?;
    let path = cx.env.file_manager1_service();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("couldn't create {}: {e}", dir.display()))?;
    }
    hyprforge_paths::write_atomic(&path, &wanted(&binary))
        .map_err(|e| format!("couldn't write {}: {e}", path.display()))?;
    Ok(Applied { change: Change::FileManager1 { previous }, note: reload(cx) })
}

/// Puts back the file that was there, or removes ours — unless it has
/// been replaced since by something that no longer starts Files.
pub(super) fn undo(cx: &Cx<'_>, previous: Option<&str>) -> Result<Option<String>, String> {
    let path = cx.env.file_manager1_service();
    let Some(now) = read(cx)? else { return Ok(None) };
    if !now.contains("hyprforge-files") {
        return Ok(Some(format!("{} was changed since setup wrote it, so it was left alone", path.display())));
    }
    match previous {
        Some(text) => hyprforge_paths::write_atomic(&path, text)
            .map_err(|e| format!("couldn't write {}: {e}", path.display()))?,
        None => std::fs::remove_file(&path)
            .map_err(|e| format!("couldn't remove {}: {e}", path.display()))?,
    }
    Ok(reload(cx))
}
