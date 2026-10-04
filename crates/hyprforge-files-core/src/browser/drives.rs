//! The browser's half of drives and network shares: the sidebar's
//! **Devices** and **Remote** sections, what a click on one of their
//! rows means, and the drive actions.
//!
//! A child of `browser` for the reason `searching` is: it reaches the
//! browser's own state without that state growing accessors for one
//! feature, and the feature reads in one place. What the sections know
//! is [`crate::devices`]'s plain data; mounting is the host's.
//!
//! # A drive is a place once it is mounted
//!
//! A mounted drive's row behaves as every other place's: it goes there,
//! files dropped on it are copied there, and its menu opens on the
//! folder. An unmounted one has no folder yet, so a click asks the host
//! to mount it and then goes there ([`Ask::Mount`] with `open`); it has
//! a menu (Mount, Eject) but is no drop target, because a drop needs
//! somewhere to land and the drive has nowhere until it is mounted.

use super::{Browser, Message, Outcome, SidebarRow, SidebarSection, ViewModel};
use crate::action::Action;
use crate::devices::{Ask, DeviceMessage, Devices, Listing, CONNECT_ICON};
use crate::icon;
use crate::sidebar::Tint;
use hyprforge_ui::theme::FontScale;
use hyprforge_volumes::{Operation, VolumeId};
use iced::Element;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// What a row in those sections is, beyond its label and path.
#[derive(Debug, Clone, PartialEq)]
pub(super) enum DeviceRow {
    Volume {
        id: VolumeId,
        mounted: bool,
        /// Shows the eject mark — mounted, able to let go, nothing
        /// already happening to it, and a host that ejects things.
        eject: bool,
    },
    Share,
    /// "Connect to Server…".
    Connect,
    /// A sentence where rows would be — UDisks2 not running. Not a
    /// button: there is nowhere to go.
    Note,
}

/// What a sidebar row is besides a button.
pub(super) enum RowPlace {
    /// A folder: a drop target, with a folder's menu.
    Folder(PathBuf),
    /// A drive not yet mounted: a menu, and nowhere to drop.
    MenuOnly(PathBuf),
    /// Neither — a saved search, Connect to Server, a note.
    Plain,
}

pub(super) fn place(row: &SidebarRow) -> RowPlace {
    if row.search.is_some() {
        return RowPlace::Plain;
    }
    match &row.device {
        None | Some(DeviceRow::Share) | Some(DeviceRow::Volume { mounted: true, .. }) => RowPlace::Folder(row.path.clone()),
        Some(DeviceRow::Volume { mounted: false, .. }) => RowPlace::MenuOnly(row.path.clone()),
        Some(DeviceRow::Connect | DeviceRow::Note) => RowPlace::Plain,
    }
}

/// What a click on a drive's or share's row does, and whether it is the
/// one on screen. `None` for a row that is not a drive's.
pub(super) fn press(row: &SidebarRow, vm: &ViewModel<'_>) -> Option<(Option<Message>, bool)> {
    let here = row.path == vm.current_dir && vm.search.current.is_none() && vm.collection.is_none();
    Some(match row.device.as_ref()? {
        DeviceRow::Volume { id, mounted, .. } => (Some(Message::Device(DeviceMessage::Open(id.clone()))), *mounted && here),
        DeviceRow::Share => (Some(Message::Navigate(row.path.clone())), here),
        DeviceRow::Connect => (Some(Message::Perform(Action::ConnectToServer)), false),
        DeviceRow::Note => (None, false),
    })
}

/// The mark at the end of a mounted drive's row: press it to eject.
/// `None` for every other row.
pub(super) fn eject_button<'a>(row: &SidebarRow, scale: FontScale) -> Option<Element<'a, Message>> {
    let Some(DeviceRow::Volume { id, eject: true, .. }) = &row.device else { return None };
    // The icon's size, not the folder mark's: the glyph fills only half
    // its box, and at the mark's 13 pixels it was a speck. In the
    // foreground colour, because the row it sits on may be the current
    // one, and the dim colour all but vanished on the accent.
    let side = scale.apply(crate::density::SIDEBAR_ICON_BASE);
    Some(
        iced::widget::button(hyprforge_ui::glyph::eject(side, hyprforge_ui::theme::text()))
            .padding(0)
            .on_press(Message::Device(DeviceMessage::Eject(id.clone())))
            .style(super::quiet_link_style)
            .into(),
    )
}

/// The Devices and Remote sections, in that order. Either may come back
/// with no rows — the caller drops empty sections, the same rule every
/// other section follows.
pub(super) fn sections(vm: &ViewModel<'_>) -> [SidebarSection; 2] {
    let devices = vm.devices;
    let mut drives = Vec::new();
    match &devices.volumes {
        Listing::NotAsked => {}
        Listing::Unavailable(why) => drives.push(note(why)),
        Listing::Listed(volumes) => {
            for volume in volumes {
                let busy = devices.busy.get(&volume.id).copied();
                let mounted = volume.is_mounted();
                drives.push(SidebarRow {
                    label: volume.label.clone(),
                    // Where it is when it is somewhere; its device file
                    // until then, which is the one path it has.
                    path: volume.mount_point.clone().unwrap_or_else(|| volume.device.clone()),
                    meta: match (busy, volume.locked) {
                        (Some(op), _) => Some(op.doing().to_string()),
                        (None, true) => Some("Locked".to_string()),
                        (None, false) => None,
                    },
                    // Removable things in the info role — "somewhere
                    // else", the role DESIGN.md reserved for this — and
                    // an internal disk in plain dim.
                    tint: match volume.kind {
                        hyprforge_volumes::VolumeKind::Internal => Tint::Dim,
                        _ => Tint::Info,
                    },
                    missing: false,
                    icon: icon::themed_key(volume.icon_name()),
                    search: None,
                    device: Some(DeviceRow::Volume {
                        id: volume.id.clone(),
                        mounted,
                        eject: devices.manages_mounts && mounted && volume.detach.is_some() && busy.is_none(),
                    }),
                });
            }
        }
    }
    let mut remote: Vec<SidebarRow> = devices
        .shares
        .iter()
        .map(|share| SidebarRow {
            label: share.label.clone(),
            path: share.path.clone(),
            meta: devices.disconnecting.contains(&share.path).then(|| "Disconnecting\u{2026}".to_string()),
            tint: Tint::Info,
            missing: false,
            icon: icon::themed_key(share.icon_name()),
            search: None,
            device: Some(DeviceRow::Share),
        })
        .collect();
    if devices.manages_mounts {
        remote.push(SidebarRow {
            label: "Connect to Server\u{2026}".to_string(),
            path: PathBuf::new(),
            meta: None,
            tint: Tint::Dim,
            missing: false,
            icon: icon::themed_key(CONNECT_ICON),
            search: None,
            device: Some(DeviceRow::Connect),
        });
    }
    [SidebarSection { title: "Devices", rows: drives }, SidebarSection { title: "Remote", rows: remote }]
}

fn note(text: &str) -> SidebarRow {
    SidebarRow {
        label: text.to_string(),
        path: PathBuf::new(),
        meta: None,
        tint: Tint::Dim,
        // Drawn dim, as a pin that cannot be read is: it says something
        // is not available, and the dim is how this sidebar says that.
        missing: true,
        icon: icon::FOLDER_KEY.to_string(),
        search: None,
        device: Some(DeviceRow::Note),
    }
}

impl Browser {
    /// The window's drives and shares changed. Icons a new drive needs
    /// are asked for now — the first listing's batch has gone.
    pub(super) fn set_devices(&mut self, devices: Arc<Devices>) -> Outcome {
        self.devices = devices;
        if !matches!(self.load_state, super::LoadState::Loaded) {
            // The first listing asks for every icon the sidebar wants,
            // these included; nothing is asked before it.
            return Outcome::None;
        }
        let keys: Vec<String> = self
            .devices
            .icon_names()
            .into_iter()
            .map(icon::themed_key)
            .filter(|key| !self.icons.contains_key(key) && !self.icons_asked.contains(key))
            .collect();
        if keys.is_empty() {
            return Outcome::None;
        }
        self.icons_asked.extend(keys.iter().cloned());
        Outcome::LoadIcons(keys)
    }

    /// Theme icon keys for the drive and share rows, for the first
    /// listing's batch.
    pub(super) fn device_icon_keys(&self) -> Vec<String> {
        self.devices.icon_names().into_iter().map(icon::themed_key).collect()
    }

    pub(super) fn update_device(&mut self, message: DeviceMessage) -> Outcome {
        match message {
            DeviceMessage::Open(id) => {
                let Some(volume) = self.devices.volume(&id) else { return Outcome::None };
                if let Some(point) = volume.mount_point.clone() {
                    return self.update(Message::Navigate(point));
                }
                if self.devices.busy.contains_key(&id) {
                    // Already being mounted: the click that started it
                    // will go there.
                    return Outcome::None;
                }
                if volume.locked {
                    let label = volume.label.clone();
                    return Outcome::Notice(hyprforge_volumes::sentence(
                        Operation::Mount,
                        &label,
                        &hyprforge_volumes::VolumeError::Locked,
                    ));
                }
                Outcome::Devices(Ask::Mount { id, open: true })
            }
            DeviceMessage::Eject(id) => {
                if self.devices.busy.contains_key(&id) {
                    return Outcome::None;
                }
                Outcome::Devices(Ask::Eject(id))
            }
        }
    }

    /// Mount, Unmount, Eject or Disconnect — on a sidebar row's drive
    /// when the menu was opened on one, else on whatever the folder in
    /// view is on. Enabled-ness was checked by the caller against the
    /// same target.
    pub(super) fn device_action(&mut self, action: Action, target: Option<&Path>) -> Outcome {
        if action == Action::Disconnect {
            let share = match target {
                Some(t) => self.devices.shares.iter().find(|s| s.path == t),
                None => self.devices.share_holding(&self.current_dir),
            };
            return match share {
                Some(share) => Outcome::Devices(Ask::Disconnect(share.path.clone())),
                None => Outcome::None,
            };
        }
        let volume = match target {
            Some(t) => self.devices.volume_named(t),
            None => self.devices.volume_holding(&self.current_dir),
        };
        let Some(id) = volume.map(|v| v.id.clone()) else { return Outcome::None };
        Outcome::Devices(match action {
            // From a menu, mounting means mounting: the person asked for
            // that and not to be taken there, which is what a click on
            // the row is for.
            Action::Mount => Ask::Mount { id, open: false },
            Action::Unmount => Ask::Unmount(id),
            _ => Ask::Eject(id),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::browser::{MenuSpot, Mode};
    use crate::devices::Share;
    use crate::menu::MenuItem;
    use crate::prefs::Prefs;
    use hyprforge_volumes::{Detach, Volume, VolumeKind};

    fn stick(label: &str, mounted: Option<&str>) -> Volume {
        Volume {
            id: id(label),
            device: PathBuf::from(format!("/dev/{label}")),
            label: label.to_string(),
            size: 32_000_000,
            filesystem: Some("vfat".into()),
            mount_point: mounted.map(PathBuf::from),
            kind: VolumeKind::Removable,
            detach: Some(Detach::PowerOff),
            locked: false,
        }
    }

    fn id(label: &str) -> VolumeId {
        VolumeId(format!("/org/freedesktop/UDisks2/block_devices/{label}"))
    }

    fn share() -> Share {
        Share {
            label: "u@box".into(),
            path: "/run/user/1000/gvfs/sftp:host=box,user=u".into(),
            kind: hyprforge_volumes::ShareKind::Gvfs { scheme: "sftp".into() },
        }
    }

    fn with(devices: Devices) -> Browser {
        let (mut browser, _) = Browser::new(Mode::App, Prefs::default(), PathBuf::from("/home/u"), vec![]);
        browser.update(Message::DirLoaded(PathBuf::from("/home/u"), Ok(vec![])));
        browser.update(Message::DevicesChanged(Arc::new(devices)));
        browser
    }

    /// As the window hands them over: a host that manages mounts.
    fn listed(volumes: Vec<Volume>) -> Devices {
        Devices { volumes: Listing::Listed(volumes), manages_mounts: true, ..Devices::default() }
    }

    fn titles(browser: &Browser) -> Vec<&'static str> {
        super::super::sidebar_sections(&browser.view_model(1000.0)).iter().map(|s| s.title).collect()
    }

    fn section(browser: &Browser, title: &str) -> SidebarSection {
        super::super::sidebar_sections(&browser.view_model(1000.0))
            .into_iter()
            .find(|s| s.title == title)
            .unwrap_or_else(|| panic!("no {title} section"))
    }

    fn enabled(browser: &Browser) -> Vec<&'static str> {
        browser
            .menu
            .as_ref()
            .expect("a menu is open")
            .items
            .iter()
            .filter_map(|i| match i {
                MenuItem::Action { label, enabled: true, .. } => Some(*label),
                _ => None,
            })
            .collect()
    }

    fn choose(browser: &mut Browser, label: &str) -> Outcome {
        let index = browser
            .menu
            .as_ref()
            .unwrap()
            .items
            .iter()
            .position(|i| matches!(i, MenuItem::Action { label: l, .. } if *l == label))
            .unwrap_or_else(|| panic!("no {label}"));
        browser.update(Message::MenuChose(index))
    }

    /// A host that never asked about drives shows no Devices heading at
    /// all, rather than an empty one claiming there are none.
    #[test]
    fn no_answer_yet_is_no_devices_section() {
        let browser = with(Devices::default());
        assert!(!titles(&browser).contains(&"Devices"));
        assert!(!titles(&browser).contains(&"Remote"));
    }

    /// UDisks2 down is a state with a message: the heading stays, with
    /// a sentence under it that is not a button.
    #[test]
    fn udisks_not_running_is_said_in_the_section_not_hidden() {
        let mut devices = Devices::default();
        devices.listed(Err(hyprforge_volumes::VolumeError::Unavailable));
        let browser = with(devices);
        let section = section(&browser, "Devices");
        assert_eq!(section.rows.len(), 1);
        assert!(section.rows[0].label.contains("UDisks2 isn't running"));
        let vm = browser.view_model(1000.0);
        assert_eq!(super::super::sidebar_press(&section.rows[0], &vm).0, None, "nowhere to go");
    }

    #[test]
    fn the_sections_sit_between_the_users_lists_and_the_trash() {
        let mut devices = listed(vec![stick("STICK", None)]);
        devices.manages_mounts = true;
        let browser = with(devices);
        // Places is there with no places built: Recent and Starred head it.
        assert_eq!(titles(&browser), ["Places", "Devices", "Remote", "Trash"]);
    }

    #[test]
    fn clicking_an_unmounted_drive_mounts_it_and_goes_there() {
        let mut browser = with(listed(vec![stick("STICK", None)]));
        let row = section(&browser, "Devices").rows.remove(0);
        let vm = browser.view_model(1000.0);
        let (press, current) = super::super::sidebar_press(&row, &vm);
        assert!(!current);
        assert_eq!(browser.update(press.unwrap()), Outcome::Devices(Ask::Mount { id: id("STICK"), open: true }));
    }

    #[test]
    fn clicking_a_mounted_drive_just_goes_there() {
        let mut browser = with(listed(vec![stick("STICK", Some("/run/media/u/STICK"))]));
        let outcome = browser.update(Message::Device(DeviceMessage::Open(id("STICK"))));
        assert!(outcome.navigates(), "{outcome:?}");
        assert_eq!(browser.current_dir(), Path::new("/run/media/u/STICK"));
    }

    /// The second click on a stick still mounting is not a second
    /// mount, and the row says what is happening.
    #[test]
    fn a_drive_already_mounting_is_not_asked_twice() {
        let mut devices = listed(vec![stick("STICK", None)]);
        devices.begin(&id("STICK"), Operation::Mount);
        let mut browser = with(devices);
        assert_eq!(browser.update(Message::Device(DeviceMessage::Open(id("STICK")))), Outcome::None);
        assert_eq!(section(&browser, "Devices").rows[0].meta.as_deref(), Some("Mounting\u{2026}"));
    }

    #[test]
    fn a_locked_drive_says_why_it_will_not_open() {
        let mut locked = stick("SECRET", None);
        locked.locked = true;
        let mut browser = with(listed(vec![locked]));
        let outcome = browser.update(Message::Device(DeviceMessage::Open(id("SECRET"))));
        assert!(matches!(&outcome, Outcome::Notice(s) if s.contains("encrypted")), "{outcome:?}");
    }

    /// Unmounted, there is nothing to open in a tab and nothing to
    /// unmount; mounted, the reverse. The menu keeps one shape.
    #[test]
    fn a_drives_menu_offers_what_its_state_allows() {
        let mut browser = with(listed(vec![stick("A", None), stick("B", Some("/run/media/u/B"))]));
        browser.update(Message::OpenContextMenu { spot: MenuSpot::Sidebar("/dev/A".into()), at: (0.0, 0.0) });
        assert_eq!(enabled(&browser), ["Mount", "Eject"]);
        assert_eq!(choose(&mut browser, "Mount"), Outcome::Devices(Ask::Mount { id: id("A"), open: false }));
        browser.update(Message::OpenContextMenu { spot: MenuSpot::Sidebar("/run/media/u/B".into()), at: (0.0, 0.0) });
        assert_eq!(enabled(&browser), ["Open in New Tab", "Unmount", "Eject"]);
        assert_eq!(choose(&mut browser, "Eject"), Outcome::Devices(Ask::Eject(id("B"))));
    }

    /// From the palette or a key, the drive actions mean the drive you
    /// are standing on.
    #[test]
    fn unmount_with_no_row_means_the_drive_in_view() {
        let mut browser = with(listed(vec![stick("B", Some("/run/media/u/B"))]));
        assert_eq!(browser.perform(Action::Unmount), Outcome::None, "not on a drive: nothing to unmount");
        browser.update(Message::Navigate("/run/media/u/B/photos".into()));
        assert_eq!(browser.perform(Action::Unmount), Outcome::Devices(Ask::Unmount(id("B"))));
    }

    #[test]
    fn the_eject_mark_shows_only_on_a_mounted_drive_that_can_let_go() {
        let mut internal = stick("DATA", Some("/run/media/u/DATA"));
        internal.detach = None;
        internal.kind = VolumeKind::Internal;
        let browser = with(listed(vec![stick("A", None), stick("B", Some("/run/media/u/B")), internal]));
        let rows = section(&browser, "Devices").rows;
        let ejects: Vec<bool> =
            rows.iter().map(|r| matches!(r.device, Some(DeviceRow::Volume { eject: true, .. }))).collect();
        assert_eq!(ejects, [false, true, false]);
        assert_eq!(rows[2].tint, Tint::Dim, "an internal disk is not 'somewhere else'");
        assert_eq!(rows[1].tint, Tint::Info);
    }

    #[test]
    fn a_share_is_a_place_with_a_disconnect_in_its_menu() {
        let devices = Devices { shares: vec![share()], manages_mounts: true, ..Devices::default() };
        let mut browser = with(devices);
        let rows = section(&browser, "Remote").rows;
        assert_eq!(rows.iter().map(|r| r.label.as_str()).collect::<Vec<_>>(), ["u@box", "Connect to Server\u{2026}"]);
        browser.update(Message::OpenContextMenu { spot: MenuSpot::Sidebar(share().path), at: (0.0, 0.0) });
        assert!(enabled(&browser).contains(&"Disconnect"));
        assert_eq!(choose(&mut browser, "Disconnect"), Outcome::Devices(Ask::Disconnect(share().path)));
    }

    /// Connect to Server is the window's: from the dialog it would be a
    /// row that does nothing, so the dialog's host leaves it off.
    #[test]
    fn connect_to_server_appears_only_when_the_host_offers_it() {
        assert!(!titles(&with(Devices::default())).contains(&"Remote"));
        let mut browser = with(Devices { manages_mounts: true, ..Devices::default() });
        let row = section(&browser, "Remote").rows.remove(0);
        let vm = browser.view_model(1000.0);
        let (press, _) = super::super::sidebar_press(&row, &vm);
        assert_eq!(press, Some(Message::Perform(Action::ConnectToServer)));
        assert_eq!(browser.update(press.unwrap()), Outcome::Window(Action::ConnectToServer));
    }

    /// The dialog's menus leave drives alone; its rows still mount on a
    /// click, because a stick you cannot open is one you cannot save to.
    #[test]
    fn the_dialog_offers_no_drive_menu_or_eject_but_still_mounts_on_a_click() {
        let (mut dialog, _) = Browser::new(
            Mode::Dialog(crate::browser::DialogKind::Save),
            Prefs::default(),
            PathBuf::from("/home/u"),
            vec![],
        );
        let config = crate::config::Config { menus: crate::menu::MenuConfig::dialog(), ..Default::default() };
        dialog.set_config(Arc::new(config));
        // As the dialog's host hands them over: it does not manage mounts.
        let devices = Devices { manages_mounts: false, ..listed(vec![stick("A", None), stick("B", Some("/m/B"))]) };
        dialog.update(Message::DevicesChanged(Arc::new(devices)));
        dialog.update(Message::OpenContextMenu { spot: MenuSpot::Sidebar("/dev/A".into()), at: (0.0, 0.0) });
        assert!(dialog.menu.is_none());
        let rows = section(&dialog, "Devices").rows;
        assert!(
            !rows.iter().any(|r| matches!(r.device, Some(DeviceRow::Volume { eject: true, .. }))),
            "no eject mark in a file chooser"
        );
        assert!(!titles(&dialog).contains(&"Remote"), "no Connect to Server either");
        let outcome = dialog.update(Message::Device(DeviceMessage::Open(id("A"))));
        assert!(matches!(outcome, Outcome::Devices(Ask::Mount { open: true, .. })), "{outcome:?}");
    }

    /// Icons for drives that arrive after the first listing are asked
    /// for when they arrive, once.
    #[test]
    fn a_new_drive_asks_for_its_icon_once() {
        let mut browser = with(Devices::default());
        let outcome = browser.update(Message::DevicesChanged(Arc::new(listed(vec![stick("A", None)]))));
        assert_eq!(
            outcome,
            Outcome::LoadIcons(vec![icon::themed_key("drive-removable-media"), icon::themed_key(CONNECT_ICON)])
        );
        let again = browser.update(Message::DevicesChanged(Arc::new(listed(vec![stick("B", None)]))));
        assert_eq!(again, Outcome::None);
    }
}
