//! A [`System`] with no system behind it.
//!
//! Units, processes, binaries and live binds are whatever the test says
//! they are, and every call that would have changed something is recorded
//! in [`MockSystem::calls`] so a test can assert on what setup *did* as
//! well as on what it reported.
//!
//! The one piece of systemd behaviour it models on purpose is the one
//! that is easy to get wrong: a unit enabled for every user (by a
//! package's `systemctl --global preset`) is *not* disabled by
//! `systemctl --user disable` — that only removes per-user links. Only a
//! mask turns it off for one user. Undo has to know that.

use crate::system::{Generated, Loaded, System, UnitState};
use hyprforge_shortcuts::binds::LiveBind;
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// One systemd user unit, as the mock models it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MockUnit {
    /// Enabled by a per-user link, which `disable` removes.
    pub enabled_for_user: bool,
    /// Enabled for every user by a package preset, which `disable` leaves.
    pub enabled_globally: bool,
    pub masked: bool,
    pub active: bool,
}

#[derive(Debug)]
struct Inner {
    binaries: BTreeMap<String, PathBuf>,
    units: BTreeMap<String, MockUnit>,
    processes: BTreeMap<String, bool>,
    binds: Result<Vec<LiveBind>, String>,
    lua_rejects: Option<String>,
    systemctl_unanswered: bool,
    calls: Vec<String>,
    /// snapper configs by name, as files a set-config rewrites.
    snapper_files: BTreeMap<String, PathBuf>,
    /// What `fprintd-list` would say: enrolled or not, or no answer.
    fingerprint: Option<bool>,
    /// Make the next root write fail, as a dismissed prompt does.
    root_refuses: bool,
}

/// See the module doc.
#[derive(Debug)]
pub struct MockSystem {
    inner: RefCell<Inner>,
}

impl Default for MockSystem {
    fn default() -> Self {
        MockSystem {
            inner: RefCell::new(Inner {
                binaries: BTreeMap::new(),
                units: BTreeMap::new(),
                processes: BTreeMap::new(),
                binds: Ok(Vec::new()),
                lua_rejects: None,
                systemctl_unanswered: false,
                calls: Vec::new(),
                snapper_files: BTreeMap::new(),
                fingerprint: None,
                root_refuses: false,
            }),
        }
    }
}

impl MockSystem {
    pub fn new() -> MockSystem {
        MockSystem::default()
    }

    /// A machine with every binary and unit the suite installs, all
    /// disabled, and Hyprland answering with no binds.
    pub fn with_suite_installed() -> MockSystem {
        let sys = MockSystem::new();
        for name in [
            "hyprforge-clipmenu",
            "hyprforge-emojimenu",
            "notifctl",
            "notifd",
            "hyprforge-files",
            "hyprforge-files-portal",
            "hyprforge-lock",
            "hyprforge-media",
            "hyprforge-displayd",
            "hyprforge-trayd",
            "hyprforge-clipd",
            "hypridle",
        ] {
            sys.install_binary(name);
        }
        for unit in crate::items::SUITE_UNITS {
            sys.set_unit(unit, MockUnit::default());
        }
        sys
    }

    pub fn install_binary(&self, name: &str) {
        self.inner
            .borrow_mut()
            .binaries
            .insert(name.to_string(), PathBuf::from("/usr/bin").join(name));
    }

    pub fn remove_binary(&self, name: &str) {
        self.inner.borrow_mut().binaries.remove(name);
    }

    pub fn set_unit(&self, unit: &str, state: MockUnit) {
        self.inner.borrow_mut().units.insert(unit.to_string(), state);
    }

    pub fn remove_unit(&self, unit: &str) {
        self.inner.borrow_mut().units.remove(unit);
    }

    pub fn unit(&self, unit: &str) -> Option<MockUnit> {
        self.inner.borrow().units.get(unit).cloned()
    }

    pub fn set_process(&self, name: &str, running: bool) {
        self.inner.borrow_mut().processes.insert(name.to_string(), running);
    }

    pub fn set_binds(&self, binds: Result<Vec<LiveBind>, String>) {
        self.inner.borrow_mut().binds = binds;
    }

    /// Makes every `apply_lua` refuse with `message`, the way Hyprland
    /// refuses a file — the file is put back as it was.
    pub fn reject_lua(&self, message: Option<&str>) {
        self.inner.borrow_mut().lua_rejects = message.map(str::to_string);
    }

    /// Makes every systemctl question fail to be asked, as a `systemctl`
    /// that timed out would.
    pub fn systemctl_unanswered(&self, unanswered: bool) {
        self.inner.borrow_mut().systemctl_unanswered = unanswered;
    }

    /// Every call that changed something, in order: `enable-now notifd.service`,
    /// `apply-lua /…/keybinds.lua`, `restart-idle`, ….
    pub fn calls(&self) -> Vec<String> {
        self.inner.borrow().calls.clone()
    }

    /// Points snapper's `config` at `path`, for set-config to rewrite.
    pub fn with_snapper_config(&self, config: &str, path: PathBuf) {
        self.inner.borrow_mut().snapper_files.insert(config.to_string(), path);
    }

    /// What `fprintd-list` answers.
    pub fn with_fingerprint(&self, enrolled: Option<bool>) {
        self.inner.borrow_mut().fingerprint = enrolled;
    }

    /// Root writes fail from now on, as when the prompt is dismissed.
    pub fn refusing_root(&self) {
        self.inner.borrow_mut().root_refuses = true;
    }

    fn record(&self, call: String) {
        self.inner.borrow_mut().calls.push(call);
    }

    fn with_unit(
        &self,
        verb: &str,
        unit: &str,
        f: impl FnOnce(&mut MockUnit),
    ) -> Result<(), String> {
        self.record(format!("{verb} {unit}"));
        let mut inner = self.inner.borrow_mut();
        if inner.systemctl_unanswered {
            return Err("systemctl timed out".into());
        }
        let Some(state) = inner.units.get_mut(unit) else {
            return Err(format!("Unit {unit} not found."));
        };
        f(state);
        Ok(())
    }
}

impl System for MockSystem {
    fn find_binary(&self, name: &str) -> Option<PathBuf> {
        self.inner.borrow().binaries.get(name).cloned()
    }

    fn unit_state(&self, unit: &str) -> Result<UnitState, String> {
        let inner = self.inner.borrow();
        if inner.systemctl_unanswered {
            return Err("systemctl timed out".into());
        }
        Ok(match inner.units.get(unit) {
            None => UnitState::NotFound,
            Some(u) if u.masked => UnitState::Masked,
            Some(u) if u.enabled_for_user || u.enabled_globally => UnitState::Enabled,
            Some(_) => UnitState::Disabled,
        })
    }

    fn unit_active(&self, unit: &str) -> Result<bool, String> {
        let inner = self.inner.borrow();
        if inner.systemctl_unanswered {
            return Err("systemctl timed out".into());
        }
        Ok(inner.units.get(unit).is_some_and(|u| u.active))
    }

    fn enable_now(&self, unit: &str) -> Result<(), String> {
        self.with_unit("enable-now", unit, |u| {
            u.enabled_for_user = true;
            u.active = !u.masked;
        })
    }

    fn disable_now(&self, unit: &str) -> Result<(), String> {
        // Removes the per-user link only — see the module doc.
        self.with_unit("disable-now", unit, |u| {
            u.enabled_for_user = false;
            u.active = false;
        })
    }

    fn mask_now(&self, unit: &str) -> Result<(), String> {
        self.with_unit("mask-now", unit, |u| {
            u.masked = true;
            u.active = false;
        })
    }

    fn unmask(&self, unit: &str) -> Result<(), String> {
        self.with_unit("unmask", unit, |u| u.masked = false)
    }

    fn try_restart(&self, unit: &str) -> Result<(), String> {
        self.record(format!("try-restart {unit}"));
        Ok(())
    }

    fn process_running(&self, name: &str) -> Option<bool> {
        Some(self.inner.borrow().processes.get(name).copied().unwrap_or(false))
    }

    fn live_binds(&self) -> Result<Vec<LiveBind>, String> {
        self.inner.borrow().binds.clone()
    }

    fn apply_lua(&self, file: &Generated) -> Result<Loaded, String> {
        self.record(format!("apply-lua {}", file.path.display()));
        if let Some(message) = self.inner.borrow().lua_rejects.clone() {
            // Hyprland refused it, and the real apply puts the previous
            // file back — so this never writes at all.
            return Err(message);
        }
        if let Some(dir) = file.path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        std::fs::write(&file.path, &file.contents).map_err(|e| e.to_string())?;
        Ok(Loaded::Live)
    }

    fn restart_idle(&self) -> Result<(), String> {
        self.record("restart-idle".to_string());
        Ok(())
    }

    fn reload_session_bus(&self) -> Result<(), String> {
        self.record("reload-session-bus".to_string());
        Ok(())
    }

    fn fingerprint_enrolled(&self, _user: &str) -> Option<bool> {
        self.inner.borrow().fingerprint
    }

    /// Recorded, and done for real — to the test's own rooted paths.
    fn write_as_root(&self, path: &Path, content: Option<&str>) -> Result<(), String> {
        if self.inner.borrow().root_refuses {
            return Err("the password prompt was dismissed".to_string());
        }
        match content {
            Some(text) => {
                self.record(format!("write-as-root {}", path.display()));
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
                }
                std::fs::write(path, text).map_err(|e| e.to_string())
            }
            None => {
                self.record(format!("remove-as-root {}", path.display()));
                match std::fs::remove_file(path) {
                    Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.to_string()),
                    _ => Ok(()),
                }
            }
        }
    }

    /// Recorded, and written into the config file the mock was pointed
    /// at — which is what snapper itself does — so a check after an apply
    /// sees the change.
    fn snapper_set_config(&self, config: &str, settings: &[(String, String)]) -> Result<(), String> {
        let pairs: Vec<String> = settings.iter().map(|(k, v)| format!("{k}={v}")).collect();
        self.record(format!("snapper -c {config} set-config {}", pairs.join(" ")));
        let Some(path) = self.inner.borrow().snapper_files.get(config).cloned() else { return Ok(()) };
        let mut text = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
        for (key, value) in settings {
            let wanted = format!("{key}=\"{value}\"");
            let lines: Vec<String> = text
                .lines()
                .map(|l| if l.trim_start().starts_with(&format!("{key}=")) { wanted.clone() } else { l.to_string() })
                .collect();
            text = lines.join("\n") + "\n";
        }
        std::fs::write(&path, text).map_err(|e| e.to_string())
    }
}
