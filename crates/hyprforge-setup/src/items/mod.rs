//! The items, in the order they are offered and applied.
//!
//! Order matters once: [`wiring`] comes first, because every generated
//! Lua file the later items write (keybinds, window rules, session) only
//! takes effect through the `require()` line it installs. Undo runs the
//! other way round.

use crate::record::Change;
use crate::state::State;
use crate::system::System;
use crate::Env;

mod binds;
mod blur;
mod defaults;
mod filemanager;
mod fingerprint;
mod gtk;
mod idle;
mod lock_restore;
mod portal;
mod services;
mod snapshots;
mod wiring;

pub use portal::{with_file_chooser, without_file_chooser, PortalEdit};

/// The suite's own user units, the ones the packages preset.
pub const SUITE_UNITS: [&str; 4] = [
    "hyprforge-displayd.service",
    "hyprforge-trayd.service",
    "hyprforge-clipd.service",
    services::NOTIFD_UNIT,
];

/// What an item's operations are given: the paths, and the system.
pub(crate) struct Cx<'a> {
    pub env: &'a Env,
    pub sys: &'a dyn System,
}

/// What a successful apply changed.
pub(crate) struct Applied {
    pub change: Change,
    /// Something worth saying about how it landed — "takes effect at next
    /// login", "Hyprland isn't running, so this loads next time".
    pub note: Option<String>,
}

/// What to tell the user about a generated file that was written but not
/// loaded — Hyprland was not there to ask.
pub(crate) fn loaded_note(loaded: crate::system::Loaded) -> Option<String> {
    match loaded {
        crate::system::Loaded::Live => None,
        crate::system::Loaded::NotReloaded(why) => Some(format!(
            "saved, but Hyprland wasn't reloaded ({why}) — it takes effect the next time \
             Hyprland starts"
        )),
    }
}

/// One thing setup can do. Plain data; the behaviour lives in one private
/// module per kind of item, reached through [`crate::check`],
/// [`crate::apply`] and [`crate::undo`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Item {
    /// Stable: the `--porcelain` id, the `setup.toml` key and the
    /// `--undo` argument. Never renamed.
    pub id: &'static str,
    /// What the Set up page and `--setup` show.
    pub label: &'static str,
    /// One line on why someone would want it.
    pub why: &'static str,
    /// Ticked unless the user says otherwise.
    pub default_on: bool,
    kind: Kind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Wiring,
    Service(&'static services::ServiceSpec),
    Notifd,
    Bind(&'static binds::BindSpec),
    IdleLock,
    LockRestore,
    NotifBlur,
    Defaults(&'static defaults::DefaultsSpec),
    PortalDialog,
    ShowInFolder,
    GtkPortal,
    PreviousVersions,
    FingerprintPolkit,
}

impl Item {
    const fn new(
        id: &'static str,
        label: &'static str,
        why: &'static str,
        default_on: bool,
        kind: Kind,
    ) -> Item {
        Item { id, label, why, default_on, kind }
    }

    /// The programs this item needs installed. An item missing one is
    /// [`State::Unavailable`], never [`State::Todo`].
    pub fn requires(&self) -> &'static [&'static str] {
        match self.kind {
            Kind::Wiring => &[],
            Kind::Service(spec) => spec.requires,
            Kind::Notifd => &["notifd"],
            Kind::Bind(spec) => spec.requires,
            Kind::IdleLock => &["hypridle", "hyprforge-lock"],
            Kind::LockRestore => &["hyprforge-lock"],
            Kind::NotifBlur => &["notifd"],
            Kind::Defaults(spec) => spec.requires,
            Kind::PortalDialog | Kind::GtkPortal => &["hyprforge-files-portal"],
            Kind::ShowInFolder => &["hyprforge-files"],
            Kind::PreviousVersions => &["hyprforge-files", "snapper", "pkexec"],
            Kind::FingerprintPolkit => &["pkexec", "fprintd-list"],
        }
    }

    /// The unit a service item enables, for "Turn off for me".
    pub fn unit(&self) -> Option<&'static str> {
        match self.kind {
            Kind::Service(spec) => Some(spec.unit),
            Kind::Notifd => Some(services::NOTIFD_UNIT),
            _ => None,
        }
    }

    pub(crate) fn check(&self, cx: &Cx<'_>) -> State {
        if let Some(missing) = self.requires().iter().find(|b| cx.sys.find_binary(b).is_none()) {
            return State::Unavailable { why: format!("{missing} isn't installed") };
        }
        match self.kind {
            Kind::Wiring => wiring::check(cx),
            Kind::Service(spec) => services::check(cx, spec),
            Kind::Notifd => services::check_notifd(cx),
            Kind::Bind(spec) => binds::check(cx, spec),
            Kind::IdleLock => idle::check(cx),
            Kind::LockRestore => lock_restore::check(cx),
            Kind::NotifBlur => blur::check(cx),
            Kind::Defaults(spec) => defaults::check(cx, spec),
            Kind::PortalDialog => portal::check(cx),
            Kind::ShowInFolder => filemanager::check(cx),
            Kind::GtkPortal => gtk::check(cx),
            Kind::PreviousVersions => snapshots::check(cx),
            Kind::FingerprintPolkit => fingerprint::check(cx),
        }
    }

    /// Only ever called on an item that just checked as [`State::Todo`].
    pub(crate) fn apply(&self, cx: &Cx<'_>) -> Result<Applied, String> {
        match self.kind {
            Kind::Wiring => wiring::apply(cx),
            Kind::Service(spec) => services::apply(cx, spec),
            Kind::Notifd => services::apply_notifd(cx),
            Kind::Bind(spec) => binds::apply(cx, spec),
            Kind::IdleLock => idle::apply(cx),
            Kind::LockRestore => lock_restore::apply(cx),
            Kind::NotifBlur => blur::apply(cx),
            Kind::Defaults(spec) => defaults::apply(cx, spec),
            Kind::PortalDialog => portal::apply(cx),
            Kind::ShowInFolder => filemanager::apply(cx),
            Kind::GtkPortal => gtk::apply(cx),
            Kind::PreviousVersions => snapshots::apply(cx),
            Kind::FingerprintPolkit => fingerprint::apply(cx),
        }
    }

    /// Puts back what `change` records. `Ok` carries a note when the
    /// result is not quite "as it was" and the user should know why.
    pub(crate) fn undo(&self, cx: &Cx<'_>, change: &Change) -> Result<Option<String>, String> {
        match (self.kind, change) {
            (Kind::Wiring, Change::Wiring { inserted, .. }) => wiring::undo(cx, inserted),
            (Kind::Service(_), Change::Service { unit }) => services::undo(cx, unit),
            (Kind::Notifd, Change::Notifd { enabled_notifd, replaced }) => {
                services::undo_notifd(cx, *enabled_notifd, replaced)
            }
            (Kind::Bind(spec), Change::Bind { name }) => binds::undo(cx, spec, name),
            (Kind::IdleLock, Change::IdleLock { previous }) => idle::undo(cx, previous),
            (Kind::LockRestore, Change::LockRestore { previous }) => {
                lock_restore::undo(cx, previous.as_ref())
            }
            (Kind::NotifBlur, Change::LayerRules { names, previous }) => {
                blur::undo(cx, names, previous)
            }
            (Kind::Defaults(_), Change::Defaults { app, previous }) => {
                defaults::undo(cx, app, previous)
            }
            (Kind::PortalDialog, change @ Change::Portal { .. }) => portal::undo(cx, change),
            (Kind::ShowInFolder, Change::FileManager1 { previous }) => {
                filemanager::undo(cx, previous.as_deref())
            }
            (Kind::GtkPortal, Change::GtkPortal {}) => gtk::undo(cx),
            (Kind::PreviousVersions, Change::Snapper { allow_users, sync_acl }) => {
                snapshots::undo(cx, allow_users, sync_acl)
            }
            (Kind::FingerprintPolkit, Change::FingerprintPam { previous }) => {
                fingerprint::undo(cx, previous.as_deref())
            }
            // A record edited by hand into the wrong shape for its item.
            // Refused rather than guessed at.
            _ => Err(format!("setup.toml records a change {} didn't make", self.id)),
        }
    }
}

/// Every item, in order. See the module doc for why the order matters.
pub static ITEMS: [Item; 21] = [
    Item::new(
        "wiring",
        "Connect Hyprforge to your Hyprland config",
        "One require() line per Hyprforge module in hyprland.lua, so keybinds, rules and settings saved here take effect.",
        true,
        Kind::Wiring,
    ),
    Item::new(
        "service-displayd",
        "Display daemon",
        "Restores your monitor layout when a display is plugged in.",
        true,
        Kind::Service(&services::DISPLAYD),
    ),
    Item::new(
        "service-trayd",
        "Tray icons",
        "Network, Bluetooth, battery and the rest in your bar's tray.",
        true,
        Kind::Service(&services::TRAYD),
    ),
    Item::new(
        "service-clipd",
        "Clipboard history",
        "Remembers what you copy, so Super+V can paste it again.",
        true,
        Kind::Service(&services::CLIPD),
    ),
    Item::new(
        "service-notifd",
        "Notifications",
        "Hyprforge's notification daemon, with a history centre and Do Not Disturb.",
        true,
        Kind::Notifd,
    ),
    Item::new(
        "bind-clipboard",
        "Super+V opens clipboard history",
        "The clipboard history popup, on the chord most desktops use for it.",
        true,
        Kind::Bind(&binds::CLIPBOARD),
    ),
    Item::new(
        "bind-emoji",
        "Super+. opens the emoji picker",
        "Search and paste an emoji anywhere.",
        true,
        Kind::Bind(&binds::EMOJI),
    ),
    Item::new(
        "bind-notifications",
        "Super+N opens the notification centre",
        "What you missed, and the history of what came in.",
        true,
        Kind::Bind(&binds::NOTIFICATIONS),
    ),
    Item::new(
        "bind-dnd",
        "Super+Shift+N toggles Do Not Disturb",
        "Silence notifications without opening anything.",
        true,
        Kind::Bind(&binds::DND),
    ),
    Item::new(
        "bind-files",
        "Super+E opens Files",
        "The file manager, on the chord most desktops use for it.",
        true,
        Kind::Bind(&binds::FILES),
    ),
    Item::new(
        "bind-lock",
        "Super+Escape locks the screen",
        "Lock the session from the keyboard.",
        true,
        Kind::Bind(&binds::LOCK),
    ),
    Item::new(
        "idle-lock",
        "Lock with Hyprforge's lock screen",
        "hypridle runs hyprforge-lock when the session is asked to lock.",
        true,
        Kind::IdleLock,
    ),
    Item::new(
        "lock-restore",
        "Restart the lock screen if it crashes",
        "Lets a new hyprforge-lock take over a lock whose screen died, instead of leaving the session stuck behind Hyprland's error screen.",
        true,
        Kind::LockRestore,
    ),
    Item::new(
        "notif-blur",
        "Blur behind notifications",
        "Hyprland blurs what is behind notification popups and the centre panel.",
        true,
        Kind::NotifBlur,
    ),
    Item::new(
        "default-folders",
        "Open folders with Files",
        "\"Open containing folder\" and every folder link go to Files.",
        true,
        Kind::Defaults(&defaults::FOLDERS),
    ),
    Item::new(
        "default-images",
        "Open pictures with Media",
        "PNG, JPEG, GIF, WebP, TIFF and BMP open in the Media viewer.",
        true,
        Kind::Defaults(&defaults::IMAGES),
    ),
    Item::new(
        "portal-dialog",
        "Use Files' open and save dialog",
        "Apps that ask the desktop portal for a file dialog get Files' one.",
        true,
        Kind::PortalDialog,
    ),
    Item::new(
        "show-in-folder",
        "Show in folder opens Files",
        "A browser's \"Show in folder\" and an editor's \"Reveal in file manager\" open Files.",
        true,
        Kind::ShowInFolder,
    ),
    Item::new(
        "gtk-portal",
        "Every GTK app uses this dialog",
        "Sets GTK_USE_PORTAL=1 so GTK apps that would draw their own dialog ask the portal. Takes effect at next login.",
        false,
        Kind::GtkPortal,
    ),
    Item::new(
        "previous-versions",
        "Previous versions of your files",
        "Lets you read snapper's snapshots of /home, so Files can show and restore older copies. Asks for your password once.",
        false,
        Kind::PreviousVersions,
    ),
    Item::new(
        "fingerprint-polkit",
        "Fingerprint for administrator prompts",
        "Adds your fingerprint reader to polkit's sign-in, so a finger can answer an administrator prompt. If you don't touch it, the password comes up after ten seconds. Asks for your password once.",
        false,
        Kind::FingerprintPolkit,
    ),
];

/// The item with this id.
pub fn item(id: &str) -> Option<&'static Item> {
    ITEMS.iter().find(|i| i.id == id)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The ids are the installer's contract — this is the list it was
    /// written against, in this order.
    #[test]
    fn the_item_ids_are_the_agreed_ones_in_order() {
        let ids: Vec<&str> = ITEMS.iter().map(|i| i.id).collect();
        assert_eq!(
            ids,
            [
                "wiring",
                "service-displayd",
                "service-trayd",
                "service-clipd",
                "service-notifd",
                "bind-clipboard",
                "bind-emoji",
                "bind-notifications",
                "bind-dnd",
                "bind-files",
                "bind-lock",
                "idle-lock",
                "lock-restore",
                "notif-blur",
                "default-folders",
                "default-images",
                "portal-dialog",
                "show-in-folder",
                "gtk-portal",
                "previous-versions",
                "fingerprint-polkit",
            ]
        );
    }

    /// The opt-ins: a variable every GTK app inherits, and the two
    /// changes that ask for a password, are not things to do without
    /// being asked.
    #[test]
    fn only_the_gtk_portal_variable_and_the_root_changes_are_off_by_default() {
        let off: Vec<&str> = ITEMS.iter().filter(|i| !i.default_on).map(|i| i.id).collect();
        assert_eq!(off, ["gtk-portal", "previous-versions", "fingerprint-polkit"]);
    }
}
