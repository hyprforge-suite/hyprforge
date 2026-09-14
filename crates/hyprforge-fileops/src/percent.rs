//! RFC 2396 percent-encoding for the trash spec's `Path=` field.
//!
//! Written by hand rather than pulled in as a dependency: it is one loop
//! in each direction, and the spec's "safe" character set is narrow
//! enough that reaching for a general-purpose URL-encoding crate (tuned
//! for query strings, not paths) would take as much reading of its docs
//! as writing this did. Confirmed against 29 real `.trashinfo` files
//! already on this machine (written by gvfs): a path containing a space
//! is stored as `...Monkey%20Around.mp4`, matching what this encodes.
//!
//! Operates on raw bytes, not `str`: a Linux filename is whatever bytes
//! `readdir` returns, not necessarily valid UTF-8, and rejecting a
//! non-UTF-8 filename outright would make trashing it impossible for
//! exactly the files most likely to need it (the ones some other tool
//! already mangled into that state). `OsStrExt`/`OsStringExt` carry those
//! bytes through encode and decode without ever requiring they form
//! valid UTF-8 — the produced `Path=` value is ASCII (every non-ASCII or
//! reserved byte is escaped), but what it *decodes back to* need not be.

use std::ffi::OsString;
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};

/// Bytes that never need escaping: RFC 2396 `unreserved`
/// (`ALPHA | DIGIT | mark`, `mark` being `` -_.!~*'() ``) plus `/`, since
/// this function encodes a whole *path*, not one segment — escaping the
/// separator would make the result unreadable for no safety benefit, as
/// `/` can never be confused with an escaped byte (`%` is always escaped
/// itself, so there is no ambiguity to create).
fn is_safe(b: u8) -> bool {
    b.is_ascii_alphanumeric()
        || matches!(b, b'-' | b'_' | b'.' | b'~' | b'!' | b'*' | b'\'' | b'(' | b')' | b'/')
}

/// Encodes `path` for use as a `.trashinfo` `Path=` value.
pub fn encode_path(path: &Path) -> String {
    let mut out = String::new();
    for &b in path.as_os_str().as_bytes() {
        if is_safe(b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

#[derive(Debug, thiserror::Error)]
#[error("invalid percent-encoding at byte offset {offset}")]
pub struct DecodeError {
    pub offset: usize,
}

/// Reverses [`encode_path`]. Rejects a `%` not followed by two hex
/// digits rather than passing it through literally — a `.trashinfo` this
/// malformed is exactly the "exists and will not parse" case CLAUDE.md
/// says must be reported, not guessed at.
pub fn decode_path(encoded: &str) -> Result<PathBuf, DecodeError> {
    let bytes = encoded.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = bytes.get(i + 1..i + 3).ok_or(DecodeError { offset: i })?;
            let hex_str = std::str::from_utf8(hex).map_err(|_| DecodeError { offset: i })?;
            let value = u8::from_str_radix(hex_str, 16).map_err(|_| DecodeError { offset: i })?;
            out.push(value);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    Ok(PathBuf::from(OsString::from_vec(out)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_path_with_a_space_hash_and_percent_round_trips() {
        let original = Path::new("/home/user/My Docs/100% #done.txt");
        let encoded = encode_path(original);
        assert_eq!(decode_path(&encoded).unwrap(), original);
    }

    #[test]
    fn a_non_ascii_path_round_trips_as_bytes_not_as_utf8_text() {
        let original = Path::new("/home/user/café/résumé.pdf");
        let encoded = encode_path(original);
        // Every byte of the encoded form is plain ASCII: non-ASCII bytes
        // must all have been escaped, not passed through raw.
        assert!(encoded.is_ascii());
        assert_eq!(decode_path(&encoded).unwrap(), original);
    }

    #[test]
    fn a_space_encodes_as_percent_two_zero_matching_gvfs_on_this_machine() {
        assert_eq!(encode_path(Path::new("Monkey Around.mp4")), "Monkey%20Around.mp4");
    }

    #[test]
    fn slashes_are_left_unescaped() {
        assert_eq!(encode_path(Path::new("/a/b/c")), "/a/b/c");
    }

    #[test]
    fn a_percent_sign_that_is_not_a_valid_escape_is_reported_not_ignored() {
        assert!(decode_path("100%").is_err());
        assert!(decode_path("100%ZZ").is_err());
    }
}
