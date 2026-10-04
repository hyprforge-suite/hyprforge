//! Files' open/save dialog for every app that asks the portal.
//!
//! `~/.config/xdg-desktop-portal/hyprland-portals.conf` gets
//! `org.freedesktop.impl.portal.FileChooser=hyprforge` in `[preferred]`.
//! Every other line stays. The portal reads only the *first*
//! `hyprland-portals.conf` it finds, so a user file holding only our line
//! would drop the system's choices for screenshots and screen sharing —
//! which is why a `default=` line is kept, or added as
//! `default=hyprland;gtk` when there is none. Undo removes our line and
//! only what was added to make room for it.

use super::{Applied, Cx};
use crate::record::Change;
use crate::state::State;

const SECTION: &str = "[preferred]";
const KEY: &str = "org.freedesktop.impl.portal.FileChooser";
const VALUE: &str = "hyprforge";
const DEFAULT_LINE: &str = "default=hyprland;gtk";
const PORTAL_UNIT: &str = "xdg-desktop-portal.service";

/// What [`with_file_chooser`] changed, for [`without_file_chooser`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortalEdit {
    pub added_section: bool,
    pub added_default: bool,
    /// The FileChooser value that was replaced, if there was one.
    pub previous: Option<String>,
}

/// `key=value` split, for a line that is one.
fn key_value(line: &str) -> Option<(&str, &str)> {
    let line = line.trim();
    if line.starts_with('#') || line.starts_with(';') {
        return None;
    }
    let (k, v) = line.split_once('=')?;
    Some((k.trim(), v.trim()))
}

/// The `[preferred]` section's line range: `(header, end)` with `end`
/// exclusive — the next section's header, or the end of the file.
fn section(lines: &[String]) -> Option<(usize, usize)> {
    let start = lines.iter().position(|l| l.trim() == SECTION)?;
    let end = lines[start + 1..]
        .iter()
        .position(|l| l.trim_start().starts_with('['))
        .map(|o| start + 1 + o)
        .unwrap_or(lines.len());
    Some((start, end))
}

fn chooser_in(lines: &[String]) -> Option<(usize, String)> {
    let (start, end) = section(lines)?;
    (start + 1..end).find_map(|i| match key_value(&lines[i]) {
        Some((k, v)) if k == KEY => Some((i, v.to_string())),
        _ => None,
    })
}

fn join(lines: &[String]) -> String {
    let mut out = lines.join("\n");
    if !out.is_empty() {
        out.push('\n');
    }
    out
}

/// Whether `text` already asks for Files' dialog.
pub fn has_file_chooser(text: &str) -> bool {
    let lines: Vec<String> = text.lines().map(str::to_string).collect();
    chooser_in(&lines).is_some_and(|(_, v)| v == VALUE)
}

/// `text` with Files' dialog preferred, and what had to change for it.
pub fn with_file_chooser(text: &str) -> (String, PortalEdit) {
    let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
    let mut edit = PortalEdit { added_section: false, added_default: false, previous: None };

    let (start, end) = match section(&lines) {
        Some(range) => range,
        None => {
            if lines.last().is_some_and(|l| !l.trim().is_empty()) {
                lines.push(String::new());
            }
            lines.push(SECTION.to_string());
            edit.added_section = true;
            (lines.len() - 1, lines.len())
        }
    };
    let mut end = end;
    let has_default =
        (start + 1..end).any(|i| key_value(&lines[i]).is_some_and(|(k, _)| k == "default"));
    if !has_default {
        lines.insert(start + 1, DEFAULT_LINE.to_string());
        edit.added_default = true;
        end += 1;
    }
    let ours = format!("{KEY}={VALUE}");
    match chooser_in(&lines) {
        Some((i, value)) => {
            edit.previous = Some(value);
            lines[i] = ours;
        }
        None => {
            // After the section's last real line, so a blank line that
            // separates it from the next section stays where it was.
            let mut at = end;
            while at > start + 1 && lines[at - 1].trim().is_empty() {
                at -= 1;
            }
            lines.insert(at, ours);
        }
    }
    (join(&lines), edit)
}

/// `text` with [`with_file_chooser`]'s change taken back out: the old
/// FileChooser value restored or our line removed, then the `default=`
/// line and the section header removed if they were ours and nothing else
/// came to depend on them.
pub fn without_file_chooser(text: &str, edit: &PortalEdit) -> String {
    let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
    match chooser_in(&lines) {
        Some((i, value)) if value == VALUE => match &edit.previous {
            Some(old) => lines[i] = format!("{KEY}={old}"),
            None => {
                lines.remove(i);
            }
        },
        // Changed since setup set it: the user's, and left alone.
        _ => return join(&lines),
    }
    if edit.added_default {
        if let Some((start, end)) = section(&lines) {
            if let Some(i) = (start + 1..end).find(|&i| lines[i].trim() == DEFAULT_LINE) {
                lines.remove(i);
            }
        }
    }
    if edit.added_section {
        if let Some((start, end)) = section(&lines) {
            if (start + 1..end).all(|i| lines[i].trim().is_empty()) {
                lines.drain(start..end);
                while lines.last().is_some_and(|l| l.trim().is_empty()) {
                    lines.pop();
                }
            }
        }
    }
    join(&lines)
}

fn read(cx: &Cx<'_>) -> Result<Option<String>, String> {
    let path = cx.env.portals_conf();
    match std::fs::read_to_string(&path) {
        Ok(text) => Ok(Some(text)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("couldn't read {}: {e}", path.display())),
    }
}

pub(super) fn check(cx: &Cx<'_>) -> State {
    match read(cx) {
        Err(e) => State::Unknown { why: e },
        Ok(Some(text)) if has_file_chooser(&text) => State::Done,
        Ok(text) => {
            let lines: Vec<String> =
                text.unwrap_or_default().lines().map(str::to_string).collect();
            State::Todo {
                what: match chooser_in(&lines) {
                    Some((_, other)) => format!("Use Files' dialog instead of {other}'s"),
                    None => "Prefer Files' dialog in hyprland-portals.conf".to_string(),
                },
            }
        }
    }
}

/// Restarts the portal so it reads the change — `try-restart`, so a
/// portal that was not running is not started by this.
fn restart(cx: &Cx<'_>) -> Option<String> {
    cx.sys.try_restart(PORTAL_UNIT).err().map(|e| {
        format!("saved, but the portal didn't restart ({e}) — log out and back in to apply")
    })
}

pub(super) fn apply(cx: &Cx<'_>) -> Result<Applied, String> {
    let existing = read(cx)?;
    let created_file = existing.is_none();
    let (text, edit) = with_file_chooser(existing.as_deref().unwrap_or(""));
    let path = cx.env.portals_conf();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("couldn't create {}: {e}", dir.display()))?;
    }
    hyprforge_paths::write_atomic(&path, &text)
        .map_err(|e| format!("couldn't write {}: {e}", path.display()))?;
    Ok(Applied {
        change: Change::Portal {
            created_file,
            added_section: edit.added_section,
            added_default: edit.added_default,
            previous: edit.previous,
        },
        note: restart(cx),
    })
}

pub(super) fn undo(cx: &Cx<'_>, change: &Change) -> Result<Option<String>, String> {
    let Change::Portal { created_file, added_section, added_default, previous } = change else {
        return Err("not a portal change".into());
    };
    let Some(text) = read(cx)? else { return Ok(None) };
    let edit = PortalEdit {
        added_section: *added_section,
        added_default: *added_default,
        previous: previous.clone(),
    };
    let reverted = without_file_chooser(&text, &edit);
    let path = cx.env.portals_conf();
    // A file setup created and that is empty once our lines are out goes
    // too: an empty user file would still shadow the system's.
    if *created_file && reverted.trim().is_empty() {
        std::fs::remove_file(&path).map_err(|e| format!("couldn't remove {}: {e}", path.display()))?;
    } else if reverted != text {
        hyprforge_paths::write_atomic(&path, &reverted)
            .map_err(|e| format!("couldn't write {}: {e}", path.display()))?;
    }
    Ok(restart(cx))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_file_gains_the_section_the_default_and_our_line() {
        let (text, edit) = with_file_chooser("");
        assert_eq!(text, "[preferred]\ndefault=hyprland;gtk\norg.freedesktop.impl.portal.FileChooser=hyprforge\n");
        assert!(edit.added_section && edit.added_default && edit.previous.is_none());
        assert_eq!(without_file_chooser(&text, &edit), "");
    }

    /// The user's own `default=` is theirs, and other sections and
    /// comments are untouched — the whole file is copied through.
    #[test]
    fn the_users_other_lines_and_their_default_are_kept() {
        let original = "# mine\n[preferred]\ndefault=gnome;gtk\norg.freedesktop.impl.portal.Screenshot=hyprland\n\n[other]\nx=y\n";
        let (text, edit) = with_file_chooser(original);
        assert!(text.contains("default=gnome;gtk"), "{text}");
        assert!(!text.contains(DEFAULT_LINE), "{text}");
        assert!(text.contains("# mine") && text.contains("[other]\nx=y"), "{text}");
        assert!(text.contains("Screenshot=hyprland\norg.freedesktop.impl.portal.FileChooser=hyprforge\n\n[other]"), "{text}");
        assert!(!edit.added_default && !edit.added_section);
        assert_eq!(without_file_chooser(&text, &edit), original);
    }

    #[test]
    fn a_previous_file_chooser_is_replaced_and_restored() {
        let original = "[preferred]\ndefault=hyprland;gtk\norg.freedesktop.impl.portal.FileChooser=kde\n";
        let (text, edit) = with_file_chooser(original);
        assert!(has_file_chooser(&text));
        assert_eq!(edit.previous.as_deref(), Some("kde"));
        assert_eq!(without_file_chooser(&text, &edit), original);
    }

    /// Undo after the user picked another dialog themselves: theirs stays.
    #[test]
    fn undo_leaves_a_chooser_the_user_set_since() {
        let (_, edit) = with_file_chooser("");
        let theirs = "[preferred]\ndefault=hyprland;gtk\norg.freedesktop.impl.portal.FileChooser=gtk\n";
        assert_eq!(without_file_chooser(theirs, &edit), theirs);
    }
}
