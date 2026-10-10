//! Every item against a temp directory and [`MockSystem`] — nothing here
//! can reach the real `~/.config`, systemd or Hyprland.

use crate::mock::{MockSystem, MockUnit};
use crate::record::{self, Change};
use crate::{apply, check, check_all, item, turn_off_for_me, undo, Env, State, ITEMS};
use hyprforge_shortcuts::binds::LiveBind;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

struct Rig {
    dir: tempfile::TempDir,
    env: Env,
    sys: MockSystem,
}

/// An executable-looking file, so a desktop entry naming it by absolute
/// path counts as installed without anything on the real `$PATH`.
fn fake_program(dir: &Path, name: &str) -> PathBuf {
    let bin = dir.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let path = bin.join(name);
    std::fs::write(&path, "#!/bin/sh\n").unwrap();
    path
}

fn desktop_entry(env: &Env, id: &str, name: &str, exec: &Path, mimes: &str) {
    let apps = env.data_home.join("applications");
    std::fs::create_dir_all(&apps).unwrap();
    std::fs::write(
        apps.join(id),
        format!(
            "[Desktop Entry]\nType=Application\nName={name}\nExec={} %U\nMimeType={mimes}\n",
            exec.display()
        ),
    )
    .unwrap();
}

/// A machine with the whole suite installed, a `hyprland.lua` of the
/// user's own, Nemo as their folder handler, and nothing set up.
fn rig() -> Rig {
    let dir = tempfile::tempdir().unwrap();
    let env = Env::rooted_at(dir.path());
    std::fs::create_dir_all(env.hypr_dir()).unwrap();
    std::fs::write(env.hyprland_lua(), "-- my config\nhl.config({ general = { gaps_in = 4 } })\n")
        .unwrap();
    let files = fake_program(dir.path(), "hyprforge-files");
    let media = fake_program(dir.path(), "hyprforge-media");
    let nemo = fake_program(dir.path(), "nemo");
    desktop_entry(&env, "hyprforge-files.desktop", "Files", &files, "inode/directory;");
    desktop_entry(&env, "hyprforge-media.desktop", "Media", &media, "image/png;image/jpeg;");
    desktop_entry(&env, "nemo.desktop", "Nemo", &nemo, "inode/directory;");
    std::fs::write(env.mimeapps_list(), "[Default Applications]\ninode/directory=nemo.desktop\n")
        .unwrap();
    let sys = MockSystem::with_suite_installed();
    // snapper's `home` config, as Arch ships it with one other user
    // already allowed — for Previous Versions.
    sys.install_binary("snapper");
    sys.install_binary("pkexec");
    std::fs::create_dir_all(&env.snapper_configs).unwrap();
    let snapper_home = env.snapper_configs.join("home");
    std::fs::write(&snapper_home, "SUBVOLUME=\"/home\"\nALLOW_USERS=\"sam\"\nSYNC_ACL=\"no\"\n").unwrap();
    sys.with_snapper_config("home", snapper_home);
    // A fingerprint reader with a finger enrolled, pam_fprintd installed,
    // and polkit's stack as Arch ships it, with no local copy — for the
    // fingerprint item.
    sys.install_binary("fprintd-list");
    sys.with_fingerprint(Some(true));
    std::fs::create_dir_all(&env.pam_modules).unwrap();
    std::fs::write(env.pam_modules.join("pam_fprintd.so"), "").unwrap();
    std::fs::create_dir_all(&env.vendor_pam_dir).unwrap();
    std::fs::write(env.vendor_pam_dir.join("polkit-1"), POLKIT_STACK).unwrap();
    Rig { dir, env, sys }
}

/// Arch's `/usr/lib/pam.d/polkit-1`.
const POLKIT_STACK: &str = "#%PAM-1.0\n\nauth       include      system-auth\naccount    include      system-auth\npassword   include      system-auth\nsession    include      system-auth\n";

impl Rig {
    fn state(&self, id: &str) -> State {
        check(&self.env, &self.sys, item(id).unwrap())
    }

    fn apply(&self, id: &str) -> Result<String, String> {
        let mut outcomes = apply(&self.env, &self.sys, &[id]).expect("record readable");
        assert_eq!(outcomes.len(), 1);
        outcomes.remove(0).result
    }

    fn undo(&self, id: &str) -> Result<String, String> {
        let mut outcomes = undo(&self.env, &self.sys, &[id]).expect("record readable");
        assert_eq!(outcomes.len(), 1);
        outcomes.remove(0).result
    }

    /// Every file under the rig and what is in it.
    fn snapshot(&self) -> BTreeMap<PathBuf, String> {
        fn walk(dir: &Path, out: &mut BTreeMap<PathBuf, String>) {
            for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
                let path = entry.path();
                if path.is_dir() {
                    walk(&path, out);
                } else {
                    out.insert(path.clone(), std::fs::read_to_string(&path).unwrap_or_default());
                }
            }
        }
        let mut out = BTreeMap::new();
        walk(self.dir.path(), &mut out);
        out
    }

    fn read(&self, path: &Path) -> String {
        std::fs::read_to_string(path).unwrap_or_default()
    }
}

fn is_todo(state: &State) -> bool {
    matches!(state, State::Todo { .. })
}

// --- the three properties every item has -------------------------------

/// Check before, apply, check after is done; applying again changes
/// nothing; undo brings it back to to-do. For every item, so a new one
/// cannot be added without meeting all three.
#[test]
fn every_item_applies_to_done_twice_harmlessly_and_undoes_to_todo() {
    for item in ITEMS.iter() {
        let r = rig();
        let before = r.state(item.id);
        assert!(is_todo(&before), "{} should start to do, was {before:?}", item.id);

        let applied = r.apply(item.id);
        assert!(applied.is_ok(), "{} failed to apply: {applied:?}", item.id);
        assert_eq!(r.state(item.id), State::Done, "{} is not done after applying", item.id);

        let files = r.snapshot();
        let calls = r.sys.calls().len();
        assert_eq!(r.apply(item.id), Ok("already done".to_string()), "{}", item.id);
        assert_eq!(r.snapshot(), files, "{} changed files on a second apply", item.id);
        assert_eq!(r.sys.calls().len(), calls, "{} acted again on a second apply", item.id);

        let undone = r.undo(item.id);
        assert!(undone.is_ok(), "{} failed to undo: {undone:?}", item.id);
        assert!(is_todo(&r.state(item.id)), "{} is not to do after undo", item.id);
        assert!(
            record::load(&r.env.setup_toml()).unwrap().items.is_empty(),
            "{} left its record behind after undo",
            item.id
        );
    }
}

/// Applying everything at once, then undoing everything, leaves the
/// user's own files as they were.
#[test]
fn undoing_everything_puts_the_users_own_files_back() {
    let r = rig();
    let hyprland = r.read(&r.env.hyprland_lua());
    let mimeapps = r.read(&r.env.mimeapps_list());
    let ids: Vec<&str> = ITEMS.iter().map(|i| i.id).collect();
    let outcomes = apply(&r.env, &r.sys, &ids).unwrap();
    assert!(outcomes.iter().all(|o| o.ok()), "{outcomes:?}");
    assert!(check_all(&r.env, &r.sys).iter().all(|(_, s)| *s == State::Done));

    let outcomes = undo(&r.env, &r.sys, &[]).unwrap();
    assert_eq!(outcomes.len(), ITEMS.len());
    assert!(outcomes.iter().all(|o| o.ok()), "{outcomes:?}");
    // Only the marker comments remain of the wiring — see `wiring::undo`.
    let after: Vec<String> = r
        .read(&r.env.hyprland_lua())
        .lines()
        .filter(|l| !l.starts_with("-- ") || l.starts_with("-- my"))
        .map(str::to_string)
        .collect();
    assert_eq!(after.join("\n") + "\n", hyprland);
    assert_eq!(r.read(&r.env.mimeapps_list()), mimeapps);
    assert!(!r.env.portals_conf().exists(), "a portals.conf setup created is removed again");
    assert!(!r.env.file_manager1_service().exists());
}

// --- keybinds ------------------------------------------------------------

#[test]
fn a_bind_lands_in_shortcuts_toml_with_the_hyprforge_description() {
    let r = rig();
    r.apply("bind-clipboard").unwrap();
    let shortcuts = hyprforge_shortcuts::storage::load(&r.env.shortcuts_toml()).unwrap();
    assert_eq!(shortcuts.len(), 1);
    assert_eq!(shortcuts[0].description, "Clipboard history");
    let lua = r.read(&r.env.keybinds_lua());
    assert!(
        lua.contains("hl.bind([[SUPER + V]], hl.dsp.exec_cmd([[hyprforge-clipmenu]]), { description = [[hyprforge: Clipboard history]] })"),
        "{lua}"
    );
}

/// The user already has Super+E in their own config. Setup names what
/// holds it and writes nothing.
#[test]
fn a_chord_bound_in_the_users_config_is_skipped_and_named() {
    let r = rig();
    r.sys.set_binds(Ok(vec![LiveBind {
        modmask: 64,
        key: "E".into(),
        dispatcher: "__lua".into(),
        description: "my editor".into(),
        ..Default::default()
    }]));
    let state = r.state("bind-files");
    let State::Unavailable { why } = &state else { panic!("{state:?}") };
    assert!(why.contains("Super+E") && why.contains("my editor"), "{why}");

    let files = r.snapshot();
    assert!(r.apply("bind-files").is_err());
    assert_eq!(r.snapshot(), files, "nothing was written");
    assert!(!r.env.shortcuts_toml().exists());
}

/// Hyprland reports whichever spelling the config wrote; `.` is Super+.
/// as much as `period` is.
#[test]
fn a_full_stop_bound_by_its_character_still_counts_as_taken() {
    let r = rig();
    r.sys.set_binds(Ok(vec![LiveBind { modmask: 64, key: ".".into(), ..Default::default() }]));
    assert!(matches!(r.state("bind-emoji"), State::Unavailable { .. }));
}

#[test]
fn a_chord_taken_by_another_hyprforge_shortcut_is_skipped_and_named() {
    let r = rig();
    let mut theirs = hyprforge_shortcuts::Shortcut {
        name: "hyprforge-term-1".into(),
        enabled: true,
        combo: hyprforge_shortcuts::KeyCombo {
            mods: vec![hyprforge_shortcuts::Modifier::Super],
            key: "v".into(),
        },
        description: "Terminal".into(),
        ..Default::default()
    };
    theirs.action.dispatcher = "exec_cmd".into();
    hyprforge_shortcuts::storage::save(&r.env.shortcuts_toml(), &[theirs.clone()]).unwrap();

    let state = r.state("bind-clipboard");
    let State::Unavailable { why } = &state else { panic!("{state:?}") };
    assert!(why.contains("Terminal"), "{why}");
    assert!(r.apply("bind-clipboard").is_err());
    assert_eq!(hyprforge_shortcuts::storage::load(&r.env.shortcuts_toml()).unwrap(), vec![theirs]);
}

/// Moved to another chord in Settings → Shortcuts is still done.
#[test]
fn a_command_already_bound_on_another_chord_is_done() {
    let r = rig();
    r.apply("bind-files").unwrap();
    let mut shortcuts = hyprforge_shortcuts::storage::load(&r.env.shortcuts_toml()).unwrap();
    shortcuts[0].combo.key = "F".into();
    hyprforge_shortcuts::storage::save(&r.env.shortcuts_toml(), &shortcuts).unwrap();
    assert_eq!(r.state("bind-files"), State::Done);
}

#[test]
fn hyprland_not_answering_makes_a_bind_unknown_not_done() {
    let r = rig();
    r.sys.set_binds(Err("hyprctl: no instance".into()));
    assert!(matches!(r.state("bind-lock"), State::Unknown { .. }));
}

/// Hyprland refused the file: the Lua was rolled back by apply_lua, and
/// shortcuts.toml must follow it, with nothing recorded.
#[test]
fn a_rejected_keybinds_file_leaves_shortcuts_toml_and_the_record_as_they_were() {
    let r = rig();
    r.sys.reject_lua(Some("Hyprland rejected the generated shortcuts"));
    assert!(r.apply("bind-dnd").is_err());
    assert!(hyprforge_shortcuts::storage::load(&r.env.shortcuts_toml()).unwrap().is_empty());
    assert!(record::load(&r.env.setup_toml()).unwrap().items.is_empty());
}

// --- services ------------------------------------------------------------

/// The packages enable the daemons for every user. Setup did not, so it
/// records nothing, and undo must not claim to have turned it off —
/// `systemctl --user disable` could not have, anyway.
#[test]
fn a_globally_enabled_unit_setup_didnt_enable_is_not_disabled_by_undo() {
    let r = rig();
    let unit = "hyprforge-trayd.service";
    r.sys.set_unit(unit, MockUnit { enabled_globally: true, active: true, ..Default::default() });
    assert_eq!(r.state("service-trayd"), State::Done);
    assert_eq!(r.apply("service-trayd"), Ok("already done".into()));

    let undone = r.undo("service-trayd").unwrap();
    assert!(undone.contains("nothing to undo"), "{undone}");
    assert!(r.sys.calls().iter().all(|c| !c.contains(unit)), "{:?}", r.sys.calls());
    assert!(r.sys.unit(unit).unwrap().enabled_globally, "still enabled for everyone");
}

/// The explicit off switch for that case is a mask, recorded, and undone
/// by unmasking.
#[test]
fn turning_off_a_globally_enabled_unit_masks_it_and_undo_unmasks_it() {
    let r = rig();
    let unit = "hyprforge-clipd.service";
    r.sys.set_unit(unit, MockUnit { enabled_globally: true, active: true, ..Default::default() });
    let outcome = turn_off_for_me(&r.env, &r.sys, "service-clipd").unwrap();
    assert!(outcome.ok(), "{outcome:?}");
    assert!(r.sys.unit(unit).unwrap().masked);
    assert!(matches!(r.state("service-clipd"), State::Unavailable { .. }));

    let undone = r.undo("service-clipd").unwrap();
    assert!(undone.contains("unmasked"), "{undone}");
    assert!(!r.sys.unit(unit).unwrap().masked);
    assert_eq!(r.state("service-clipd"), State::Done);
}

/// Setup enabled it, and a package also enabled it for everyone: undo
/// removes setup's link and says the unit is still on, rather than
/// reporting it off.
#[test]
fn undo_says_when_a_unit_it_disabled_is_still_enabled_for_everyone() {
    let r = rig();
    let unit = "hyprforge-displayd.service";
    r.apply("service-displayd").unwrap();
    let mut state = r.sys.unit(unit).unwrap();
    state.enabled_globally = true;
    r.sys.set_unit(unit, state);
    let undone = r.undo("service-displayd").unwrap();
    assert!(undone.contains("still enabled for every user"), "{undone}");
}

#[test]
fn a_service_that_isnt_installed_is_unavailable_not_todo() {
    let r = rig();
    r.sys.remove_unit("hyprforge-displayd.service");
    assert!(matches!(r.state("service-displayd"), State::Unavailable { .. }));
    r.sys.remove_binary("hyprforge-trayd");
    assert!(matches!(r.state("service-trayd"), State::Unavailable { .. }));
}

#[test]
fn systemctl_not_answering_is_unknown_never_done() {
    let r = rig();
    r.sys.set_unit("hyprforge-trayd.service", MockUnit { enabled_globally: true, ..Default::default() });
    r.sys.systemctl_unanswered(true);
    assert!(matches!(r.state("service-trayd"), State::Unknown { .. }));
    assert!(matches!(r.state("service-notifd"), State::Unknown { .. }));
}

/// dunst is named before anything happens, turned off by apply, and
/// brought back by undo.
#[test]
fn a_competing_notification_daemon_is_named_replaced_and_restored() {
    let r = rig();
    r.sys.set_unit("dunst.service", MockUnit { enabled_for_user: true, active: true, ..Default::default() });
    let state = r.state("service-notifd");
    let State::Todo { what } = &state else { panic!("{state:?}") };
    assert!(what.contains("Replace dunst"), "{what}");

    r.apply("service-notifd").unwrap();
    let dunst = r.sys.unit("dunst.service").unwrap();
    assert!(!dunst.enabled_for_user && !dunst.active);
    assert!(r.sys.unit("notifd.service").unwrap().active);
    assert_eq!(r.state("service-notifd"), State::Done);

    r.undo("service-notifd").unwrap();
    let dunst = r.sys.unit("dunst.service").unwrap();
    assert!(dunst.enabled_for_user && dunst.active);
    assert!(!r.sys.unit("notifd.service").unwrap().enabled_for_user);
}

/// A competitor a package enabled for everyone survives `disable`, so it
/// is masked — and unmasked again on undo.
#[test]
fn a_globally_enabled_competitor_is_masked_and_unmasked() {
    let r = rig();
    r.sys.set_unit("mako.service", MockUnit { enabled_globally: true, active: true, ..Default::default() });
    r.apply("service-notifd").unwrap();
    assert!(r.sys.unit("mako.service").unwrap().masked);
    let entry = record::load(&r.env.setup_toml()).unwrap().items.remove("service-notifd").unwrap();
    let Change::Notifd { replaced, .. } = entry.change else { panic!() };
    assert!(replaced[0].masked);

    r.undo("service-notifd").unwrap();
    let mako = r.sys.unit("mako.service").unwrap();
    assert!(!mako.masked && mako.active);
}

/// Started by `exec-once`, a competitor cannot be stopped for good from
/// here; the item says what to remove instead of half-doing it.
#[test]
fn a_competitor_started_by_the_users_config_is_named_and_left_alone() {
    let r = rig();
    r.sys.set_process("swaync", true);
    let state = r.state("service-notifd");
    let State::Unavailable { why } = &state else { panic!("{state:?}") };
    assert!(why.contains("swaync") && why.contains("autostart"), "{why}");
}

// --- idle ------------------------------------------------------------------

#[test]
fn the_previous_lock_command_is_named_and_restored() {
    let r = rig();
    let mut settings = hyprforge_ecosystem::idle::Settings::default();
    settings.general.lock_cmd = "hyprlock".into();
    hyprforge_ecosystem::storage::save(&r.env.idle_toml(), &settings).unwrap();
    let State::Todo { what } = r.state("idle-lock") else { panic!() };
    assert!(what.contains("hyprlock"), "{what}");

    r.apply("idle-lock").unwrap();
    let conf = r.read(&r.env.idle_conf());
    assert!(conf.contains("lock_cmd = pidof hyprforge-lock || hyprforge-lock"), "{conf}");
    assert!(r.read(&r.env.hypridle_conf()).contains(&r.env.idle_conf().display().to_string()));

    r.undo("idle-lock").unwrap();
    let back: hyprforge_ecosystem::idle::Settings =
        hyprforge_ecosystem::storage::load(&r.env.idle_toml()).unwrap();
    assert_eq!(back.general.lock_cmd, "hyprlock");
}

/// The lock command in the user's own hypridle.conf is what runs today,
/// and the item says what it would replace.
#[test]
fn a_lock_command_in_the_users_hypridle_conf_is_named() {
    let r = rig();
    std::fs::write(r.env.hypridle_conf(), "general {\n    lock_cmd = swaylock\n}\n").unwrap();
    let State::Todo { what } = r.state("idle-lock") else { panic!() };
    assert!(what.contains("swaylock"), "{what}");
}

/// hypridle has no IPC: restarted if running, never started if not.
#[test]
fn hypridle_is_restarted_only_if_it_was_running() {
    let r = rig();
    r.apply("idle-lock").unwrap();
    assert!(!r.sys.calls().contains(&"restart-idle".to_string()));

    let r = rig();
    r.sys.set_process("hypridle", true);
    r.apply("idle-lock").unwrap();
    assert!(r.sys.calls().contains(&"restart-idle".to_string()));
}

// --- the rest -----------------------------------------------------------------

#[test]
fn the_previous_folder_handler_is_named_and_restored() {
    let r = rig();
    let State::Todo { what } = r.state("default-folders") else { panic!() };
    assert!(what.contains("instead of Nemo"), "{what}");
    r.apply("default-folders").unwrap();
    assert!(r.read(&r.env.mimeapps_list()).contains("inode/directory=hyprforge-files.desktop"));
    r.undo("default-folders").unwrap();
    assert_eq!(
        r.read(&r.env.mimeapps_list()),
        "[Default Applications]\ninode/directory=nemo.desktop\n"
    );
}

#[test]
fn an_app_with_no_desktop_entry_is_unavailable() {
    let r = rig();
    std::fs::remove_file(r.env.data_home.join("applications/hyprforge-media.desktop")).unwrap();
    assert!(matches!(r.state("default-images"), State::Unavailable { .. }));
}

/// The user's other portal choices, and their `default=`, survive.
#[test]
fn portals_conf_keeps_the_users_lines_and_their_default() {
    let r = rig();
    let original = "[preferred]\ndefault=gnome;gtk\norg.freedesktop.impl.portal.Screenshot=hyprland\n";
    std::fs::create_dir_all(r.env.portals_conf().parent().unwrap()).unwrap();
    std::fs::write(r.env.portals_conf(), original).unwrap();
    r.apply("portal-dialog").unwrap();
    let text = r.read(&r.env.portals_conf());
    assert!(text.contains("default=gnome;gtk") && text.contains("Screenshot=hyprland"), "{text}");
    assert!(!text.contains("default=hyprland;gtk"), "{text}");
    assert!(r.sys.calls().contains(&"try-restart xdg-desktop-portal.service".to_string()));
    r.undo("portal-dialog").unwrap();
    assert_eq!(r.read(&r.env.portals_conf()), original);
}

#[test]
fn a_previous_file_manager1_service_is_restored() {
    let r = rig();
    let nemo = "[D-BUS Service]\nName=org.freedesktop.FileManager1\nExec=/usr/bin/nemo-desktop\n";
    std::fs::create_dir_all(r.env.file_manager1_service().parent().unwrap()).unwrap();
    std::fs::write(r.env.file_manager1_service(), nemo).unwrap();
    assert!(r.state("show-in-folder").is_todo());
    r.apply("show-in-folder").unwrap();
    assert!(r.read(&r.env.file_manager1_service()).contains("Exec=/usr/bin/hyprforge-files --dbus-service"));
    assert!(r.sys.calls().contains(&"reload-session-bus".to_string()));
    r.undo("show-in-folder").unwrap();
    assert_eq!(r.read(&r.env.file_manager1_service()), nemo);
}

#[test]
fn the_gtk_portal_variable_says_it_waits_for_the_next_login() {
    let r = rig();
    let State::Todo { what } = r.state("gtk-portal") else { panic!() };
    assert!(what.contains("next login"), "{what}");
    r.apply("gtk-portal").unwrap();
    assert!(r.read(&r.env.session_lua()).contains("hl.env([[GTK_USE_PORTAL]], [[1]])"));
}

#[test]
fn notif_blur_writes_layer_rules_hyprland_reads() {
    let r = rig();
    r.apply("notif-blur").unwrap();
    let lua = r.read(&r.env.window_rules_lua());
    assert!(lua.contains("match = { namespace = [[^notif$]] }, blur = true"), "{lua}");
    assert!(lua.contains("[[^notif-center$]]"), "{lua}");
}

// --- lock restore ------------------------------------------------------------

#[test]
fn lock_restore_writes_the_option_hyprland_reads_into_system_lua() {
    let r = rig();
    r.apply("lock-restore").unwrap();
    let lua = r.read(&r.env.system_lua());
    assert!(lua.contains("allow_session_lock_restore = true"), "{lua}");
    assert!(r.read(&r.env.system_toml()).contains("\"misc:allow_session_lock_restore\" = true"));
}

/// Everything else the System page owns survives an apply and an undo,
/// and a value the user had before comes back rather than being cleared.
#[test]
fn lock_restore_undo_puts_back_what_system_toml_held_and_keeps_the_rest() {
    let r = rig();
    std::fs::create_dir_all(r.env.hyprforge_dir()).unwrap();
    std::fs::write(
        r.env.system_toml(),
        "\"misc:vrr\" = 1\n\"misc:allow_session_lock_restore\" = false\n",
    )
    .unwrap();
    r.apply("lock-restore").unwrap();
    r.undo("lock-restore").unwrap();
    let toml = r.read(&r.env.system_toml());
    assert!(toml.contains("\"misc:allow_session_lock_restore\" = false"), "{toml}");
    assert!(toml.contains("\"misc:vrr\" = 1"), "{toml}");
}

/// Turned off by hand on the System page after setup turned it on: that
/// is the user's choice now, and undo does not override it.
#[test]
fn lock_restore_undo_leaves_a_value_the_user_changed_since() {
    let r = rig();
    r.apply("lock-restore").unwrap();
    std::fs::write(r.env.system_toml(), "\"misc:allow_session_lock_restore\" = false\n").unwrap();
    let files = r.snapshot();
    let said = r.undo("lock-restore").unwrap();
    assert!(said.contains("changed since"), "{said}");
    let mut after = r.snapshot();
    after.remove(&r.env.setup_toml());
    let mut before = files;
    before.remove(&r.env.setup_toml());
    assert_eq!(after, before, "nothing but the record moved");
}

/// Hyprland refusing the generated file leaves `system.toml` as it was,
/// so the page and the running compositor still agree.
#[test]
fn a_rejected_system_lua_leaves_system_toml_as_it_was() {
    let r = rig();
    std::fs::create_dir_all(r.env.hyprforge_dir()).unwrap();
    std::fs::write(r.env.system_toml(), "\"misc:vrr\" = 1\n").unwrap();
    r.sys.reject_lua(Some("system.lua:1: bad"));
    assert!(r.apply("lock-restore").is_err());
    assert_eq!(r.read(&r.env.system_toml()), "\"misc:vrr\" = 1\n");
    assert!(is_todo(&r.state("lock-restore")));
}

#[test]
fn a_hyprland_conf_only_setup_cannot_be_wired() {
    let r = rig();
    std::fs::remove_file(r.env.hyprland_lua()).unwrap();
    std::fs::write(r.env.hypr_dir().join("hyprland.conf"), "monitor=,preferred,auto,1\n").unwrap();
    assert!(matches!(r.state("wiring"), State::Unavailable { .. }));
}

/// Undo of the wiring takes out its own lines and nothing of the user's.
#[test]
fn wiring_undo_removes_only_the_lines_setup_added() {
    let r = rig();
    let mut user = r.read(&r.env.hyprland_lua());
    user.push_str("require(\"hyprforge/keybinds\")\n");
    std::fs::write(r.env.hyprland_lua(), &user).unwrap();
    r.apply("wiring").unwrap();
    r.undo("wiring").unwrap();
    let after = r.read(&r.env.hyprland_lua());
    assert!(after.contains("require(\"hyprforge/keybinds\")"), "the user's own line stays: {after}");
    assert!(!after.contains("require(\"hyprforge/session\")"), "{after}");
}

// --- the record ------------------------------------------------------------

/// Unparseable is not empty: undo refuses rather than undoing nothing,
/// and apply refuses rather than overwrite the only record.
#[test]
fn an_unparseable_setup_toml_refuses_undo_and_apply() {
    let r = rig();
    r.apply("bind-files").unwrap();
    std::fs::write(r.env.setup_toml(), "items = = broken").unwrap();
    let files = r.snapshot();
    assert!(undo(&r.env, &r.sys, &[]).is_err());
    assert!(apply(&r.env, &r.sys, &["bind-lock"]).is_err());
    assert_eq!(r.snapshot(), files, "nothing was touched");
}

#[test]
fn a_check_that_cannot_read_its_file_is_unknown_never_done() {
    let r = rig();
    std::fs::create_dir_all(r.env.hyprforge_dir()).unwrap();
    for path in [
        r.env.shortcuts_toml(),
        r.env.idle_toml(),
        r.env.window_rules_toml(),
        r.env.session_toml(),
        r.env.system_toml(),
    ] {
        std::fs::write(path, "= not toml").unwrap();
    }
    for id in ["bind-files", "idle-lock", "lock-restore", "notif-blur", "gtk-portal"] {
        assert!(matches!(r.state(id), State::Unknown { .. }), "{id}: {:?}", r.state(id));
    }
}

#[test]
fn an_item_whose_program_isnt_installed_is_unavailable_not_todo() {
    let r = rig();
    for (binary, id) in [
        ("hyprforge-clipmenu", "bind-clipboard"),
        ("hypridle", "idle-lock"),
        ("hyprforge-lock", "lock-restore"),
        ("notifd", "notif-blur"),
        ("hyprforge-files-portal", "portal-dialog"),
        ("hyprforge-files", "show-in-folder"),
    ] {
        r.sys.remove_binary(binary);
        let state = r.state(id);
        let State::Unavailable { why } = &state else { panic!("{id}: {state:?}") };
        assert!(why.contains(binary), "{why}");
    }
}

/// Previous Versions: [`rig`] already has snapper's `home` config.
fn snapper_rig() -> Rig {
    rig()
}

#[test]
fn previous_versions_adds_this_user_beside_the_others_and_turns_on_the_acl() {
    let r = snapper_rig();
    assert!(matches!(r.state("previous-versions"), State::Todo { .. }));
    r.apply("previous-versions").unwrap();
    assert_eq!(r.sys.calls().last().unwrap(), "snapper -c home set-config ALLOW_USERS=sam alex SYNC_ACL=yes");
    assert_eq!(r.state("previous-versions"), State::Done, "the check reads what snapper wrote");
}

/// Undo takes the user off while syncing is still on — so snapper takes
/// the ACL back off `.snapshots` — then puts syncing back.
#[test]
fn previous_versions_undo_removes_the_user_before_the_acl_syncing() {
    let r = snapper_rig();
    r.apply("previous-versions").unwrap();
    r.undo("previous-versions").unwrap();
    let calls = r.sys.calls();
    let n = calls.len();
    assert_eq!(calls[n - 2], "snapper -c home set-config ALLOW_USERS=sam");
    assert_eq!(calls[n - 1], "snapper -c home set-config SYNC_ACL=no");
    assert!(matches!(r.state("previous-versions"), State::Todo { .. }));
}

/// With no local copy, the vendor's stack is copied to /etc/pam.d with
/// the reader first; undo removes the copy, so the vendor's applies
/// again — never an edit of the vendor file.
#[test]
fn fingerprint_writes_a_local_stack_and_undo_removes_it() {
    let r = rig();
    let local = r.env.pam_dir.join("polkit-1");
    assert!(is_todo(&r.state("fingerprint-polkit")));
    r.apply("fingerprint-polkit").unwrap();
    let written = r.read(&local);
    assert!(written.contains("auth       sufficient   pam_fprintd.so timeout=10"), "{written}");
    assert!(written.contains("auth       include      system-auth"));
    assert_eq!(r.read(&r.env.vendor_pam_dir.join("polkit-1")), POLKIT_STACK, "the vendor's file is never touched");
    r.undo("fingerprint-polkit").unwrap();
    assert!(!local.exists(), "no local copy before, none after");
    assert!(is_todo(&r.state("fingerprint-polkit")));
}

/// A local stack someone wrote themselves is the one changed — and the
/// one put back, word for word.
#[test]
fn fingerprint_keeps_a_local_stack_and_undo_restores_it_exactly() {
    let r = rig();
    let local = r.env.pam_dir.join("polkit-1");
    let mine = "#%PAM-1.0\n# mine\nauth       required     pam_env.so\nauth       include      system-auth\naccount    include      system-auth\n";
    std::fs::create_dir_all(&r.env.pam_dir).unwrap();
    std::fs::write(&local, mine).unwrap();
    r.apply("fingerprint-polkit").unwrap();
    assert!(r.read(&local).contains("# mine"));
    r.undo("fingerprint-polkit").unwrap();
    assert_eq!(r.read(&local), mine);
}

#[test]
fn fingerprint_is_offered_only_when_a_finger_could_answer() {
    let r = rig();
    r.sys.with_fingerprint(Some(false));
    assert!(matches!(r.state("fingerprint-polkit"), State::Unavailable { .. }), "nothing enrolled");
    r.sys.with_fingerprint(None);
    assert!(matches!(r.state("fingerprint-polkit"), State::Unknown { .. }), "fprintd not answering is not \"no\"");
    let r = rig();
    std::fs::remove_file(r.env.pam_modules.join("pam_fprintd.so")).unwrap();
    assert!(matches!(r.state("fingerprint-polkit"), State::Unavailable { .. }), "no PAM module");
}

/// A dismissed password prompt changes nothing and records nothing.
#[test]
fn fingerprint_refused_leaves_nothing_behind() {
    let r = rig();
    r.sys.refusing_root();
    let before = r.snapshot();
    assert!(r.apply("fingerprint-polkit").is_err());
    assert_eq!(r.snapshot(), before);
    assert!(is_todo(&r.state("fingerprint-polkit")));
}

#[test]
fn previous_versions_is_unavailable_without_a_home_config() {
    let r = rig();
    std::fs::remove_file(r.env.snapper_configs.join("home")).unwrap();
    assert!(matches!(r.state("previous-versions"), State::Unavailable { .. }));
}
