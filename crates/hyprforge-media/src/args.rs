//! What the command line asked for.
//!
//! Pure over `&[String]`, so every case is a test rather than a thing to
//! try by hand.
//!
//! # `file://`, because that is what a desktop hands a viewer
//!
//! The `.desktop` entry uses `%U`, so `gio` passes a URI rather than a
//! path — and a viewer that cannot read one is a viewer that does
//! nothing when you double-click a photograph in the file manager. That
//! is the single most common way this program will ever be started.

use std::path::PathBuf;

/// What to show at startup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Args {
    /// A picture, whose folder becomes the filmstrip.
    Picture(PathBuf),
    /// A folder, showing its first item.
    Folder(PathBuf),
    /// Nothing was named.
    Nothing,
}

/// Parses the arguments after the program name.
///
/// Whether a path is a picture or a folder is not decided here — this
/// module never touches the filesystem, because a function that does
/// cannot be tested without one. [`Args::Picture`] is the shape of a
/// plain path; the caller asks the disk which it is.
pub fn parse(args: &[String]) -> Args {
    let Some(first) = args.iter().find(|a| !a.starts_with('-')) else {
        return Args::Nothing;
    };
    match from_uri(first) {
        Some(path) => Args::Picture(path),
        None => Args::Picture(PathBuf::from(first)),
    }
}

/// A `file://` URI as a path, or `None` if this is not one.
///
/// Percent-decoding by hand rather than with a dependency: the only
/// escapes that reach a viewer in practice are spaces and the handful of
/// punctuation `gio` escapes, and the whole decoder is fifteen lines.
fn from_uri(text: &str) -> Option<PathBuf> {
    let rest = text.strip_prefix("file://")?;
    // `file:///home/x` — the empty authority is the common form, but a
    // localhost authority is legal and means the same thing.
    let rest = rest.strip_prefix("localhost").unwrap_or(rest);
    if !rest.starts_with('/') {
        return None;
    }
    Some(PathBuf::from(percent_decode(rest)))
}

fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).ok();
            if let Some(value) = hex.and_then(|h| u8::from_str_radix(h, 16).ok()) {
                out.push(value);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    // A URI that does not decode to UTF-8 is not something to fail over:
    // the lossy form still names a file the user can be told about.
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_string()).collect()
    }

    #[test]
    fn the_first_picture_is_the_one_named_on_the_command_line() {
        assert_eq!(
            parse(&args(&["/photos/a.jpg"])),
            Args::Picture(PathBuf::from("/photos/a.jpg"))
        );
    }

    #[test]
    fn nothing_named_is_nothing_to_show() {
        assert_eq!(parse(&[]), Args::Nothing);
    }

    /// How the file manager actually starts this program.
    #[test]
    fn a_file_uri_is_the_path_it_names() {
        assert_eq!(
            parse(&args(&["file:///home/someone/Pictures/a.jpg"])),
            Args::Picture(PathBuf::from("/home/someone/Pictures/a.jpg"))
        );
    }

    #[test]
    fn a_uri_with_escapes_decodes_them() {
        assert_eq!(
            parse(&args(&["file:///home/a%20b/c%2Bd.jpg"])),
            Args::Picture(PathBuf::from("/home/a b/c+d.jpg"))
        );
    }

    #[test]
    fn a_localhost_authority_means_the_same_thing() {
        assert_eq!(
            parse(&args(&["file://localhost/srv/a.jpg"])),
            Args::Picture(PathBuf::from("/srv/a.jpg"))
        );
    }

    /// A URI for something that is not a local file is not a path, and
    /// must not be turned into one by stripping a prefix.
    #[test]
    fn a_uri_that_is_not_a_local_file_is_left_alone() {
        assert_eq!(
            parse(&args(&["https://example.com/a.jpg"])),
            Args::Picture(PathBuf::from("https://example.com/a.jpg"))
        );
    }

    /// A flag is not a filename. Nothing here defines any flags yet, but
    /// reading `--help` as a path would be a strange first impression.
    #[test]
    fn a_flag_is_not_mistaken_for_a_picture() {
        assert_eq!(parse(&args(&["--help"])), Args::Nothing);
        assert_eq!(
            parse(&args(&["--verbose", "/photos/a.jpg"])),
            Args::Picture(PathBuf::from("/photos/a.jpg"))
        );
    }

    /// A malformed escape is kept as written rather than dropped: the
    /// name is still the best thing to show in a message about it.
    #[test]
    fn a_broken_escape_does_not_lose_the_rest_of_the_name() {
        assert_eq!(
            parse(&args(&["file:///home/a%zz/b.jpg"])),
            Args::Picture(PathBuf::from("/home/a%zz/b.jpg"))
        );
    }
}
