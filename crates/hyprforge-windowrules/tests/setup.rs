use hyprforge_windowrules::setup::{
    apply, create_lua_config, detect, discover, install, HyprConfig, SetupPlan, REQUIRE_LINE,
};

const NO_REQUIRES: &str = "hl.config({\n    debug = { disable_logs = false },\n})\n";

const WITH_REQUIRES: &str = "\
hl.config({ debug = { disable_logs = false } })

require(\"monitors\")

hl.monitor({ output = \"\", mode = \"preferred\" })
";

const ALREADY_INSTALLED: &str = "\
require(\"hyprforge/window-rules\")
require(\"monitors\")
";

/// Window rules are `BeforeUserRequires`, and that has to hold even when
/// the user has no `require()` of their own — which is the common shape.
///
/// Hyprland applies the *last* matching rule (verified against 0.56.1), so
/// appending here would silently make Hyprforge's generated rules override
/// the user's own hand-written ones. The line belongs above their content.
#[test]
fn detects_no_requires_and_still_inserts_above_the_users_content() {
    let plan = detect(NO_REQUIRES);
    assert_eq!(plan, SetupPlan::NeedsInsert { insert_before_line: 1 });

    let out = apply(NO_REQUIRES, &plan);
    assert!(
        out.find("hyprforge/window-rules").unwrap() < out.find("hl.config").unwrap(),
        "the require must precede the user's own config: {out}"
    );
}

#[test]
fn detects_existing_requires_inserts_before_first() {
    let plan = detect(WITH_REQUIRES);
    // require("monitors") is on line 3 (1-indexed).
    assert_eq!(
        plan,
        SetupPlan::NeedsInsert {
            insert_before_line: 3
        }
    );
}

#[test]
fn detects_already_installed() {
    assert_eq!(detect(ALREADY_INSTALLED), SetupPlan::AlreadyPresent);
}

#[test]
fn apply_inserts_before_first_require_so_it_evaluates_earliest() {
    let plan = detect(WITH_REQUIRES);
    let result = apply(WITH_REQUIRES, &plan);
    let lines: Vec<&str> = result.lines().collect();

    let hyprforge_idx = lines
        .iter()
        .position(|l| l.contains("hyprforge/window-rules"))
        .unwrap();
    let monitors_idx = lines
        .iter()
        .position(|l| l.contains("require(\"monitors\")"))
        .unwrap();

    assert!(
        hyprforge_idx < monitors_idx,
        "hyprforge require must precede the user's own requires so the \
         user's named rules (evaluated later) override Hyprforge's"
    );
}

#[test]
fn apply_on_already_present_is_a_noop() {
    let result = apply(ALREADY_INSTALLED, &SetupPlan::AlreadyPresent);
    assert_eq!(result, ALREADY_INSTALLED);
}

#[test]
fn install_backs_up_and_writes_atomically() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("hyprland.lua");
    std::fs::write(&path, WITH_REQUIRES).unwrap();

    let plan = install(&path).unwrap();
    assert!(matches!(plan, SetupPlan::NeedsInsert { .. }));

    let backup = path.with_extension("lua.hyprforge.bak");
    assert_eq!(std::fs::read_to_string(&backup).unwrap(), WITH_REQUIRES);

    let new_contents = std::fs::read_to_string(&path).unwrap();
    assert!(new_contents.contains("hyprforge/window-rules"));
    assert_eq!(detect(&new_contents), SetupPlan::AlreadyPresent);
}

#[test]
fn install_is_idempotent() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("hyprland.lua");
    std::fs::write(&path, ALREADY_INSTALLED).unwrap();

    let plan = install(&path).unwrap();
    assert_eq!(plan, SetupPlan::AlreadyPresent);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), ALREADY_INSTALLED);
    // No backup should have been made — nothing changed.
    assert!(!path.with_extension("lua.hyprforge.bak").exists());
}

// --- config discovery -------------------------------------------------
//
// These cover the dead-end this flow used to have: a user with no
// hyprland.lua (or only a hyprland.conf) previously reached the confirm
// dialog and then a hard read error from install(), with no way forward.

#[test]
fn discovers_lua_config() {
    let dir = tempfile::tempdir().unwrap();
    let lua = dir.path().join("hyprland.lua");
    std::fs::write(&lua, NO_REQUIRES).unwrap();
    assert_eq!(discover(dir.path()), HyprConfig::Lua(lua));
}

#[test]
fn discovers_conf_only_when_no_lua_exists() {
    let dir = tempfile::tempdir().unwrap();
    let conf = dir.path().join("hyprland.conf");
    std::fs::write(&conf, "monitor=,preferred,auto,1\n").unwrap();
    assert_eq!(discover(dir.path()), HyprConfig::ConfOnly(conf));
}

#[test]
fn lua_wins_when_both_configs_exist() {
    let dir = tempfile::tempdir().unwrap();
    let lua = dir.path().join("hyprland.lua");
    std::fs::write(&lua, NO_REQUIRES).unwrap();
    std::fs::write(dir.path().join("hyprland.conf"), "monitor=,preferred,auto,1\n").unwrap();
    assert_eq!(discover(dir.path()), HyprConfig::Lua(lua));
}

#[test]
fn discovers_missing_on_empty_dir() {
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(discover(dir.path()), HyprConfig::Missing);
}

#[test]
fn discover_ignores_a_directory_named_like_the_config() {
    // A `hyprland.lua/` directory must not be reported as a usable config.
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("hyprland.lua")).unwrap();
    assert_eq!(discover(dir.path()), HyprConfig::Missing);
}

#[test]
fn create_lua_config_writes_a_config_that_needs_no_further_setup() {
    let dir = tempfile::tempdir().unwrap();
    let lua = dir.path().join("hyprland.lua");
    create_lua_config(&lua).unwrap();

    let contents = std::fs::read_to_string(&lua).unwrap();
    assert!(contents.contains(REQUIRE_LINE));
    // The whole point: the created file is already fully set up, so the GUI
    // can skip straight past the insertion step.
    assert_eq!(detect(&contents), SetupPlan::AlreadyPresent);
    assert_eq!(discover(dir.path()), HyprConfig::Lua(lua));
}

#[test]
fn create_lua_config_creates_missing_parent_directories() {
    let dir = tempfile::tempdir().unwrap();
    let lua = dir.path().join("hypr").join("hyprland.lua");
    create_lua_config(&lua).unwrap();
    assert!(lua.is_file());
}

#[test]
fn create_lua_config_refuses_to_clobber_an_existing_config() {
    let dir = tempfile::tempdir().unwrap();
    let lua = dir.path().join("hyprland.lua");
    std::fs::write(&lua, NO_REQUIRES).unwrap();

    assert!(create_lua_config(&lua).is_err());
    // The user's config is untouched — editing an existing file is
    // install()'s job, which backs up first.
    assert_eq!(std::fs::read_to_string(&lua).unwrap(), NO_REQUIRES);
}
