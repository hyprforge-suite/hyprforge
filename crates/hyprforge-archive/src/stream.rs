//! One place that turns a [`Compression`] into a reader or a writer.
//!
//! Four codecs, four crates, four slightly different constructors — and
//! every caller in this crate wants the same two things from them. Left
//! at each call site, the `match` gets copied into the sniffer, the tar
//! reader, the tar writer and the single-file path, and the day a fifth
//! compression arrives is the day three of those four get updated.

use crate::format::Compression;
use std::io::{Read, Write};

/// A boxed reader, because the four decoders are four unrelated types
/// and the tar reader on top of them does not care which it has.
pub type BoxRead<'a> = Box<dyn Read + Send + 'a>;
pub type BoxWrite<'a> = Box<dyn Write + Send + 'a>;

/// Wraps `inner` in the decoder for `compression`.
///
/// [`Compression::None`] hands `inner` straight back rather than
/// wrapping it in an identity decoder, so an uncompressed tar costs
/// nothing extra to read.
pub fn decoder<'a>(compression: Compression, inner: BoxRead<'a>) -> std::io::Result<BoxRead<'a>> {
    Ok(match compression {
        Compression::None => inner,
        Compression::Gzip => Box::new(flate2::read::MultiGzDecoder::new(inner)),
        Compression::Bzip2 => Box::new(bzip2::read::MultiBzDecoder::new(inner)),
        Compression::Xz => Box::new(liblzma::read::XzDecoder::new_multi_decoder(inner)),
        Compression::Zstd => Box::new(zstd::stream::read::Decoder::new(inner)?),
    })
}

/// The compression level every writer here uses.
///
/// Deliberately not each codec's maximum. A file manager compressing a
/// folder is doing it while someone waits, and xz at level 9 on a
/// gigabyte costs minutes and a gigabyte of memory to save a few percent
/// over level 6 — which is also every one of these tools' own default
/// when run from a shell, so an archive made here is the size someone
/// would expect from `tar caf`.
const LEVEL: u32 = 6;

/// Wraps `inner` in the encoder for `compression`.
pub fn encoder<'a>(compression: Compression, inner: BoxWrite<'a>) -> std::io::Result<BoxWrite<'a>> {
    Ok(match compression {
        Compression::None => inner,
        Compression::Gzip => Box::new(flate2::write::GzEncoder::new(
            inner,
            flate2::Compression::new(LEVEL),
        )),
        Compression::Bzip2 => Box::new(bzip2::write::BzEncoder::new(
            inner,
            bzip2::Compression::new(LEVEL),
        )),
        Compression::Xz => Box::new(liblzma::write::XzEncoder::new(inner, LEVEL)),
        // zstd's scale runs to 22 and its own default is 3; 6 is a
        // comparable point on it, not the same number meaning the same
        // thing by coincidence.
        Compression::Zstd => Box::new(
            zstd::stream::write::Encoder::new(inner, LEVEL as i32)?.auto_finish(),
        ),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every codec has to survive the round trip through this module's
    /// own pair, because a mismatched encoder and decoder (gzip out, zlib
    /// in — they differ only in a header) produces a file that looks fine
    /// until something else tries to read it.
    #[test]
    fn every_compression_round_trips_through_its_own_pair() {
        for compression in [
            Compression::None,
            Compression::Gzip,
            Compression::Bzip2,
            Compression::Xz,
            Compression::Zstd,
        ] {
            // Long enough to actually compress, and repetitive so a
            // codec that silently did nothing would stand out.
            let original = "the quick brown fox ".repeat(500).into_bytes();

            let mut packed = Vec::new();
            {
                let mut encoder = encoder(compression, Box::new(&mut packed)).unwrap();
                encoder.write_all(&original).unwrap();
                encoder.flush().unwrap();
            }

            let mut unpacked = Vec::new();
            decoder(compression, Box::new(packed.as_slice()))
                .unwrap()
                .read_to_end(&mut unpacked)
                .unwrap();

            assert_eq!(unpacked, original, "{compression:?} did not round trip");
        }
    }
}
