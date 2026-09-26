//! A video's thumbnail: its first *real* frame.
//!
//! The first frame of most videos is black — a fade in, a camera's first
//! exposure, a title card's lead-in — and a grid of black squares says
//! nothing about which video is which. So ffmpeg decodes from the start
//! and keeps the first frame that is not mostly dark: `blackframe` scores
//! every frame (`amount=0`, so every frame gets a score), and `metadata`
//! selects the first scoring under [`DARK_PERCENT`]. A video dark from end
//! to end — a night sky, a screen recording of a terminal — gets its
//! plain first frame rather than nothing.
//!
//! The search reads at most [`SEARCH_SECONDS`] of the file, so a long video
//! that never lightens costs a bounded amount of decoding, and every run
//! is bounded in time by `hyprforge_process::TIMEOUT`, like every other
//! external program in this suite.
//!
//! One function for the file manager and the photo viewer, because they
//! share one thumbnail cache: two apps picking different frames for the
//! same file would each find the other's there and show it.

use std::ffi::OsString;
use std::path::Path;
use std::process::Command;

/// A frame this dark or darker (percent of its pixels under ffmpeg's
/// default black threshold) is not the one to show.
pub const DARK_PERCENT: u32 = 90;
/// How much of the file is searched for a frame that is not dark.
pub const SEARCH_SECONDS: u32 = 20;

/// Why there is no verdict about the file — which a cache must not record
/// as the file's failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unavailable {
    /// ffmpeg is not installed: the one case worth telling the user about.
    Missing,
    /// Did not finish in time, or could not be started.
    Busy,
}

/// The first frame of `path` that is not mostly dark, as PNG bytes fitted
/// inside an `edge`-pixel square. `Ok(None)` when ffmpeg ran and found no
/// frame at all — not a video, or a damaged one.
pub fn first_real_frame(path: &Path, edge: u32) -> Result<Option<Vec<u8>>, Unavailable> {
    if let Some(png) = run(Command::new("ffmpeg").args(arguments(path, edge, true)))? {
        return Ok(Some(png));
    }
    // Dark all the way through the search: the plain first frame.
    run(Command::new("ffmpeg").args(arguments(path, edge, false)))
}

/// The ffmpeg command line, as data so a test can read it.
pub fn arguments(path: &Path, edge: u32, skip_dark: bool) -> Vec<OsString> {
    let fit = format!("scale={edge}:{edge}:force_original_aspect_ratio=decrease");
    let filter = if skip_dark {
        format!(
            "blackframe=amount=0,metadata=mode=select:key=lavfi.blackframe.pblack:value={DARK_PERCENT}:function=less,{fit}"
        )
    } else {
        fit
    };
    let mut args: Vec<OsString> = vec!["-v".into(), "error".into()];
    if skip_dark {
        args.extend(["-t".into(), SEARCH_SECONDS.to_string().into()]);
    }
    args.extend(["-i".into(), path.as_os_str().to_owned()]);
    args.extend(
        ["-an", "-sn", "-frames:v", "1", "-vf"].iter().map(OsString::from).chain([filter.into()]).chain(
            ["-f", "image2pipe", "-c:v", "png", "-"].iter().map(OsString::from),
        ),
    );
    args
}

fn run(command: &mut Command) -> Result<Option<Vec<u8>>, Unavailable> {
    match hyprforge_process::output(command, hyprforge_process::TIMEOUT) {
        Ok(out) if out.status.success() && !out.stdout.is_empty() => Ok(Some(out.stdout)),
        Ok(_) => Ok(None),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Err(Unavailable::Missing),
        Err(e) => {
            tracing::debug!(error = %e, "ffmpeg did not finish");
            Err(Unavailable::Busy)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn joined(args: &[OsString]) -> String {
        args.iter().map(|a| a.to_string_lossy().into_owned()).collect::<Vec<_>>().join(" ")
    }

    #[test]
    fn the_search_skips_dark_frames_within_a_bounded_stretch() {
        let a = joined(&arguments(Path::new("/v/clip.mp4"), 128, true));
        assert!(a.contains("-t 20 -i /v/clip.mp4"), "{a}");
        assert!(a.contains("blackframe=amount=0,metadata=mode=select:key=lavfi.blackframe.pblack:value=90:function=less"), "{a}");
        assert!(a.contains("-frames:v 1"), "{a}");
    }

    /// Fitted inside a square, so a portrait video is as bounded as a
    /// landscape one — the cache's 128-pixel `normal` size either way.
    #[test]
    fn the_frame_is_fitted_inside_a_square() {
        let a = joined(&arguments(Path::new("/v/clip.mp4"), 128, false));
        assert!(a.contains("scale=128:128:force_original_aspect_ratio=decrease"), "{a}");
        assert!(!a.contains("blackframe") && !a.contains("-t "), "{a}");
    }

    /// Against the real ffmpeg: a clip that opens on two seconds of black
    /// and then turns orange must be thumbnailed orange. Not in tier 1 —
    /// it needs ffmpeg, which tier 1 does not — so run it by hand:
    /// `cargo test -p hyprforge-video -- --ignored`.
    #[test]
    #[ignore]
    fn a_video_that_fades_in_is_thumbnailed_after_the_fade() {
        let dir = tempfile::tempdir().unwrap();
        let clip = dir.path().join("fade.mp4");
        let made = Command::new("ffmpeg")
            .args(["-v", "error", "-f", "lavfi", "-i", "color=black:s=160x90:d=2", "-f", "lavfi", "-i"])
            .arg("color=0xff8800:s=160x90:d=2")
            .args(["-filter_complex", "[0][1]concat=n=2:v=1:a=0", "-pix_fmt", "yuv420p"])
            .arg(&clip)
            .status();
        if !made.is_ok_and(|s| s.success()) {
            eprintln!("HYPRFORGE-SKIP: ffmpeg could not make the fixture");
            return;
        }
        let png = first_real_frame(&clip, 128).unwrap().expect("a frame");
        let decoder = png::Decoder::new(std::io::Cursor::new(png));
        let mut reader = decoder.read_info().unwrap();
        let mut buf = vec![0; reader.output_buffer_size().unwrap()];
        let info = reader.next_frame(&mut buf).unwrap();
        let px = &buf[..info.buffer_size()];
        let channels = info.color_type.samples();
        let (r, g, b) = (px[0], px[1], px[2]);
        assert!(r > 200 && g > 100 && b < 60, "the thumbnail is {r},{g},{b} ({channels} channels), not the orange after the fade");
    }
}
