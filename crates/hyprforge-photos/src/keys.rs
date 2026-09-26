//! What this app can be asked to do, and the keys that ask for it.
//!
//! The grammar is `hyprforge-keys`; this is the table. Written as strings
//! and parsed by the same function that reads `photos-config.toml`, so
//! the shipped defaults *are* a config and there is no second table for
//! the file to drift from.
//!
//! # Bare letters, unlike the file manager
//!
//! This app declares [`BARE_KEYS`] as [`BareKeys::Bindable`], and it is
//! the reason that option exists. A viewer has nowhere to type: there is
//! no search box, no rename field, nothing a character could go into. So
//! `n`, `p`, `f`, `i`, `+` and `-` are simply keys — which is what every
//! image viewer anyone has used has taught people to expect.
//!
//! The file manager answers the other way, because a bare letter there
//! goes into type-to-search. Neither answer is right for both apps, and
//! shipping the file manager's answer here would refuse this table at
//! load with a message about a search box that does not exist.

use hyprforge_keys::{BareKeys, Bindable};

/// This app has nowhere to type, so a printable key is bindable.
pub const BARE_KEYS: BareKeys = BareKeys::Bindable;

/// Everything the viewer can be asked to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Action {
    /// The next picture in the folder, in the order the file manager
    /// shows it.
    Next,
    Previous,
    /// One row down in the grid or the library; the next picture in
    /// Photo mode, where there are no rows.
    Below,
    /// One row up; the previous picture in Photo mode.
    Above,
    First,
    Last,
    ZoomIn,
    ZoomOut,
    /// Fit the whole picture in the window — the state it opens in.
    ZoomFit,
    /// One image pixel to one screen pixel.
    ZoomActual,
    /// Turn the picture on screen. Does not touch the file; writing a
    /// rotation back is an edit of somebody's original and is a separate
    /// decision.
    RotateLeft,
    RotateRight,
    ToggleFullscreen,
    ToggleInfo,
    ToggleFilmstrip,
    /// The Places sidebar — `Ctrl+B`, as in the file manager.
    ToggleSidebar,
    /// The three modes of the window, mockup `2a`'s segmented switch.
    ShowPhoto,
    ShowGrid,
    ShowLibrary,
    /// Whatever Enter means where you are: open the tile in the grid,
    /// the folder in the library, and hand a clip to its player in Photo
    /// mode. The app decides, the way it decides what Escape means.
    Activate,
    /// The library one folder up — mockup `1h`'s Backspace.
    Up,
    /// The folder this window showed before, and back again.
    Back,
    Forward,
    /// Fullscreen, one picture after another — mockup `1e`.
    Slideshow,
    /// Hand this file to whatever the desktop opens it with — how a clip
    /// gets played, since nothing here decodes video.
    OpenExternally,
    /// Open the file manager on this picture's folder. A logged warning,
    /// not a failure, when the file manager is not installed.
    ShowInFiles,
    /// Put the picture on the clipboard, for pasting into another app.
    Copy,
    /// Make this picture the wallpaper, through the same settings the
    /// Wallpaper page in Settings edits — so the two never disagree.
    SetWallpaper,
    /// Send to the trash, reversibly.
    Trash,
    /// Leave fullscreen, or close.
    Close,
}

impl Action {
    pub fn all() -> Vec<Action> {
        vec![
            Action::Next,
            Action::Previous,
            Action::Below,
            Action::Above,
            Action::First,
            Action::Last,
            Action::ZoomIn,
            Action::ZoomOut,
            Action::ZoomFit,
            Action::ZoomActual,
            Action::RotateLeft,
            Action::RotateRight,
            Action::ToggleFullscreen,
            Action::ToggleInfo,
            Action::ToggleFilmstrip,
            Action::ToggleSidebar,
            Action::ShowPhoto,
            Action::ShowGrid,
            Action::ShowLibrary,
            Action::Activate,
            Action::Up,
            Action::Back,
            Action::Forward,
            Action::Slideshow,
            Action::OpenExternally,
            Action::ShowInFiles,
            Action::Copy,
            Action::SetWallpaper,
            Action::Trash,
            Action::Close,
        ]
    }

    /// The id `photos-config.toml` uses. Part of the file format:
    /// renaming one breaks somebody's config.
    pub fn id(self) -> &'static str {
        match self {
            Action::Next => "next",
            Action::Previous => "previous",
            Action::Below => "below",
            Action::Above => "above",
            Action::First => "first",
            Action::Last => "last",
            Action::ZoomIn => "zoom-in",
            Action::ZoomOut => "zoom-out",
            Action::ZoomFit => "zoom-fit",
            Action::ZoomActual => "zoom-actual",
            Action::RotateLeft => "rotate-left",
            Action::RotateRight => "rotate-right",
            Action::ToggleFullscreen => "fullscreen",
            Action::ToggleInfo => "info",
            Action::ToggleFilmstrip => "filmstrip",
            Action::ToggleSidebar => "sidebar",
            Action::ShowPhoto => "photo-mode",
            Action::ShowGrid => "grid-mode",
            Action::ShowLibrary => "library-mode",
            Action::Activate => "activate",
            Action::Up => "up",
            Action::Back => "back",
            Action::Forward => "forward",
            Action::Slideshow => "slideshow",
            Action::OpenExternally => "open-externally",
            Action::ShowInFiles => "show-in-files",
            Action::Copy => "copy",
            Action::SetWallpaper => "set-wallpaper",
            Action::Trash => "trash",
            Action::Close => "close",
        }
    }

    /// What a menu or a settings page calls this.
    pub fn label(self) -> &'static str {
        match self {
            Action::Next => "Next",
            Action::Previous => "Previous",
            Action::Below => "Down",
            Action::Above => "Up",
            Action::First => "First",
            Action::Last => "Last",
            Action::ZoomIn => "Zoom In",
            Action::ZoomOut => "Zoom Out",
            Action::ZoomFit => "Fit to Window",
            Action::ZoomActual => "Actual Size",
            Action::RotateLeft => "Rotate Left",
            Action::RotateRight => "Rotate Right",
            Action::ToggleFullscreen => "Fullscreen",
            Action::ToggleInfo => "Information",
            Action::ToggleFilmstrip => "Filmstrip",
            Action::ToggleSidebar => "Sidebar",
            Action::ShowPhoto => "Photo",
            Action::ShowGrid => "Grid",
            Action::ShowLibrary => "Library",
            Action::Activate => "Open",
            Action::Up => "Enclosing Folder",
            Action::Back => "Back",
            Action::Forward => "Forward",
            Action::Slideshow => "Slideshow",
            Action::OpenExternally => "Open With…",
            Action::ShowInFiles => "Show in Files",
            Action::Copy => "Copy",
            Action::SetWallpaper => "Set as Wallpaper",
            Action::Trash => "Move to Trash",
            Action::Close => "Close",
        }
    }

    /// The shipped bindings.
    ///
    /// Chosen to match what image viewers have taught people rather than
    /// to be internally tidy: Space and Right both go forward because
    /// both are what people press, `+`/`-` zoom, `0` fits and `1` is
    /// actual size.
    pub fn default_keys(self) -> &'static [&'static str] {
        match self {
            // `Space` alongside the arrows: it is how people page
            // through a folder of photographs, and this app has no text
            // field for it to type into.
            Action::Next => &["Right", "Space", "N", "PageDown"],
            Action::Previous => &["Left", "P", "PageUp"],
            // Down and Up were Next and Previous before the grid had
            // rows; in Photo mode they still are — see `Action::Below`.
            Action::Below => &["Down"],
            Action::Above => &["Up"],
            Action::First => &["Home"],
            Action::Last => &["End"],
            Action::ZoomIn => &["Plus", "Ctrl+Plus"],
            Action::ZoomOut => &["Minus", "Ctrl+Minus"],
            Action::ZoomFit => &["0", "F"],
            Action::ZoomActual => &["1"],
            Action::RotateLeft => &["Shift+R"],
            Action::RotateRight => &["R"],
            Action::ToggleFullscreen => &["F11", "Ctrl+F"],
            Action::ToggleInfo => &["I"],
            Action::ToggleFilmstrip => &["F9"],
            Action::ToggleSidebar => &["Ctrl+B"],
            // Nothing by default: Escape leaves the grid and the library
            // for the photo, and the switch is one click. Bindable.
            Action::ShowPhoto => &[],
            Action::ShowGrid => &["G"],
            Action::ShowLibrary => &["L"],
            // Enter moved here from `OpenExternally`, which kept it on
            // a still for years; in Photo mode `Activate` still hands a
            // clip to its player, and Shift+Enter always opens outside.
            Action::Activate => &["Enter"],
            Action::Up => &["Backspace"],
            Action::Back => &["Alt+Left"],
            Action::Forward => &["Alt+Right"],
            Action::Slideshow => &["F5"],
            Action::OpenExternally => &["Shift+Enter"],
            Action::ShowInFiles => &["Ctrl+O"],
            Action::Copy => &["Ctrl+C"],
            Action::SetWallpaper => &["W"],
            Action::Trash => &["Delete"],
            // Escape leaves fullscreen first and closes otherwise — the
            // app decides which, not the keymap.
            Action::Close => &["Escape", "Ctrl+W", "Q"],
        }
    }

    /// The action a `photos-config.toml` id names, or `None` for one this
    /// version does not know — which the caller reports, never ignores.
    pub fn from_id(id: &str) -> Option<Action> {
        Action::all().into_iter().find(|a| a.id() == id)
    }
}

/// Four methods that delegate to the inherent ones of the same name.
/// Inherent methods win name resolution, so `Action::all()` still reaches
/// the function above.
impl Bindable for Action {
    fn all() -> Vec<Action> {
        Action::all()
    }

    fn id(self) -> &'static str {
        Action::id(self)
    }

    fn default_keys(self) -> &'static [&'static str] {
        Action::default_keys(self)
    }

    fn from_id(id: &str) -> Option<Action> {
        Action::from_id(id)
    }
}

/// The shipped keymap.
pub type Keymap = hyprforge_keys::Keymap<Action>;

/// What a key press turned out to mean here.
pub type Resolved = hyprforge_keys::Resolved<Action>;

pub fn defaults() -> Keymap {
    Keymap::defaults(BARE_KEYS)
}

/// The key to name for `action` in a hint — "G grid", "I info" — or
/// `None` when nothing is bound to it.
///
/// Read from the live keymap, never written into the hint: a status bar
/// that says "G grid" after the user moved the grid to `Ctrl+G` is
/// teaching them a key that does nothing. The first *default* that is
/// still bound wins, so the hint names the key people expect (`Right`,
/// not `N`) for as long as it works.
pub fn hint(keymap: &Keymap, action: Action) -> Option<String> {
    let bound = keymap.combos_for(action);
    let preferred = action
        .default_keys()
        .iter()
        .filter_map(|text| hyprforge_keys::Combo::parse(text).ok())
        .find(|combo| bound.contains(combo));
    preferred.or_else(|| bound.first().copied()).map(|combo| key_label(&combo))
}

/// A binding as a hint prints it: arrows as arrows, everything else as
/// the config file spells it.
pub fn key_label(combo: &hyprforge_keys::Combo) -> String {
    use hyprforge_keys::Key;
    let plain = combo.mods == hyprforge_keys::Modifiers::default();
    match combo.key {
        Key::Left if plain => "←".to_string(),
        Key::Right if plain => "→".to_string(),
        Key::Up if plain => "↑".to_string(),
        Key::Down if plain => "↓".to_string(),
        Key::Escape if plain => "Esc".to_string(),
        Key::Delete if plain => "Del".to_string(),
        _ => combo.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hyprforge_keys::{Combo, KeyPress};

    fn press(text: &str) -> KeyPress {
        let combo = Combo::parse(text).unwrap();
        KeyPress { key: combo.key, mods: combo.mods, text: None }
    }

    #[test]
    fn every_default_binding_parses() {
        for action in Action::all() {
            for text in action.default_keys() {
                assert!(Combo::parse(text).is_ok(), "{action:?}: {text}");
            }
        }
    }

    /// Two actions on one key would make one of them unreachable, and
    /// which one would depend on iteration order.
    #[test]
    fn no_two_actions_share_a_default_key() {
        let mut seen = std::collections::HashMap::new();
        for action in Action::all() {
            for text in action.default_keys() {
                let combo = Combo::parse(text).unwrap();
                if let Some(other) = seen.insert(combo, action) {
                    panic!("{text} is bound to both {other:?} and {action:?}");
                }
            }
        }
    }

    /// The reason `BareKeys` exists, from this side: the keys people
    /// actually press in a viewer are bare letters, and here they bind.
    #[test]
    fn a_viewer_with_nowhere_to_type_can_bind_a_bare_letter() {
        let keys = defaults();
        assert_eq!(keys.resolve(&press("N")), Some(Resolved::Action(Action::Next)));
        assert_eq!(keys.resolve(&press("P")), Some(Resolved::Action(Action::Previous)));
        assert_eq!(keys.resolve(&press("Space")), Some(Resolved::Action(Action::Next)));
        assert_eq!(keys.resolve(&press("I")), Some(Resolved::Action(Action::ToggleInfo)));
    }

    /// And nothing here ever resolves to text, because there is nowhere
    /// for text to go.
    #[test]
    fn a_press_never_resolves_to_text_in_this_app() {
        let keys = defaults();
        let z = KeyPress {
            key: hyprforge_keys::Key::Char('z'),
            mods: hyprforge_keys::Modifiers::default(),
            text: Some('z'),
        };
        assert_eq!(keys.resolve(&z), None);
    }

    /// The hint follows the keymap, not the defaults: rebinding the grid
    /// changes what the status bar teaches.
    #[test]
    fn a_hint_names_the_key_that_is_actually_bound() {
        let keys = defaults();
        assert_eq!(hint(&keys, Action::ShowGrid).as_deref(), Some("G"));
        assert_eq!(hint(&keys, Action::Next).as_deref(), Some("→"));
        assert_eq!(hint(&keys, Action::ShowPhoto), None);

        let overrides = std::collections::BTreeMap::from([("grid-mode".to_string(), vec!["Ctrl+G".to_string()])]);
        let (rebound, problems) = hyprforge_keys::merge::keymap_with::<Action>(&overrides, BARE_KEYS);
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(hint(&rebound, Action::ShowGrid).as_deref(), Some("Ctrl+G"));
    }

    #[test]
    fn every_action_has_an_id_and_a_label() {
        for action in Action::all() {
            assert!(!action.id().is_empty(), "{action:?}");
            assert!(!action.label().is_empty(), "{action:?}");
            assert_eq!(Action::from_id(action.id()), Some(action));
        }
    }

    /// Ids are the file format, so two sharing one would make a config
    /// entry ambiguous.
    #[test]
    fn no_two_actions_share_an_id() {
        let mut ids: Vec<&str> = Action::all().iter().map(|a| a.id()).collect();
        ids.sort_unstable();
        let before = ids.len();
        ids.dedup();
        assert_eq!(ids.len(), before);
    }
}
