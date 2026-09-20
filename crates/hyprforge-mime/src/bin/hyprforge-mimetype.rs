//! `mimetype` — what kind of file is this?
//!
//! A drop-in for the `mimetype` command from `perl-file-mimeinfo`, in
//! Rust, with the same options, the same output shape and the same
//! answers. That compatibility is the point rather than a nicety:
//! `xdg-open`, on a desktop it does not recognise (Hyprland is one),
//! asks `xdg-mime query filetype`, which runs `mimetype` when it is
//! installed and falls back to `file --mime-type` when it is not. The
//! fallback reads bytes and never names, so an STL becomes
//! `application/octet-stream`, has no default application, and
//! `xdg-open` — finding nothing — works down a built-in list of web
//! browsers. That is how a 3D model opens in Firefox.
//!
//! So this exists to be installed as `mimetype` on a machine that would
//! rather not carry a perl distribution for it. Every answer comes from
//! `hyprforge-mime`, which reads the same freedesktop database in the
//! same order — see [`hyprforge_mime::lookup`].
//!
//! ```text
//! mimetype part.3mf              part.3mf: model/3mf
//! mimetype -b part.3mf           model/3mf
//! mimetype -d part.3mf           part.3mf: 3MF 3D model
//! mimetype -a download           download: image/png
//! ```
//!
//! # Where it deliberately differs
//!
//! Nothing in the output, and one thing in the implementation: a file
//! this cannot read at all is reported by its name rather than by
//! dying. The perl version returns nothing for it.

use hyprforge_mime::lookup::{self, Lookup};
use hyprforge_mime::types::Types;
use std::io::{IsTerminal, Read, Write};
use std::path::{Path, PathBuf};

/// Exit codes, which are the perl script's own.
mod exit {
    /// An option nothing here knows.
    pub const UNKNOWN_OPTION: i32 = 1;
    /// An option that takes an argument, without one.
    pub const MISSING_ARGUMENT: i32 = 2;
    /// Nothing to look at.
    pub const NO_FILES: i32 = 4;
}

#[derive(Debug, Default)]
struct Args {
    files: Vec<String>,
    /// Print only the type, with no filename in front.
    brief: bool,
    /// Do not pad the filenames into a column.
    noalign: bool,
    /// Print what a person would call the type.
    describe: bool,
    /// Behave like `file(1)`: describe rather than name the type.
    file_compat: bool,
    /// With `--file-compat`, print the type after all.
    mimetype: bool,
    /// Read the file to type from standard input.
    stdin: bool,
    /// Read the *names* to look at from this file.
    namefile: Option<String>,
    /// `%f`, `%m`, `%d`.
    output_format: Option<String>,
    /// Which translation of a description to prefer.
    language: Option<String>,
    /// What comes between the name and the type.
    separator: String,
    /// Consider only the contents.
    magic_only: bool,
    /// Every way of answering, not just the best one.
    all: bool,
    /// Follow a symlink to what it points at.
    dereference: bool,
    /// The database directories to read, overriding XDG.
    database: Option<String>,
    debug: bool,
    help: bool,
    version: bool,
}

fn main() {
    let raw: Vec<String> = std::env::args().skip(1).collect();
    // Invoked as `file`, behave like it — the perl script does this and
    // some setups rely on it.
    let called_as_file = std::env::args()
        .next()
        .map(|arg0| Path::new(&arg0).file_name().map(|n| n == "file").unwrap_or(false))
        .unwrap_or(false);

    let mut args = match parse(raw) {
        Ok(args) => args,
        Err((message, code)) => {
            eprintln!("{message}");
            if code != exit::NO_FILES {
                eprintln!("Try 'mimetype --help' for more information.");
            }
            std::process::exit(code);
        }
    };
    args.file_compat |= called_as_file;
    // `file` describes; `file -i` names the type.
    if args.file_compat && !args.mimetype {
        args.describe = true;
    }

    if args.help {
        print!("{}", usage());
        return;
    }
    if args.version {
        println!("mimetype (hyprforge-mime) {}", env!("CARGO_PKG_VERSION"));
        return;
    }

    if let Some(namefile) = &args.namefile {
        match std::fs::read_to_string(namefile) {
            Ok(text) => {
                let mut names: Vec<String> = text.lines().map(str::to_string).collect();
                names.append(&mut args.files);
                args.files = names;
            }
            Err(e) => {
                eprintln!("mimetype: couldn't open file: {namefile}: {e}");
                std::process::exit(exit::MISSING_ARGUMENT);
            }
        }
    }

    if args.files.is_empty() && !args.stdin {
        eprintln!("usage: mimetype [options] files");
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
    let lookup = Lookup::load_from(&dirs);
    if lookup.globs.is_empty() && lookup.magic.is_empty() {
        eprintln!("mimetype: no mimeinfo database found");
        std::process::exit(exit::UNKNOWN_OPTION);
    }
    let mut types = lookup.types.clone();

    // The name column is as wide as the widest name, with "STDIN" (five
    // characters) as the floor — the perl script's own rule.
    let width = match args.brief || args.noalign {
        true => 0,
        false => args.files.iter().map(|f| shown_name(f).chars().count()).chain([5]).max().unwrap_or(5),
    };

    let mut out = std::io::stdout().lock();
    if args.stdin {
        let mut data = Vec::new();
        let _ = std::io::stdin().lock().read_to_end(&mut data);
        let found = lookup.of_data_and_name(&data, None);
        let line = format(&args, &mut types, &dirs, "STDIN", &found.mime, width);
        let _ = writeln!(out, "{line}");
        return;
    }

    for file in &args.files {
        let path = PathBuf::from(file);
        let name = shown_name(file);
        for mime in answers(&lookup, &path, &args) {
            let line = format(&args, &mut types, &dirs, &name, &mime, width);
            let _ = writeln!(out, "{line}");
        }
    }
}

/// Every type to print for one file: one line normally, several with
/// `--all`.
///
/// `--all` reports each *method's* answer in turn — what it is on disk,
/// what its name says, what its contents say, and the fallback — rather
/// than a ranked list, which is what the perl version does and is more
/// useful for the question it is usually asked: "why did it decide
/// that?"
fn answers(lookup: &Lookup, path: &Path, args: &Args) -> Vec<String> {
    let follow = args.dereference;
    if args.magic_only {
        let data = read_head(path);
        let found = lookup.magic.of_data(&data).map(|m| m.mime);
        return vec![found.unwrap_or_else(|| lookup::fallback(&data).to_string())];
    }
    if !args.all {
        return vec![lookup.of_file(path, follow).mime];
    }
    let mut out = Vec::new();
    if let Some(inode) = hyprforge_mime::types::inode_type(path, follow) {
        out.push(inode.to_string());
    }
    if let Some(name) = lookup.globs.type_of(path) {
        out.push(name.to_string());
    }
    let data = read_head(path);
    if let Some(found) = lookup.magic.of_data(&data) {
        out.push(found.mime);
    }
    out.push(lookup::fallback(&data).to_string());
    out.dedup();
    out
}

/// The first part of a file, for content matching. An unreadable file
/// gives nothing, which the fallback then calls text — the same answer
/// the perl version reaches by a different route.
fn read_head(path: &Path) -> Vec<u8> {
    std::fs::read(path).unwrap_or_default()
}

/// One output line.
fn format(
    args: &Args,
    types: &mut Types,
    dirs: &[PathBuf],
    name: &str,
    mime: &str,
    width: usize,
) -> String {
    let described = |types: &mut Types| -> String {
        types
            .describe_from(dirs, mime, args.language.as_deref())
            .map(str::to_string)
            // A type with no description of its own is shown as itself
            // rather than as nothing at all.
            .unwrap_or_else(|| mime.to_string())
    };
    if let Some(template) = &args.output_format {
        let description = described(types);
        return template
            .replace("%f", name)
            .replace("%m", mime)
            .replace("%d", &description);
    }
    let value = match args.describe {
        true => described(types),
        false => mime.to_string(),
    };
    if args.brief {
        return value;
    }
    if args.noalign {
        return format!("{name}{} {value}", args.separator);
    }
    let padding = " ".repeat((width + 1).saturating_sub(name.chars().count()));
    format!("{name}{}{padding}{value}", args.separator)
}

/// A filename as printed. On a terminal the perl script quotes it (so a
/// name with a space or a newline in it cannot be misread); piped, it
/// prints the name raw, which is what a script downstream needs.
fn shown_name(file: &str) -> String {
    if std::io::stdout().is_terminal() {
        format!("{file:?}")
    } else {
        file.to_string()
    }
}

/// The option table, in the shape the perl script uses: long name,
/// short name, and whether it takes an argument.
const OPTIONS: &[(&str, Option<char>, bool)] = &[
    ("help", Some('h'), false),
    ("usage", Some('u'), false),
    ("version", Some('v'), false),
    ("stdin", None, false),
    ("brief", Some('b'), false),
    ("namefile", Some('f'), true),
    ("noalign", Some('N'), false),
    ("describe", Some('d'), false),
    ("file-compat", None, false),
    ("output-format", None, true),
    ("language", Some('l'), true),
    ("mimetype", Some('i'), false),
    ("dereference", Some('L'), false),
    ("separator", Some('F'), true),
    ("debug", Some('D'), false),
    ("database", None, true),
    ("all", Some('a'), false),
    ("magic-only", Some('M'), false),
];

/// Parses the command line, or says what was wrong and with what exit
/// code.
///
/// Options stop at the first argument that is not one, and `--` ends
/// them explicitly — so a file genuinely called `-b` can be asked about
/// as `mimetype -- -b`.
fn parse(raw: Vec<String>) -> Result<Args, (String, i32)> {
    let mut args = Args { separator: ":".to_string(), ..Args::default() };
    let mut rest = raw.into_iter().peekable();
    while let Some(argument) = rest.peek().cloned() {
        if !argument.starts_with('-') || argument == "-" {
            break;
        }
        rest.next();
        if argument == "--" {
            break;
        }
        // --long, or --long=value
        if let Some(long) = argument.strip_prefix("--") {
            let (name, inline) = match long.split_once('=') {
                Some((name, value)) => (name, Some(value.to_string())),
                None => (long, None),
            };
            let Some((_, _, takes_value)) = OPTIONS.iter().find(|(option, _, _)| *option == name)
            else {
                return Err((format!("mimetype: unrecognized option '--{name}'"), exit::UNKNOWN_OPTION));
            };
            let value = match takes_value {
                false => None,
                true => match inline.or_else(|| rest.next()) {
                    Some(value) => Some(value),
                    None => {
                        return Err((
                            format!("mimetype: option '--{name}' requires an argument"),
                            exit::MISSING_ARGUMENT,
                        ))
                    }
                },
            };
            set(&mut args, name, value);
            continue;
        }
        // -abc, where each letter is its own option and the last may
        // take the next argument.
        for letter in argument.trim_start_matches('-').chars() {
            let Some((name, _, takes_value)) =
                OPTIONS.iter().find(|(_, short, _)| *short == Some(letter))
            else {
                return Err((format!("mimetype: unrecognized option '{letter}'"), exit::UNKNOWN_OPTION));
            };
            let value = match takes_value {
                false => None,
                true => match rest.next() {
                    Some(value) => Some(value),
                    None => {
                        return Err((
                            format!("mimetype: option '-{letter}' requires an argument"),
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
        "stdin" => args.stdin = true,
        "brief" => args.brief = true,
        "namefile" => args.namefile = value,
        "noalign" => args.noalign = true,
        "describe" => args.describe = true,
        "file-compat" => args.file_compat = true,
        "output-format" => args.output_format = value,
        "language" => args.language = value,
        "mimetype" => args.mimetype = true,
        "dereference" => args.dereference = true,
        "separator" => args.separator = value.unwrap_or_else(|| ":".to_string()),
        "debug" => args.debug = true,
        "database" => args.database = value,
        "all" => args.all = true,
        "magic-only" => args.magic_only = true,
        _ => {}
    }
}

fn usage() -> String {
    "\
usage: mimetype [options] files

  -h, --help             this text
  -u, --usage            the same
  -v, --version          print the version
      --stdin            read the file from standard input
  -b, --brief            print only the type, with no filename
  -f, --namefile FILE    read the filenames to look at from FILE
  -N, --noalign          do not pad filenames into a column
  -d, --describe         print what a person calls the type
      --file-compat      behave like file(1)
      --output-format F  %f filename, %m type, %d description
  -l, --language LANG    which translation of a description to use
  -i, --mimetype         with --file-compat, print the type after all
  -L, --dereference      follow a symlink to what it points at
  -F, --separator SEP    what goes between name and type (default ':')
  -D, --debug            say which directories are being read
      --database DIRS    read these directories instead of the XDG ones
  -a, --all              every way of answering, not just the best
  -M, --magic-only       consider only the file's contents

Answers come from the freedesktop shared MIME database, in the order
File::MimeInfo::Magic uses: what it is on disk, then strong content
rules, then the filename, then weaker content rules, then a fallback.
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
    fn short_options_can_be_bundled_and_the_last_one_takes_the_argument() {
        let args = parsed(&["-bl", "de", "part.3mf"]);
        assert!(args.brief);
        assert_eq!(args.language.as_deref(), Some("de"));
        assert_eq!(args.files, ["part.3mf"]);
    }

    #[test]
    fn a_long_option_takes_its_value_attached_or_apart() {
        assert_eq!(parsed(&["--language=fr", "x"]).language.as_deref(), Some("fr"));
        assert_eq!(parsed(&["--language", "fr", "x"]).language.as_deref(), Some("fr"));
        assert_eq!(parsed(&["--database", "/a:/b", "x"]).database.as_deref(), Some("/a:/b"));
    }

    /// Options stop at the first thing that is not one, so a file
    /// called `-b` can be asked about — and a filename that merely
    /// starts with a dash after `--` is a filename.
    #[test]
    fn a_double_dash_ends_the_options() {
        let args = parsed(&["--brief", "--", "-b", "--all"]);
        assert!(args.brief);
        assert!(!args.all, "past the --, it is a filename");
        assert_eq!(args.files, ["-b", "--all"]);
    }

    #[test]
    fn a_bare_dash_is_a_filename_not_an_option() {
        assert_eq!(parsed(&["-"]).files, ["-"]);
    }

    #[test]
    fn an_unknown_option_is_named_with_the_exit_code_the_script_uses() {
        let (message, code) = parse(vec!["--nonsense".to_string()]).unwrap_err();
        assert!(message.contains("--nonsense"), "{message}");
        assert_eq!(code, exit::UNKNOWN_OPTION);

        let (message, code) = parse(vec!["--language".to_string()]).unwrap_err();
        assert!(message.contains("requires an argument"), "{message}");
        assert_eq!(code, exit::MISSING_ARGUMENT);
    }

    /// The column is as wide as the widest name plus one, which is what
    /// makes several files line up.
    #[test]
    fn the_default_output_aligns_the_types_into_a_column() {
        let args = Args { separator: ":".to_string(), ..Args::default() };
        let mut types = Types::default();
        let line = format(&args, &mut types, &[], "a.txt", "text/plain", 9);
        assert_eq!(line, "a.txt:     text/plain");
        // A name longer than the column gets no padding at all, not
        // even one space — perl's `' ' x (negative)` is the empty
        // string. Unreachable in normal use, since the width is the
        // longest name, but faithful for anything that sets it itself.
        let longer = format(&args, &mut types, &[], "longer.png", "image/png", 9);
        assert_eq!(longer, "longer.png:image/png");
    }

    #[test]
    fn brief_and_noalign_and_a_custom_separator() {
        let mut types = Types::default();
        let brief = Args { brief: true, separator: ":".to_string(), ..Args::default() };
        assert_eq!(format(&brief, &mut types, &[], "a.txt", "text/plain", 9), "text/plain");

        let noalign = Args { noalign: true, separator: ":".to_string(), ..Args::default() };
        assert_eq!(format(&noalign, &mut types, &[], "a.txt", "text/plain", 9), "a.txt: text/plain");

        let tabbed = Args { noalign: true, separator: "\t".to_string(), ..Args::default() };
        assert_eq!(format(&tabbed, &mut types, &[], "a.txt", "text/plain", 9), "a.txt\t text/plain");
    }

    #[test]
    fn an_output_format_fills_in_the_name_type_and_description() {
        let args = Args {
            output_format: Some("%f is %m (%d)".to_string()),
            separator: ":".to_string(),
            ..Args::default()
        };
        let mut types = Types::default();
        assert_eq!(
            format(&args, &mut types, &[], "a.txt", "text/plain", 0),
            "a.txt is text/plain (text/plain)",
            "with no database to read, a description falls back to the type"
        );
    }

    /// `--all` reports each method's answer, and a file whose name and
    /// contents agree does not say the same thing twice.
    #[test]
    fn all_reports_each_method_without_repeating_itself() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.txt");
        std::fs::write(&path, b"plain words").unwrap();
        let lookup = Lookup {
            globs: hyprforge_mime::Globs::parse("50:text/plain:*.txt\n"),
            ..Lookup::default()
        };
        let args = Args { all: true, ..Args::default() };
        assert_eq!(answers(&lookup, &path, &args), ["text/plain"]);
    }

    #[test]
    fn magic_only_ignores_the_name_entirely() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("lying.txt");
        std::fs::write(&path, b"\x00\x01\x02\x03").unwrap();
        let lookup = Lookup {
            globs: hyprforge_mime::Globs::parse("50:text/plain:*.txt\n"),
            ..Lookup::default()
        };
        let args = Args { magic_only: true, ..Args::default() };
        assert_eq!(answers(&lookup, &path, &args), ["application/octet-stream"]);
    }
}
