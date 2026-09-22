//! Everything that can go wrong, phrased as a sentence someone can act
//! on — the same standard `FilesError` holds itself to, and for the same
//! reason: these messages reach a status bar verbatim, and "check the
//! logs" is not a thing a file manager may say.

use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum ArchiveError {
    /// The bytes are not an archive this crate knows, whatever the name
    /// says.
    ///
    /// Distinct from [`ArchiveError::Damaged`] on purpose: a JPEG named
    /// `.zip` is a mistake about *what the file is*, and telling someone
    /// their archive is corrupt would send them looking for a backup of
    /// a photo that was never damaged.
    #[error("{path} isn't an archive — its contents don't match any format that can be opened.")]
    NotAnArchive { path: PathBuf },

    /// The format is right and the bytes are not.
    #[error("{path} is a damaged {format} archive ({detail}).")]
    Damaged {
        path: PathBuf,
        format: &'static str,
        detail: String,
    },

    /// A format this build can name but cannot handle — a zip whose
    /// entries use a compression method none of the enabled decoders
    /// implement, say. Named apart from `Damaged` because nothing is
    /// wrong with the archive and another tool will open it.
    #[error("{path} uses {detail}, which this can't read yet.")]
    Unsupported { path: PathBuf, detail: String },

    /// The archive, or this member of it, needs a password.
    #[error("{path} is encrypted and needs a password.")]
    PasswordRequired { path: PathBuf },

    #[error("There's no “{member}” inside {archive}.")]
    MemberNotFound { archive: PathBuf, member: String },

    /// A member whose path climbs out of the destination — see
    /// [`crate::model::normalise`] and the extraction guard. Its own
    /// variant rather than a generic failure because it is the one
    /// failure here that is someone's *intent* rather than an accident,
    /// and the message has to be plain about what was refused.
    #[error("{archive} contains an entry (“{member}”) that would write outside the folder you chose. Nothing was extracted.")]
    UnsafeMemberPath { archive: PathBuf, member: String },

    /// Stopped because the person asked for it. An outcome, not a
    /// failure — a caller that reports it as an error is telling someone
    /// their own cancel button went wrong.
    #[error("Cancelled.")]
    Cancelled,

    #[error("{path} couldn't be read ({source}).")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

impl ArchiveError {
    pub fn io(path: impl Into<PathBuf>, source: std::io::Error) -> ArchiveError {
        ArchiveError::Io {
            path: path.into(),
            source,
        }
    }

    /// Whether this is the cancel outcome — so a caller can drop it on
    /// the floor without matching the whole enum.
    pub fn cancelled(&self) -> bool {
        matches!(self, ArchiveError::Cancelled)
    }
}

pub type Result<T> = std::result::Result<T, ArchiveError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_message_is_a_sentence_and_never_defers_to_a_log() {
        let cases = [
            ArchiveError::NotAnArchive { path: "/x/a.zip".into() },
            ArchiveError::Damaged {
                path: "/x/a.zip".into(),
                format: "zip",
                detail: "the central directory is truncated".into(),
            },
            ArchiveError::PasswordRequired { path: "/x/a.7z".into() },
            ArchiveError::MemberNotFound {
                archive: "/x/a.zip".into(),
                member: "gone.txt".into(),
            },
            ArchiveError::UnsafeMemberPath {
                archive: "/x/a.tar".into(),
                member: "../../etc/passwd".into(),
            },
            ArchiveError::Cancelled,
            ArchiveError::Unsupported {
                path: "/x/a.zip".into(),
                detail: "a compression method this build has no decoder for".into(),
            },
            // The two variants that end in text from somebody else's
            // library — which is exactly where a sentence stops being
            // one, because nothing obliges a `zip::result::ZipError` to
            // end in a full stop.
            ArchiveError::io(
                "/x/a.zip",
                std::io::Error::new(std::io::ErrorKind::PermissionDenied, "permission denied"),
            ),
        ];
        for error in cases {
            let message = error.to_string();
            assert!(
                message.ends_with('.') || message.ends_with('!'),
                "not a sentence: {message}"
            );
            let lowered = message.to_lowercase();
            assert!(
                !lowered.contains("log") && !lowered.contains("error code"),
                "sends the reader somewhere they cannot go: {message}"
            );
        }
    }

    /// The distinction the two variants exist to keep: a file that was
    /// never an archive must not be reported as a broken one.
    #[test]
    fn a_file_that_is_not_an_archive_is_not_called_damaged() {
        let message = ArchiveError::NotAnArchive { path: "/x/photo.zip".into() }.to_string();
        assert!(!message.to_lowercase().contains("damaged"));
        assert!(!message.to_lowercase().contains("corrupt"));
    }
}
