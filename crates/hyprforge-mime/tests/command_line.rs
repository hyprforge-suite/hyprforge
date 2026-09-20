//! The two commands, run as commands.
//!
//! Everything else in this crate is tested through the library. These
//! run the binaries, because the things that matter about them are
//! things a library test cannot see: the exact bytes on stdout (another
//! program parses them), the exit codes (a shell script branches on
//! them), and that `mimeopen` launches what it says it launched.
//!
//! Every run is pointed at a fixture database with `--database`, and at
//! a temporary `XDG_CONFIG_HOME`, so nothing here reads the machine's
//! own database or writes the person's own `mimeapps.list` — the rule
//! that keeps the Settings app's tests honest applies to a command that
//! can change a default just as much.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const MIMETYPE: &str = env!("CARGO_BIN_EXE_hyprforge-mimetype");
const MIMEOPEN: &str = env!("CARGO_BIN_EXE_hyprforge-mimeopen");

/// A database with one filename rule, one strong content rule, and two
/// applications — one of which is what a launch should actually run.
fn fixture() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    let mime = data.join("mime");
    std::fs::create_dir_all(mime.join("model")).unwrap();
    std::fs::create_dir_all(data.join("applications")).unwrap();

    std::fs::write(mime.join("globs2"), "50:model/stl:*.stl\n50:text/plain:*.txt\n").unwrap();
    std::fs::write(
        mime.join("model").join("stl.xml"),
        "<mime-type type=\"model/stl\">\n  <comment>STL 3D model</comment>\n</mime-type>\n",
    )
    .unwrap();
    let mut magic = b"MIME-Magic\0\n[90:image/png]\n>0=".to_vec();
    magic.extend(4u16.to_be_bytes());
    magic.extend(b"\x89PNG\n");
    std::fs::write(mime.join("magic"), magic).unwrap();

    // `Exec=true` — a real program that takes any arguments and exits 0,
    // so a launch can be proved to have happened without a window
    // appearing on the machine running the tests.
    std::fs::write(
        data.join("applications").join("viewer.desktop"),
        "[Desktop Entry]\nType=Application\nName=Model Viewer\nExec=true %f\n",
    )
    .unwrap();
    std::fs::write(
        data.join("applications").join("mimeinfo.cache"),
        "[MIME Cache]\nmodel/stl=viewer.desktop;\n",
    )
    .unwrap();
    dir
}

fn data_dir(fixture: &tempfile::TempDir) -> PathBuf {
    fixture.path().join("data")
}

/// Runs one of the commands against the fixture, with its own config
/// home so a written default lands in the temporary directory.
fn run(program: &str, fixture: &tempfile::TempDir, args: &[&str]) -> (String, String, i32) {
    let config = fixture.path().join("config");
    std::fs::create_dir_all(&config).unwrap();
    let output = Command::new(program)
        .arg("--database")
        .arg(data_dir(fixture))
        .args(args)
        .env("XDG_CONFIG_HOME", &config)
        .env("XDG_DATA_HOME", fixture.path().join("empty-data"))
        .env("XDG_DATA_DIRS", fixture.path().join("empty-dirs"))
        .output()
        .expect("the binary is built by cargo before this test runs");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code().unwrap_or(-1),
    )
}

fn write(dir: &Path, name: &str, contents: &[u8]) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, contents).unwrap();
    path
}

/// The output another program parses: `name: type`, aligned into a
/// column, on stdout, exit 0.
#[test]
fn mimetype_prints_name_then_type_and_exits_cleanly() {
    let fixture = fixture();
    let one = write(fixture.path(), "part.stl", b"solid\n");
    let two = write(fixture.path(), "a.txt", b"words\n");
    let (stdout, stderr, code) =
        run(MIMETYPE, &fixture, &[one.to_str().unwrap(), two.to_str().unwrap()]);
    assert_eq!(code, 0, "{stderr}");
    let lines: Vec<&str> = stdout.lines().collect();
    assert!(lines[0].starts_with(&format!("{}:", one.display())), "{stdout}");
    assert!(lines[0].ends_with("model/stl"), "{stdout}");
    assert!(lines[1].ends_with("text/plain"), "{stdout}");
    // Piped rather than on a terminal, so names are raw and the types
    // still line up under each other.
    let column = |line: &str| line.rfind(' ').map(|i| i + 1).unwrap_or(0);
    assert_eq!(column(lines[0]), column(lines[1]), "the column is aligned: {stdout}");
}

#[test]
fn mimetype_brief_and_describe_are_the_two_short_forms() {
    let fixture = fixture();
    let path = write(fixture.path(), "part.stl", b"solid\n");
    let file = path.to_str().unwrap();
    assert_eq!(run(MIMETYPE, &fixture, &["-b", file]).0, "model/stl\n");
    assert_eq!(run(MIMETYPE, &fixture, &["-bd", file]).0, "STL 3D model\n");
    assert_eq!(
        run(MIMETYPE, &fixture, &["--noalign", file]).0,
        format!("{file}: model/stl\n"),
        "the form the reference's own test suite asserts"
    );
}

/// Contents win over the name when the rule is a strong one — the same
/// claim the library makes, proved through the command.
#[test]
fn mimetype_reads_contents_when_the_name_lies() {
    let fixture = fixture();
    let path = write(fixture.path(), "lying.txt", b"\x89PNG\r\n\x1a\n");
    let (stdout, _, code) = run(MIMETYPE, &fixture, &["-b", path.to_str().unwrap()]);
    assert_eq!(code, 0);
    assert_eq!(stdout, "image/png\n");
}

/// A database with nothing in it is a failure, not an empty answer —
/// the reference's own test asserts a non-zero exit for exactly this.
#[test]
fn mimetype_with_an_empty_database_fails() {
    let fixture = fixture();
    let empty = tempfile::tempdir().unwrap();
    let output = Command::new(MIMETYPE)
        .arg("--database")
        .arg(empty.path())
        .arg(fixture.path().join("part.stl"))
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("database"));
}

#[test]
fn mimetype_reports_an_unknown_option_with_the_references_exit_code() {
    let fixture = fixture();
    let (_, stderr, code) = run(MIMETYPE, &fixture, &["--nonsense", "x"]);
    assert_eq!(code, 1);
    assert!(stderr.contains("--nonsense"), "{stderr}");

    let (_, stderr, code) = run(MIMETYPE, &fixture, &[]);
    assert_eq!(code, 4, "no files at all");
    assert!(stderr.contains("usage:"), "{stderr}");
}

/// `mimeopen -n` is the form `xdg-open` itself falls back to: no
/// questions, launch what is registered.
#[test]
fn mimeopen_launches_the_registered_application() {
    let fixture = fixture();
    let path = write(fixture.path(), "part.stl", b"solid\n");
    let (stdout, stderr, code) = run(MIMEOPEN, &fixture, &["-n", path.to_str().unwrap()]);
    assert_eq!(code, 0, "stderr: {stderr}");
    assert!(stdout.contains("Model Viewer"), "says what it opened it with: {stdout}");
    assert!(stdout.contains("(model/stl)"), "and what it decided the file was: {stdout}");
}

/// Nothing installed for the type is its own exit code, so a caller can
/// tell it apart from "that did not work".
#[test]
fn mimeopen_says_when_nothing_opens_that_type() {
    let fixture = fixture();
    let path = write(fixture.path(), "notes.txt", b"words\n");
    let (_, stderr, code) = run(MIMEOPEN, &fixture, &["-n", path.to_str().unwrap()]);
    assert_eq!(code, 6, "{stderr}");
    assert!(stderr.contains("No applications found"), "{stderr}");
}

/// Asking for a default writes one — into the temporary config home,
/// never the machine's own.
#[test]
fn mimeopen_remembers_a_choice_in_the_config_home_it_was_given() {
    let fixture = fixture();
    let path = write(fixture.path(), "part.stl", b"solid\n");
    // With one candidate and no default set, the reference asks nothing
    // and remembers it.
    let (stdout, stderr, code) = run(MIMEOPEN, &fixture, &[path.to_str().unwrap()]);
    assert_eq!(code, 0, "{stderr}");
    assert!(stdout.contains("Model Viewer"), "{stdout}");

    let written = fixture.path().join("config").join("mimeapps.list");
    let text = std::fs::read_to_string(&written).expect("a default was recorded");
    assert!(text.contains("[Default Applications]"), "{text}");
    assert!(text.contains("model/stl=viewer.desktop"), "{text}");
}

/// "Other…": none of the applications offered, run this instead. The
/// command becomes a desktop entry in the user's own applications
/// directory, and that entry is what gets launched and remembered.
#[test]
fn mimeopen_can_be_told_a_command_of_its_own() {
    let fixture = fixture();
    let path = write(fixture.path(), "part.stl", b"solid\n");
    let config = fixture.path().join("config");
    let data_home = fixture.path().join("home-data");
    std::fs::create_dir_all(&config).unwrap();

    // One application is offered, so "Other..." is item 2. The command
    // is `true`, which exists everywhere and does nothing.
    let mut child = Command::new(MIMEOPEN)
        .arg("--database")
        .arg(data_dir(&fixture))
        .args(["-d", path.to_str().unwrap()])
        .env("XDG_CONFIG_HOME", &config)
        .env("XDG_DATA_HOME", &data_home)
        .env("XDG_DATA_DIRS", fixture.path().join("empty-dirs"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(b"2\ntrue\n").unwrap();
    let output = child.wait_with_output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    assert!(stdout.contains("Other..."), "the option is offered: {stdout}");
    assert!(stdout.contains("Opening"), "and something was opened: {stdout}");

    let entry = data_home.join("applications").join("true-usercreated-1.desktop");
    let text = std::fs::read_to_string(&entry).expect("an entry was written for the command");
    assert!(text.contains("Exec=true %f"), "{text}");
    assert!(text.contains("NoDisplay=true"), "a one-off choice is not a menu entry");

    let recorded = std::fs::read_to_string(config.join("mimeapps.list")).unwrap();
    assert!(
        recorded.contains("model/stl=true-usercreated-1.desktop"),
        "and it became the default: {recorded}"
    );
}

/// A prompt answered with anything but a number in range is cancelled,
/// with the reference's own exit code and nothing launched.
#[test]
fn mimeopen_treats_a_stray_answer_as_cancelled() {
    let fixture = fixture();
    let path = write(fixture.path(), "part.stl", b"solid\n");
    let config = fixture.path().join("config");
    std::fs::create_dir_all(&config).unwrap();
    let mut child = Command::new(MIMEOPEN)
        .arg("--database")
        .arg(data_dir(&fixture))
        .args(["-a", path.to_str().unwrap()])
        .env("XDG_CONFIG_HOME", &config)
        .env("XDG_DATA_HOME", fixture.path().join("home-data"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(b"nope\n").unwrap();
    let output = child.wait_with_output().unwrap();
    assert_eq!(output.status.code(), Some(8));
    assert!(String::from_utf8_lossy(&output.stderr).contains("Cancelled"));
    assert!(!config.join("mimeapps.list").exists(), "nothing was remembered");
}

#[test]
fn both_commands_describe_themselves() {
    let fixture = fixture();
    for program in [MIMETYPE, MIMEOPEN] {
        let (stdout, _, code) = run(program, &fixture, &["--help"]);
        assert_eq!(code, 0);
        assert!(stdout.starts_with("usage:"), "{stdout}");
        let (stdout, _, code) = run(program, &fixture, &["--version"]);
        assert_eq!(code, 0);
        assert!(stdout.contains("hyprforge-mime"), "{stdout}");
    }
}
