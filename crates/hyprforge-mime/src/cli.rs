//! The argument grammar `mimetype` and `mimeopen` share.
//!
//! Both are drop-ins for the commands of the same name in
//! `perl-file-mimeinfo`, and both are invoked by `xdg-open`, so the
//! grammar *is* the compatibility claim: bundled short options
//! (`-bl de`), `--long=value` as well as `--long value`, `--` ending the
//! options, and a bare `-` being a filename rather than a flag.
//!
//! It lives here because it was written twice. The two parsers were
//! identical but for the program name in four error strings, with their
//! own tests each — so a fix to bundling or to `--` had to be made and
//! re-tested twice, and a divergence would have meant two commands that
//! parse their shared options differently. That is the one kind of bug
//! a compatibility command cannot afford.

/// Exit codes, which are the reference implementation's own.
pub mod exit {
    /// An option nothing here knows.
    pub const UNKNOWN_OPTION: i32 = 1;
    /// An option that takes an argument, without one.
    pub const MISSING_ARGUMENT: i32 = 2;
    /// Nothing to look at.
    pub const NO_FILES: i32 = 4;
}

/// One option: its long name, its short letter if it has one, and
/// whether it takes a value.
pub type Option_ = (&'static str, Option<char>, bool);

/// What a command line turned out to be: the options given, in order,
/// each with its value; and everything that was not an option.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Parsed {
    pub options: Vec<(&'static str, Option<String>)>,
    pub files: Vec<String>,
}

/// Splits a command line against an option table.
///
/// `program` appears in the error messages, which are the reference's
/// own wording. An error carries the exit code to use with it, because
/// the two are one decision: an unknown option is 1, a missing argument
/// is 2.
pub fn parse(program: &str, table: &[Option_], raw: Vec<String>) -> Result<Parsed, (String, i32)> {
    let mut parsed = Parsed::default();
    let mut rest = raw.into_iter().peekable();
    while let Some(argument) = rest.peek().cloned() {
        // A bare `-` is a filename, not an option — and `--` ends the
        // options, so a file genuinely called `-b` can be asked about.
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
            let Some((name, _, takes_value)) = table.iter().find(|(option, _, _)| *option == name)
            else {
                return Err((
                    format!("{program}: unrecognized option '--{name}'"),
                    exit::UNKNOWN_OPTION,
                ));
            };
            let value = match takes_value {
                false => None,
                true => match inline.or_else(|| rest.next()) {
                    Some(value) => Some(value),
                    None => {
                        return Err((
                            format!("{program}: option '--{name}' requires an argument"),
                            exit::MISSING_ARGUMENT,
                        ))
                    }
                },
            };
            parsed.options.push((name, value));
            continue;
        }
        // `-abc`: each letter is its own option, and one that takes a
        // value takes the next argument.
        for letter in argument.trim_start_matches('-').chars() {
            let Some((name, _, takes_value)) =
                table.iter().find(|(_, short, _)| *short == Some(letter))
            else {
                return Err((
                    format!("{program}: unrecognized option '{letter}'"),
                    exit::UNKNOWN_OPTION,
                ));
            };
            let value = match takes_value {
                false => None,
                true => match rest.next() {
                    Some(value) => Some(value),
                    None => {
                        return Err((
                            format!("{program}: option '-{letter}' requires an argument"),
                            exit::MISSING_ARGUMENT,
                        ))
                    }
                },
            };
            parsed.options.push((name, value));
        }
    }
    parsed.files.extend(rest);
    Ok(parsed)
}

/// A filename as printed. On a terminal the reference quotes it, so a
/// name with a space or a newline in it cannot be misread; piped, it
/// prints the name raw, which is what a script downstream needs.
pub fn shown_name(file: &str) -> String {
    use std::io::IsTerminal;
    if std::io::stdout().is_terminal() {
        format!("{file:?}")
    } else {
        file.to_string()
    }
}

/// The directories to read the database from: what `--database` named,
/// or the XDG ones.
pub fn database_dirs(database: &Option<String>) -> Vec<std::path::PathBuf> {
    match database {
        Some(list) => list.split(':').filter(|d| !d.is_empty()).map(std::path::PathBuf::from).collect(),
        None => crate::data_dirs(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TABLE: &[Option_] = &[
        ("brief", Some('b'), false),
        ("language", Some('l'), true),
        ("all", Some('a'), false),
        ("database", None, true),
    ];

    fn parsed(argv: &[&str]) -> Parsed {
        parse("mimetype", TABLE, argv.iter().map(|a| a.to_string()).collect()).expect("these parse")
    }

    #[test]
    fn short_options_can_be_bundled_and_the_last_one_takes_the_argument() {
        let args = parsed(&["-bl", "de", "part.3mf"]);
        assert_eq!(args.options, [("brief", None), ("language", Some("de".to_string()))]);
        assert_eq!(args.files, ["part.3mf"]);
    }

    #[test]
    fn a_long_option_takes_its_value_attached_or_apart() {
        assert_eq!(parsed(&["--language=fr", "x"]).options, [("language", Some("fr".to_string()))]);
        assert_eq!(parsed(&["--language", "fr", "x"]).options, [("language", Some("fr".to_string()))]);
    }

    /// Options stop at the first thing that is not one, so a file
    /// called `-b` can be asked about.
    #[test]
    fn a_double_dash_ends_the_options() {
        let args = parsed(&["--brief", "--", "-b", "--all"]);
        assert_eq!(args.options, [("brief", None)]);
        assert_eq!(args.files, ["-b", "--all"]);
    }

    #[test]
    fn a_bare_dash_is_a_filename_not_an_option() {
        assert_eq!(parsed(&["-"]).files, ["-"]);
    }

    #[test]
    fn an_unknown_option_is_named_with_the_exit_code_the_reference_uses() {
        let (message, code) = parse("mimetype", TABLE, vec!["--nonsense".to_string()]).unwrap_err();
        assert!(message.contains("mimetype: unrecognized option '--nonsense'"), "{message}");
        assert_eq!(code, exit::UNKNOWN_OPTION);

        let (message, code) = parse("mimeopen", TABLE, vec!["--language".to_string()]).unwrap_err();
        assert!(message.contains("mimeopen: option '--language' requires an argument"), "{message}");
        assert_eq!(code, exit::MISSING_ARGUMENT);
    }

    /// The program name in the message is the caller's, since the two
    /// commands share this parser.
    #[test]
    fn the_error_names_the_command_that_was_run() {
        let (message, _) = parse("mimeopen", TABLE, vec!["-z".to_string()]).unwrap_err();
        assert!(message.starts_with("mimeopen:"), "{message}");
    }

    #[test]
    fn the_database_option_replaces_the_xdg_directories() {
        let dirs = database_dirs(&Some("/a:/b".to_string()));
        assert_eq!(dirs, [std::path::PathBuf::from("/a"), std::path::PathBuf::from("/b")]);
        assert_eq!(database_dirs(&None), crate::data_dirs());
    }
}
