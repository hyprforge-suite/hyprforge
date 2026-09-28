//! Handing a picture to another program: "Open With…" and "Show in Files".
//!
//! # Why not the desktop's own opener, as Files does
//!
//! `hyprforge-files`' `launch.rs` opens a file with `gio open`, which asks
//! `mimeapps.list` for the default — the right call there, and the
//! reasoning (why `gio` before `xdg-open`, why never a third opinion about
//! defaults) is written down in that file. Here it would be wrong in the
//! one case that matters: when this viewer *is* the default for the type,
//! `gio open` reopens the picture in the viewer it was already in. So
//! "Open With…" asks the same database for the same default, and takes
//! the next choice when the answer is this app — still the user's own
//! list, read the same way, never a third opinion.
//!
//! # Spawned and reaped, never waited on
//!
//! A program opened from here can run for hours. Waiting for it would
//! freeze the window; not waiting at all would leave a zombie for as long
//! as the viewer stays open. A thread that does nothing but reap it is
//! the middle.

use hyprforge_mime::App;
use std::path::Path;
use std::process::{Command, Stdio};

/// This viewer's own desktop entry — the id `mimeapps.list` knows it by.
pub const OWN_ID: &str = "hyprforge-media.desktop";

/// The program "Open With…" should use: the user's default for the type
/// unless that is this viewer, then the first other installed program
/// registered for it. `None` when nothing else can open it.
pub fn other_app<'a>(default: Option<&'a App>, candidates: &[&'a App]) -> Option<&'a App> {
    let usable = |app: &&App| app.id != OWN_ID && app.installed;
    default.filter(usable).or_else(|| candidates.iter().copied().find(usable))
}

/// Opens `file` with the program whose desktop entry is at `entry`.
pub fn open_with(entry: &Path, file: &Path) -> Result<(), String> {
    spawn(Command::new("gio").arg("launch").arg(entry).arg(file)).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            "Couldn't open it in another program because gio isn't installed (it comes with glib2)."
                .to_string()
        } else {
            format!("Couldn't open it in another program: {e}")
        }
    })
}

/// Opens the file manager on `file`'s folder.
///
/// The file manager opens the folder containing a file it is handed —
/// its own `resolve_start_dir` — so the picture's path is passed as-is.
/// Not installed is a sentence, never a failure of the viewer: every
/// component runs alone.
pub fn show_in_files(file: &Path) -> Result<(), String> {
    spawn(Command::new("hyprforge-files").arg(file)).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            "Hyprforge Files isn't installed, so there's nothing to show the folder in.".to_string()
        } else {
            format!("Couldn't open the file manager: {e}")
        }
    })
}

fn spawn(command: &mut Command) -> std::io::Result<()> {
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// The entry claims what this build can open, and nothing else: a
    /// type claimed and not decodable is a file association that opens a
    /// window with an error, and one decodable and not claimed is a
    /// format the viewer never gets asked about.
    ///
    /// Asked of the build rather than of the root manifest's feature
    /// list, because the two differ — iced's `image` feature brings
    /// `image`'s default codecs — and the build is what opens the file.
    #[test]
    fn the_desktop_entry_claims_exactly_what_this_build_decodes() {
        // The decoder's name for a type, and the shared MIME database's
        // canonical one, which is what the entry claims.
        const CANONICAL: [(&str, &str); 3] = [
            ("image/x-icon", "image/vnd.microsoft.icon"),
            ("image/x-targa", "image/x-tga"),
            ("image/x-qoi", "image/qoi"),
        ];
        let canonical = |kind: &'static str| CANONICAL.iter().find(|(d, _)| *d == kind).map_or(kind, |(_, c)| *c);
        let entry = include_str!("../packaging/hyprforge-media.desktop");
        let mut claimed: Vec<&str> = entry
            .lines()
            .find_map(|l| l.strip_prefix("MimeType="))
            .expect("declares its types")
            .split(';')
            .filter(|t| !t.is_empty())
            .collect();
        // Farbfeld has no MIME type at all — `image` answers
        // octet-stream — so there is nothing an entry could claim for it.
        // Pictures the image decoder opens, the 3D models the mesh loader
        // opens, and the videos the listing classifies and mpv plays —
        // the viewer claims those, and nothing else.
        let mut decodable: Vec<&str> = hyprforge_image::format::decodable_mime_types()
            .into_iter()
            .filter(|k| *k != "application/octet-stream")
            .map(canonical)
            .chain(hyprforge_mesh::Format::ALL.map(hyprforge_mesh::Format::mime_type))
            .chain(crate::folder::VIDEO_MIME_TYPES)
            .collect();
        claimed.sort_unstable();
        decodable.sort_unstable();
        assert_eq!(claimed, decodable);
    }

    fn app(id: &str, installed: bool) -> App {
        App {
            id: id.to_string(),
            name: id.trim_end_matches(".desktop").to_string(),
            icon: None,
            path: PathBuf::from(format!("/usr/share/applications/{id}")),
            installed,
        }
    }

    #[test]
    fn the_users_default_is_used_when_it_is_another_program() {
        let gimp = app("gimp.desktop", true);
        let other = app("other.desktop", true);
        assert_eq!(other_app(Some(&gimp), &[&other, &gimp]).map(|a| a.id.as_str()), Some("gimp.desktop"));
    }

    /// The case this module exists for: the viewer is the default, and
    /// "Open With…" reopening the picture in the viewer would be a button
    /// that does nothing.
    #[test]
    fn when_this_viewer_is_the_default_the_next_program_is_used() {
        let own = app(OWN_ID, true);
        let gimp = app("gimp.desktop", true);
        assert_eq!(other_app(Some(&own), &[&own, &gimp]).map(|a| a.id.as_str()), Some("gimp.desktop"));
    }

    #[test]
    fn a_program_whose_executable_is_gone_is_skipped() {
        let gone = app("gone.desktop", false);
        let gimp = app("gimp.desktop", true);
        assert_eq!(other_app(Some(&gone), &[&gone, &gimp]).map(|a| a.id.as_str()), Some("gimp.desktop"));
    }

    #[test]
    fn nothing_else_registered_is_none_rather_than_this_viewer() {
        let own = app(OWN_ID, true);
        assert_eq!(other_app(Some(&own), &[&own]), None);
    }
}
