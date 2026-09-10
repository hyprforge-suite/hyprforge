//! What's actually open, so a rule can be built from a real window instead of
//! a remembered class name.
//!
//! A mistyped class produces a rule that simply never matches, and nothing
//! anywhere reports it — the rule saves, the Lua loads, and the window keeps
//! behaving as before. Reading the open windows removes the guess. It's the
//! same move the Displays module makes by showing real monitors rather than
//! asking for connector names.
//!
//! This lives in the library rather than the GUI so it can be tested against a
//! captured payload, with no compositor in the loop.

use serde::Deserialize;
use std::process::Command;

#[derive(Debug, thiserror::Error)]
pub enum ClientsError {
    #[error("could not run hyprctl: {0}")]
    Spawn(#[source] std::io::Error),
    #[error("hyprctl clients failed: {stderr}")]
    Failed { stderr: String },
    #[error("could not parse hyprctl clients output: {0}")]
    Parse(#[source] serde_json::Error),
}

/// Which workspace a window is on. `name` is what a rule would use — it's the
/// ID for a numbered workspace and the real name for a named one.
#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
pub struct ClientWorkspace {
    #[serde(default)]
    pub id: i64,
    #[serde(default)]
    pub name: String,
}

/// One open window, as `hyprctl clients -j` describes it.
///
/// Every field is `#[serde(default)]` and unknown keys are ignored, so a
/// Hyprland release that adds or drops a key can't stop the picker working.
/// Only the fields a rule can actually match on are modelled; the geometry,
/// focus history and swallowing state that also come back are deliberately
/// left out.
#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Client {
    pub class: String,
    pub initial_class: String,
    pub title: String,
    pub initial_title: String,
    pub floating: bool,
    pub xwayland: bool,
    /// 0 when not fullscreen; the mode otherwise.
    pub fullscreen: i64,
    /// `none` when the window declares nothing, which is the common case.
    pub content_type: String,
    pub tags: Vec<String>,
    pub workspace: ClientWorkspace,
    /// False for a window that exists but isn't on screen. Those are filtered
    /// out of the picker — you can't point at what you can't see.
    pub mapped: bool,
}

impl Client {
    /// What the picker shows. Two windows of the same app are common — this
    /// machine routinely has several terminals — so the class alone can't
    /// identify a row; the title is what tells them apart.
    pub fn label(&self) -> String {
        if self.title.trim().is_empty() {
            self.class.clone()
        } else {
            format!("{} — {}", self.class, self.title)
        }
    }

    /// The content type as a rule would match on it, or `None` when the
    /// window declares nothing. Hyprland reports the absence as the string
    /// `none`, which would be a meaningless matcher if passed through.
    pub fn content_type_for_match(&self) -> Option<&str> {
        match self.content_type.trim() {
            "" | "none" => None,
            other => Some(other),
        }
    }
}

/// The open windows, most recently focused first is *not* guaranteed — the
/// order is whatever Hyprland returns.
///
/// Windows that aren't mapped, or that report no class at all, are dropped:
/// neither can be pointed at, and a rule matching an empty class would match
/// nothing.
pub fn list_clients() -> Result<Vec<Client>, ClientsError> {
    let out = hyprforge_core::command::output(
        Command::new("hyprctl").arg("clients").arg("-j"),
        hyprforge_core::command::TIMEOUT,
    )
        .map_err(ClientsError::Spawn)?;

    if !out.status.success() {
        return Err(ClientsError::Failed {
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        });
    }
    parse_clients(&String::from_utf8_lossy(&out.stdout))
}

/// Split out from [`list_clients`] so the parsing can be tested without a
/// compositor.
pub fn parse_clients(json: &str) -> Result<Vec<Client>, ClientsError> {
    let all: Vec<Client> = serde_json::from_str(json).map_err(ClientsError::Parse)?;
    Ok(all
        .into_iter()
        .filter(|c| c.mapped && !c.class.trim().is_empty())
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A real `hyprctl clients -j` capture, not a hand-written approximation
    /// — it's the only way this test can catch a field Hyprland renames.
    const CAPTURE: &str = include_str!("../tests/fixtures/hyprctl-clients.json");

    #[test]
    fn parses_a_real_capture() {
        let clients = parse_clients(CAPTURE).expect("the captured payload should parse");
        assert_eq!(clients.len(), 4);
        let ghostty = &clients[0];
        assert_eq!(ghostty.class, "com.mitchellh.ghostty");
        assert_eq!(ghostty.initial_class, "com.mitchellh.ghostty");
        assert_eq!(ghostty.initial_title, "Ghostty");
        assert_eq!(ghostty.workspace.name, "3");
        assert!(!ghostty.xwayland);
    }

    /// An app that reports no initial title is ordinary, not a parse failure.
    #[test]
    fn an_empty_initial_title_is_not_an_error() {
        let clients = parse_clients(CAPTURE).unwrap();
        let bambu = clients.iter().find(|c| c.class == "BambuStudio").unwrap();
        assert_eq!(bambu.initial_title, "");
        assert!(bambu.xwayland, "this one is an XWayland window");
    }

    /// The case the picker exists to disambiguate: two windows, same class,
    /// different titles. Picking by class alone can't tell them apart, which
    /// is why the label carries the title.
    #[test]
    fn windows_sharing_a_class_get_distinguishable_labels() {
        let clients = parse_clients(CAPTURE).unwrap();
        let ghosttys: Vec<_> = clients
            .iter()
            .filter(|c| c.class == "com.mitchellh.ghostty")
            .collect();
        assert_eq!(ghosttys.len(), 2);
        assert_ne!(ghosttys[0].label(), ghosttys[1].label());
    }

    #[test]
    fn a_titleless_window_falls_back_to_its_class() {
        let c = Client { class: "foo".into(), title: "   ".into(), ..Default::default() };
        assert_eq!(c.label(), "foo");
    }

    /// `none` is how Hyprland spells "declares nothing" — passing it through
    /// as a matcher would select nothing at all.
    #[test]
    fn a_content_type_of_none_is_not_offered_as_a_matcher() {
        let clients = parse_clients(CAPTURE).unwrap();
        assert!(clients.iter().all(|c| c.content_type_for_match().is_none()));

        let game = Client { content_type: "game".into(), ..Default::default() };
        assert_eq!(game.content_type_for_match(), Some("game"));
    }

    /// No window on this machine carried a tag when the fixture was taken, so
    /// that case is covered here rather than by doctoring the capture.
    #[test]
    fn tags_are_read_when_present() {
        let json = r#"[{"class":"foot","title":"t","mapped":true,"tags":["term","code"]}]"#;
        let clients = parse_clients(json).unwrap();
        assert_eq!(clients[0].tags, vec!["term", "code"]);
    }

    /// Hyprland adds keys between releases; an unmodelled one must not take
    /// the picker down.
    #[test]
    fn unknown_keys_are_ignored() {
        let json = r#"[{"class":"foot","mapped":true,"somethingNewIn058":42}]"#;
        assert_eq!(parse_clients(json).unwrap().len(), 1);
    }

    /// A missing key is equally survivable — every field has a default.
    #[test]
    fn a_sparse_entry_still_parses() {
        let clients = parse_clients(r#"[{"class":"foot","mapped":true}]"#).unwrap();
        assert_eq!(clients[0].title, "");
        assert_eq!(clients[0].workspace, ClientWorkspace::default());
    }

    /// Neither an unmapped window nor a classless one can be pointed at.
    #[test]
    fn unmapped_and_classless_windows_are_dropped() {
        let json = r#"[
            {"class":"visible","mapped":true},
            {"class":"hidden","mapped":false},
            {"class":"","mapped":true}
        ]"#;
        let clients = parse_clients(json).unwrap();
        assert_eq!(clients.len(), 1);
        assert_eq!(clients[0].class, "visible");
    }

    #[test]
    fn no_windows_open_is_an_empty_list_not_an_error() {
        assert!(parse_clients("[]").unwrap().is_empty());
    }

    #[test]
    fn malformed_json_is_an_error_rather_than_a_panic() {
        assert!(parse_clients("not json").is_err());
    }
}
