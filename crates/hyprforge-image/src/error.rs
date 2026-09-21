//! What can go wrong, kept apart.
//!
//! Three states, and collapsing any two of them costs the user the one
//! sentence that would have told them what to do — the rule CLAUDE.md
//! records about never reading "this file could not be read" as "there is
//! nothing here".

use std::path::PathBuf;

/// Why a picture could not be shown.
#[derive(Debug, thiserror::Error)]
pub enum ImageError {
    /// The file could not be read at all: gone, or not permitted.
    #[error("couldn't read {path}: {source}")]
    Unreadable {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    /// The file is there and readable, and is not a picture this build
    /// can decode — a format nobody enabled, or bytes that are not an
    /// image at all.
    ///
    /// Distinct from [`ImageError::Unreadable`] because the two need
    /// different sentences: one is "this file is missing", the other is
    /// "this file is not something I can open", and a viewer that says
    /// the first about the second sends the user looking for a problem
    /// with their disk.
    #[error("{path} isn't an image this build can decode: {source}")]
    Undecodable {
        path: PathBuf,
        #[source]
        source: image::ImageError,
    },

    /// The header parsed and claims dimensions past what may be decoded
    /// at all — see [`crate::budget`].
    ///
    /// Its own case rather than an `Undecodable`, because nothing is
    /// wrong with the file: this is a refusal, and it is the one failure
    /// here that a bigger machine would not have.
    #[error("{path} is {width}x{height}, which is too large to decode safely")]
    TooLarge { path: PathBuf, width: u32, height: u32 },
}

impl ImageError {
    /// The file this is about, for a message that names it.
    pub fn path(&self) -> &std::path::Path {
        match self {
            ImageError::Unreadable { path, .. }
            | ImageError::Undecodable { path, .. }
            | ImageError::TooLarge { path, .. } => path,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::ErrorKind;

    /// The distinction this module exists for, pinned as text: the two
    /// messages must not read the same, or the state they describe has
    /// been collapsed in practice however separate the types are.
    #[test]
    fn a_missing_file_and_an_unsupported_one_do_not_say_the_same_thing() {
        let missing = ImageError::Unreadable {
            path: PathBuf::from("/x/gone.png"),
            source: std::io::Error::new(ErrorKind::NotFound, "no such file"),
        };
        let unsupported = ImageError::TooLarge {
            path: PathBuf::from("/x/huge.png"),
            width: 200_000,
            height: 4,
        };
        assert_ne!(missing.to_string(), unsupported.to_string());
        assert!(missing.to_string().contains("gone.png"));
        assert!(unsupported.to_string().contains("200000"));
    }

    /// Every message names the file. A viewer with several pictures open
    /// and a message that does not say which one is a message about
    /// nothing.
    #[test]
    fn every_failure_names_the_file_it_is_about() {
        let e = ImageError::Unreadable {
            path: PathBuf::from("/photos/a.jpg"),
            source: std::io::Error::new(ErrorKind::PermissionDenied, "denied"),
        };
        assert_eq!(e.path(), std::path::Path::new("/photos/a.jpg"));
        assert!(e.to_string().contains("/photos/a.jpg"));
    }
}
