use hyprforge_windowrules::setup::{apply, detect, install, SetupPlan};

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

#[test]
fn detects_no_requires_appends_at_end() {
    let plan = detect(NO_REQUIRES);
    let expected_line = NO_REQUIRES.lines().count() + 1;
    assert_eq!(
        plan,
        SetupPlan::NeedsInsert {
            insert_before_line: expected_line
        }
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
