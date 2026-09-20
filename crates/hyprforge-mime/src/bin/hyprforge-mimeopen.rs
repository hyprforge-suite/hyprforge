//! `mimeopen` — open a file with the application that should open it,
//! asking when there is a choice to make.
//!
//! The companion to `hyprforge-mimetype`, and a drop-in for the
//! `mimeopen` command from `perl-file-mimeinfo`: the same options, the
//! same prompts, the same exit codes. `xdg-open` falls back to
//! `mimeopen -L -n` when its own lookup finds nothing, so this is the
//! second place a desktop can be rescued from opening a 3D model in a
//! web browser.
//!
//! ```text
//! mimeopen part.3mf          opens it, asking if nothing is set
//! mimeopen -n part.3mf       never asks; does nothing if nothing is set
//! mimeopen -d part.3mf       asks, and remembers the answer
//! mimeopen -a part.3mf       asks, and does not remember
//! ```
//!
//! # What it will not do
//!
//! Parse `Exec=`. The chosen application is launched through `gio
//! launch`, which owns the field codes, `TryExec` and terminal
//! handling; see `hyprforge_files::launch` for the same decision made
//! for the file manager. Without `gio` this says so rather than
//! guessing at a command line.

use hyprforge_mime::{App, MimeDb};
use std::borrow::Cow;
use std::io::{BufRead, IsTerminal, Write};
use std::path::{Path, PathBuf};

mod exit {
    pub const UNKNOWN_OPTION: i32 = 1;
    pub const MISSING_ARGUMENT: i32 = 2;
    pub const NO_FILES: i32 = 4;
    /// Nothing could say what the file is.
    pub const UNKNOWN_TYPE: i32 = 5;
    /// Nothing installed opens that type.
    pub const NO_APPLICATION: i32 = 6;
    /// The person said no.
    pub const CANCELLED: i32 = 8;
}

#[derive(Debug, Default)]
struct Args {
    files: Vec<String>,
    /// Ask which application, without remembering the answer.
    ask: bool,
    /// Ask, and make the answer the default for that type.
    ask_default: bool,
    /// Never ask: use the default, or the first application there is.
    no_ask: bool,
    /// Decide the type from the contents alone.
    magic_only: bool,
    /// Follow a symlink to what it points at.
    dereference: bool,
    database: Option<String>,
    debug: bool,
    help: bool,
    version: bool,
}

const OPTIONS: &[(&str, Option<char>, bool)] = &[
    ("help", Some('h'), false),
    ("usage", Some('u'), false),
    ("version", Some('v'), false),
    ("dereference", Some('L'), false),
    ("debug", Some('D'), false),
    ("database", None, true),
    ("magic-only", Some('M'), false),
    ("ask", Some('a'), false),
    ("ask-default", Some('d'), false),
    ("no-ask", Some('n'), false),
];

fn main() {
    let args = match parse(std::env::args().skip(1).collect()) {
        Ok(args) => args,
        Err((message, code)) => {
            eprintln!("{message}");
            if code != exit::NO_FILES {
                eprintln!("Try 'mimeopen --help' for more information.");
            }
            std::process::exit(code);
        }
    };
    if args.help {
        print!("{}", usage());
        return;
    }
    if args.version {
        println!("mimeopen (hyprforge-mime) {}", env!("CARGO_PKG_VERSION"));
        return;
    }
    if args.files.is_empty() {
        eprintln!("usage: mimeopen [options] files");
        std::process::exit(exit::NO_FILES);
    }

    let dirs: Vec<PathBuf> = match &args.database {
        Some(list) => list.split(':').filter(|d| !d.is_empty()).map(PathBuf::from).collect(),
        None => hyprforge_mime::data_dirs(),
    };
    if args.debug {
        let shown: Vec<String> = dirs.iter().map(|d| d.display().to_string()).collect();
        println!("> Data dirs are: {}", shown.join(", "));
    }
    let db = MimeDb::load_from(&dirs, &hyprforge_mime::mimeapps_paths());

    // One type for the whole set, from the first file — the same as the
    // reference, and the only thing that makes sense: they are all
    // being handed to one application.
    let first = PathBuf::from(&args.files[0]);
    let mime = match args.magic_only {
        true => db.lookup().magic.of_file(&first).map(|m| m.mime),
        false => Some(db.lookup().of_file(&first, args.dereference).mime),
    };
    let Some(mime) = mime.filter(|mime| !mime.is_empty()) else {
        eprintln!("Could not determine mimetype for file: {}", quoted(&args.files[0]));
        std::process::exit(exit::UNKNOWN_TYPE);
    };

    // The type's own applications, then those of the types it is a kind
    // of — an archive manager can open a 3MF.
    let candidates = db.apps_for_including_parents(&mime);
    let default = db.default_for(&mime).filter(|app| app.installed);

    let chosen: Option<Cow<'_, App>> = if args.no_ask {
        default.or_else(|| candidates.first().copied()).map(Cow::Borrowed)
    } else if args.ask {
        choose(&mime, false, &candidates)
    } else if args.ask_default {
        choose(&mime, true, &candidates)
    } else if default.is_some() {
        default.map(Cow::Borrowed)
    } else {
        // Nothing is set: one candidate needs no question, several do.
        match candidates.as_slice() {
            [only] => Some(Cow::Borrowed(*only)),
            _ => choose(&mime, true, &candidates),
        }
    };

    let Some(chosen) = chosen else {
        eprintln!("No applications found for mimetype: {mime}");
        std::process::exit(exit::NO_APPLICATION);
    };

    // Remember it, if that is what was asked for. After the choice and
    // before the launch, so a person who picks and then watches the
    // application fail still has the association they asked for.
    if args.ask_default || (!args.ask && !args.no_ask && default.is_none()) {
        if let Err(e) = hyprforge_mime::set_default(&mime, &chosen.id) {
            eprintln!("mimeopen: couldn't remember that choice: {e}");
        }
    }

    let names: Vec<String> = args.files.iter().map(|f| quoted(f)).collect();
    println!("Opening {} with {}  ({mime})", names.join(", "), chosen.name);

    match launch(&chosen.path, &args.files) {
        Ok(()) => {}
        Err(message) => {
            eprintln!("mimeopen: {message}");
            std::process::exit(exit::NO_APPLICATION);
        }
    }
}

/// Hands the files to one desktop entry, through `gio launch`.
fn launch(entry: &Path, files: &[String]) -> Result<(), String> {
    let status = std::process::Command::new("gio")
        .arg("launch")
        .arg(entry)
        .args(files)
        .status();
    match status {
        Ok(status) if status.success() => Ok(()),
        Ok(status) => Err(format!("the application exited with {status}")),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Err(
            "gio isn't installed, so a chosen application can't be started. Install glib2."
                .to_string(),
        ),
        Err(e) => Err(e.to_string()),
    }
}

/// The numbered prompt, and the answer.
///
/// Reads from the terminal, so it exists only for an interactive run —
/// `--no-ask` is what a script or `xdg-open` uses. Anything that is not
/// a number in range is "cancelled", which is the reference's behaviour
/// and the safe reading of a stray keypress.
///
/// When a default is being set, the list ends with "Other…", which
/// takes a command line and writes a desktop entry for it — the way to
/// say "none of these, use *this*". Borrowed or owned, because that
/// last option produces an application that did not exist a moment ago.
fn choose<'a>(mime: &str, set_default: bool, apps: &[&'a App]) -> Option<Cow<'a, App>> {
    if apps.is_empty() && !set_default {
        return None;
    }
    let mut out = std::io::stdout().lock();
    if set_default {
        let _ = writeln!(out, "Please choose a default application for files of type {mime}\n");
    } else {
        let _ = writeln!(out, "Please choose an application\n");
    }
    for (index, app) in apps.iter().enumerate() {
        let entry = app.id.trim_end_matches(".desktop");
        let _ = writeln!(out, "\t{}) {}  ({entry})", index + 1, app.name);
    }
    if set_default {
        let _ = writeln!(out, "\t{}) Other...", apps.len() + 1);
    }
    let _ = write!(out, "\nuse application #");
    let _ = out.flush();

    let mut answer = String::new();
    if std::io::stdin().lock().read_line(&mut answer).is_err() {
        eprintln!("Cancelled");
        std::process::exit(exit::CANCELLED);
    }
    let Ok(number) = answer.trim().parse::<usize>() else {
        eprintln!("Cancelled");
        std::process::exit(exit::CANCELLED);
    };
    if set_default && number == apps.len() + 1 {
        return custom();
    }
    match number.checked_sub(1).and_then(|index| apps.get(index)) {
        Some(app) => Some(Cow::Borrowed(*app)),
        None => {
            eprintln!("Cancelled");
            std::process::exit(exit::CANCELLED);
        }
    }
}

/// "Other…": a command typed at the prompt, written out as a desktop
/// entry so it can be launched and remembered like any other
/// application.
fn custom<'a>() -> Option<Cow<'a, App>> {
    let mut out = std::io::stdout().lock();
    let _ = write!(out, "use command: ");
    let _ = out.flush();
    let mut command = String::new();
    if std::io::stdin().lock().read_line(&mut command).is_err() || command.trim().is_empty() {
        eprintln!("Cancelled");
        std::process::exit(exit::CANCELLED);
    }
    let command = command.trim().to_string();
    let applications = hyprforge_paths::data_home().join("applications");
    match hyprforge_mime::apps::write_custom_entry(&applications, &command) {
        Ok(id) => {
            let path = applications.join(&id);
            Some(Cow::Owned(App {
                name: command.split_whitespace().next().unwrap_or(&command).to_string(),
                id,
                icon: None,
                path,
                installed: true,
            }))
        }
        Err(e) => {
            eprintln!("mimeopen: couldn't record that command: {e}");
            std::process::exit(exit::NO_APPLICATION);
        }
    }
}

/// A filename as printed: quoted on a terminal, raw when piped — the
/// same rule `mimetype` follows.
fn quoted(file: &str) -> String {
    if std::io::stdout().is_terminal() {
        format!("{file:?}")
    } else {
        file.to_string()
    }
}

fn parse(raw: Vec<String>) -> Result<Args, (String, i32)> {
    let mut args = Args::default();
    let mut rest = raw.into_iter().peekable();
    while let Some(argument) = rest.peek().cloned() {
        if !argument.starts_with('-') || argument == "-" {
            break;
        }
        rest.next();
        if argument == "--" {
            break;
        }
        if let Some(long) = argument.strip_prefix("--") {
            let (name, inline) = match long.split_once('=') {
                Some((name, value)) => (name, Some(value.to_string())),
                None => (long, None),
            };
            let Some((_, _, takes_value)) = OPTIONS.iter().find(|(option, _, _)| *option == name)
            else {
                return Err((format!("mimeopen: unrecognized option '--{name}'"), exit::UNKNOWN_OPTION));
            };
            let value = match takes_value {
                false => None,
                true => match inline.or_else(|| rest.next()) {
                    Some(value) => Some(value),
                    None => {
                        return Err((
                            format!("mimeopen: option '--{name}' requires an argument"),
                            exit::MISSING_ARGUMENT,
                        ))
                    }
                },
            };
            set(&mut args, name, value);
            continue;
        }
        for letter in argument.trim_start_matches('-').chars() {
            let Some((name, _, takes_value)) =
                OPTIONS.iter().find(|(_, short, _)| *short == Some(letter))
            else {
                return Err((format!("mimeopen: unrecognized option '{letter}'"), exit::UNKNOWN_OPTION));
            };
            let value = match takes_value {
                false => None,
                true => match rest.next() {
                    Some(value) => Some(value),
                    None => {
                        return Err((
                            format!("mimeopen: option '-{letter}' requires an argument"),
                            exit::MISSING_ARGUMENT,
                        ))
                    }
                },
            };
            set(&mut args, name, value);
        }
    }
    args.files.extend(rest);
    Ok(args)
}

fn set(args: &mut Args, name: &str, value: Option<String>) {
    match name {
        "help" | "usage" => args.help = true,
        "version" => args.version = true,
        "dereference" => args.dereference = true,
        "debug" => args.debug = true,
        "database" => args.database = value,
        "magic-only" => args.magic_only = true,
        "ask" => args.ask = true,
        "ask-default" => args.ask_default = true,
        "no-ask" => args.no_ask = true,
        _ => {}
    }
}

fn usage() -> String {
    "\
usage: mimeopen [options] files

  -h, --help             this text
  -u, --usage            the same
  -v, --version          print the version
  -a, --ask              ask which application, and do not remember
  -d, --ask-default      ask, and make the answer the default
  -n, --no-ask           never ask; use the default if there is one
  -M, --magic-only       decide the type from the contents alone
  -L, --dereference      follow a symlink to what it points at
  -D, --debug            say which directories are being read
      --database DIRS    read these directories instead of the XDG ones

With none of -a, -d or -n: the default is used if there is one, and
otherwise you are asked and the answer is remembered.
"
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parsed(argv: &[&str]) -> Args {
        parse(argv.iter().map(|a| a.to_string()).collect()).expect("these parse")
    }

    #[test]
    fn the_three_asking_modes_are_told_apart() {
        assert!(parsed(&["-a", "f"]).ask);
        assert!(parsed(&["-d", "f"]).ask_default);
        assert!(parsed(&["-n", "f"]).no_ask);
        let plain = parsed(&["f"]);
        assert!(!plain.ask && !plain.ask_default && !plain.no_ask);
    }

    /// The form `xdg-open` itself uses as a fallback, which must keep
    /// working exactly as it reads.
    #[test]
    fn the_form_xdg_open_calls_is_parsed() {
        let args = parsed(&["-L", "-n", "/tmp/part.3mf"]);
        assert!(args.dereference && args.no_ask);
        assert_eq!(args.files, ["/tmp/part.3mf"]);
    }

    #[test]
    fn options_and_files_are_told_apart_the_same_way_as_mimetype() {
        assert_eq!(parsed(&["--", "-n"]).files, ["-n"]);
        assert_eq!(parsed(&["-"]).files, ["-"]);
        assert_eq!(parsed(&["--database", "/a:/b", "f"]).database.as_deref(), Some("/a:/b"));
        let (message, code) = parse(vec!["--nope".to_string()]).unwrap_err();
        assert!(message.contains("--nope"));
        assert_eq!(code, exit::UNKNOWN_OPTION);
    }

    /// An empty list is not a prompt with nothing under it.
    /// With nothing installed and no default to set, there is nothing
    /// to ask about. (Setting a default still has "Other…" to offer,
    /// which is why that case is not the same.)
    #[test]
    fn nothing_to_choose_from_is_not_a_question() {
        assert!(choose("model/stl", false, &[]).is_none());
    }
}
