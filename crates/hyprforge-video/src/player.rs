//! A video playing: mpv on a thread of its own, drawing frames into memory.
//!
//! The window never talks to mpv. It sends [`Command`]s and receives
//! [`Update`]s through a callback — new frames as RGBA, and the playback
//! state (position, length, paused, ended) when it changes — so this
//! crate needs no async runtime and no toolkit, and the photo viewer
//! wraps the callback in whatever channel its runtime likes.
//!
//! # Frames are drawn at the size they are shown
//!
//! mpv's software renderer scales and letterboxes into a buffer the size
//! the window says ([`Command::Size`]), in the window's backdrop colour,
//! so a 4K video in a 1200-pixel pane costs a 1200-pixel frame, not a
//! 4K one: 5.7MB a frame at 1600x900, whatever the file. One buffer is
//! reused for drawing; each delivered frame is one copy of it.
//!
//! # Audio
//!
//! mpv plays the sound itself, through whatever the desktop's audio
//! server is. Dropping the [`Player`] stops it — which is what moving to
//! the next picture must do.

use crate::mpv::{Api, Happened, Mpv, MpvError, FORMAT_DOUBLE, FORMAT_FLAG, FORMAT_INT64};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// A drawn frame: RGBA, eight bits a channel, rows packed tight.
#[derive(Clone, PartialEq, Eq)]
pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

impl std::fmt::Debug for Frame {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Frame({}x{})", self.width, self.height)
    }
}

/// Where playback is.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Playback {
    /// Seconds in.
    pub position: f64,
    /// Seconds long, once mpv knows.
    pub duration: Option<f64>,
    pub paused: bool,
    /// At the end, holding the last frame (`keep-open`).
    pub ended: bool,
    pub muted: bool,
    /// The video's own display size, once known.
    pub video_size: Option<(u32, u32)>,
}

/// What the player tells the window.
#[derive(Debug, Clone)]
pub enum Update {
    Frame(Arc<Frame>),
    Playback(Playback),
    /// The file could not be played.
    Failed(String),
}

/// What the window tells the player.
#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    Pause(bool),
    TogglePause,
    /// Seconds; relative to where playback is, or from the start.
    Seek { seconds: f64, relative: bool },
    Mute(bool),
    /// The size to draw frames at, physical pixels.
    Size(u32, u32),
    /// Back to the start and playing — what play at the end means.
    Restart,
}

/// How a player starts.
#[derive(Debug, Clone)]
pub struct Options {
    /// The colour around a letterboxed frame: the window's backdrop, so a
    /// video sits on the same plane a photograph does.
    pub background: [u8; 3],
    /// The first size to draw at, until the window says otherwise.
    pub size: (u32, u32),
    pub paused: bool,
    pub muted: bool,
}

enum Msg {
    Wake,
    Cmd(Command),
    Quit,
}

/// A playing video. Dropping it stops playback and frees mpv.
pub struct Player {
    tx: Sender<Msg>,
    thread: Option<JoinHandle<()>>,
}

impl Player {
    /// Starts playing `path`. Fails at once when libmpv is missing — that
    /// much is known before any thread starts — and reports anything the
    /// file itself gets wrong as [`Update::Failed`] later.
    pub fn open(
        path: &Path,
        options: Options,
        on_update: impl FnMut(Update) + Send + 'static,
    ) -> Result<Player, MpvError> {
        let api = Api::load()?;
        let (tx, rx) = mpsc::channel();
        let wake = tx.clone();
        let path = path.to_path_buf();
        let thread = std::thread::Builder::new()
            .name("hyprforge-video".into())
            .spawn(move || run(api, path, options, wake, rx, on_update))
            .map_err(|e| MpvError::Failed(format!("could not start the player thread: {e}")))?;
        Ok(Player { tx, thread: Some(thread) })
    }

    pub fn send(&self, command: Command) {
        // A player whose thread has ended has nothing to be told.
        let _ = self.tx.send(Msg::Cmd(command));
    }
}

impl Drop for Player {
    fn drop(&mut self) {
        let _ = self.tx.send(Msg::Quit);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// The rounding that keeps every row 64-byte aligned for mpv's fast path:
/// a width that is a multiple of 16 pixels is a stride that is a multiple
/// of 64 bytes, so no row needs padding or repacking.
pub fn draw_size(width: u32, height: u32) -> (u32, u32) {
    ((width.max(16) / 16) * 16, height.max(1))
}

/// A byte buffer whose start is 64-byte aligned — mpv's own advice for
/// its fast path — found by offset into a slightly larger `Vec`, which
/// needs no `unsafe`.
struct Buffer {
    bytes: Vec<u8>,
    offset: usize,
}

impl Buffer {
    fn new(len: usize) -> Buffer {
        let bytes = vec![0u8; len + 63];
        let offset = (64 - (bytes.as_ptr() as usize % 64)) % 64;
        Buffer { bytes, offset }
    }

    fn slice(&mut self, len: usize) -> &mut [u8] {
        &mut self.bytes[self.offset..self.offset + len]
    }
}

/// `rgb0` rows to tight RGBA: mpv leaves the fourth byte as garbage
/// ("often 0, but not necessarily" — render.h), and iced would read it as
/// transparency.
pub fn to_rgba(rgb0: &[u8], width: u32, height: u32, stride: usize) -> Vec<u8> {
    let row = width as usize * 4;
    let mut out = Vec::with_capacity(row * height as usize);
    for y in 0..height as usize {
        out.extend_from_slice(&rgb0[y * stride..y * stride + row]);
    }
    for alpha in out.iter_mut().skip(3).step_by(4) {
        *alpha = 255;
    }
    out
}

/// How often a moving position is reported: fine enough for a seek bar,
/// coarse enough not to wake the window every frame for it.
const POSITION_EVERY: Duration = Duration::from_millis(200);

fn run(
    api: Arc<Api>,
    path: PathBuf,
    options: Options,
    wake: Sender<Msg>,
    rx: mpsc::Receiver<Msg>,
    mut on_update: impl FnMut(Update),
) {
    if let Err(e) = play(api, &path, options, wake, &rx, &mut on_update) {
        on_update(Update::Failed(e.to_string()));
    }
}

fn play(
    api: Arc<Api>,
    path: &Path,
    options: Options,
    wake: Sender<Msg>,
    rx: &mpsc::Receiver<Msg>,
    on_update: &mut impl FnMut(Update),
) -> Result<(), MpvError> {
    let mut mpv = Mpv::new(api)?;
    let [r, g, b] = options.background;
    for (name, value) in [
        // No window of mpv's own: frames come out through the renderer.
        ("vo", "libmpv"),
        // Decode on the GPU where it is safe, copy back, and fall back to
        // software quietly where it is not.
        ("hwdec", "auto-safe"),
        // Hold the last frame at the end instead of going idle and blank.
        ("keep-open", "yes"),
        // mpv's own user interface and config are not this window's.
        ("config", "no"),
        ("osc", "no"),
        ("input-default-bindings", "no"),
        ("input-vo-keyboard", "no"),
        ("terminal", "no"),
        ("audio-client-name", "hyprforge-photos"),
        ("background-color", &format!("#{r:02x}{g:02x}{b:02x}")),
        ("pause", if options.paused { "yes" } else { "no" }),
        ("mute", if options.muted { "yes" } else { "no" }),
    ] {
        mpv.set_option(name, value)?;
    }
    mpv.initialize()?;
    let mut render = mpv.software_renderer()?;
    let (w1, w2) = (wake.clone(), wake);
    mpv.on_wakeup(move || {
        let _ = w1.send(Msg::Wake);
    });
    render.on_update(move || {
        let _ = w2.send(Msg::Wake);
    });
    for (name, format) in [
        ("time-pos", FORMAT_DOUBLE),
        ("duration", FORMAT_DOUBLE),
        ("pause", FORMAT_FLAG),
        ("eof-reached", FORMAT_FLAG),
        ("mute", FORMAT_FLAG),
        ("dwidth", FORMAT_INT64),
        ("dheight", FORMAT_INT64),
    ] {
        mpv.observe(name, format)?;
    }
    let file = path.to_str().ok_or_else(|| MpvError::Failed("the path is not UTF-8".into()))?;
    mpv.command(&["loadfile", file])?;

    let (mut width, mut height) = draw_size(options.size.0, options.size.1);
    let mut buffer = Buffer::new(width as usize * 4 * height as usize);
    let mut playback = Playback { paused: options.paused, muted: options.muted, ..Playback::default() };
    let mut reported = playback.clone();
    let mut last_position = Instant::now() - POSITION_EVERY;
    let mut size_changed = false;

    'outer: loop {
        match rx.recv_timeout(Duration::from_millis(250)) {
            Ok(Msg::Quit) | Err(RecvTimeoutError::Disconnected) => break,
            Ok(Msg::Cmd(command)) => match command {
                Command::Pause(on) => mpv.set_flag("pause", on)?,
                Command::TogglePause => mpv.command(&["cycle", "pause"])?,
                Command::Seek { seconds, relative } => {
                    let mode = if relative { "relative" } else { "absolute" };
                    // A seek past either end is mpv's to clamp; an error
                    // here (nothing loaded yet) is not worth failing for.
                    let _ = mpv.command(&["seek", &format!("{seconds:.3}"), mode]);
                }
                Command::Mute(on) => mpv.set_flag("mute", on)?,
                Command::Size(w, h) => {
                    let (w, h) = draw_size(w, h);
                    if (w, h) != (width, height) {
                        (width, height) = (w, h);
                        buffer = Buffer::new(width as usize * 4 * height as usize);
                        size_changed = true;
                    }
                }
                Command::Restart => {
                    let _ = mpv.command(&["seek", "0", "absolute"]);
                    mpv.set_flag("pause", false)?;
                }
            },
            Ok(Msg::Wake) | Err(RecvTimeoutError::Timeout) => {}
        }

        loop {
            match mpv.next_event(0.0) {
                Happened::Nothing => break,
                Happened::Shutdown => break 'outer,
                Happened::EndFile { error: true } => {
                    on_update(Update::Failed("mpv couldn't play this file".into()));
                }
                Happened::Double(name, value) => match name.as_str() {
                    "time-pos" => playback.position = value.max(0.0),
                    "duration" => playback.duration = Some(value).filter(|d| *d > 0.0),
                    _ => {}
                },
                Happened::Flag(name, on) => match name.as_str() {
                    "pause" => playback.paused = on,
                    "eof-reached" => playback.ended = on,
                    "mute" => playback.muted = on,
                    _ => {}
                },
                Happened::Int(name, value) => {
                    let (w, h) = playback.video_size.unwrap_or((0, 0));
                    match name.as_str() {
                        "dwidth" => playback.video_size = Some((value.max(0) as u32, h)),
                        "dheight" => playback.video_size = Some((w, value.max(0) as u32)),
                        _ => {}
                    }
                }
                Happened::FileLoaded | Happened::EndFile { .. } | Happened::Other => {}
            }
        }

        // Everything but the position is reported at once; the position,
        // which moves every frame, at most every `POSITION_EVERY`.
        let moved_only = Playback { position: reported.position, ..playback.clone() } == reported;
        if playback != reported && (!moved_only || last_position.elapsed() >= POSITION_EVERY) {
            on_update(Update::Playback(playback.clone()));
            reported = playback.clone();
            last_position = Instant::now();
        }

        if render.frame_waiting() || size_changed {
            size_changed = false;
            let stride = width as usize * 4;
            let len = stride * height as usize;
            render.draw(buffer.slice(len), width, height, stride)?;
            let pixels = to_rgba(buffer.slice(len), width, height, stride);
            on_update(Update::Frame(Arc::new(Frame { width, height, pixels })));
        }
    }
    // The render context before the handle, as render.h requires.
    drop(render);
    drop(mpv);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_drawn_width_is_always_a_whole_number_of_64_byte_rows() {
        for w in [1, 15, 16, 17, 1599, 1600, 1601, 3840] {
            let (dw, _) = draw_size(w, 900);
            assert_eq!((dw as usize * 4) % 64, 0, "{w} drew at {dw}");
            assert!(dw <= w.max(16), "{w} grew to {dw}");
        }
    }

    #[test]
    fn the_buffer_starts_on_a_64_byte_boundary() {
        for len in [64, 1000, 5_760_000] {
            let mut b = Buffer::new(len);
            assert_eq!(b.slice(len).as_ptr() as usize % 64, 0);
            assert_eq!(b.slice(len).len(), len);
        }
    }

    /// Against the real libmpv: an orange clip, played, delivers a frame
    /// whose middle is orange, and reports the clip's length. Needs mpv and
    /// ffmpeg, which tier 1 does not, so it runs by hand:
    /// `cargo test -p hyprforge-video -- --ignored`.
    #[test]
    #[ignore]
    fn a_played_clip_delivers_its_own_pixels() {
        let dir = tempfile::tempdir().unwrap();
        let clip = dir.path().join("orange.mp4");
        let made = std::process::Command::new("ffmpeg")
            .args(["-v", "error", "-f", "lavfi", "-i", "color=0xff8800:s=320x180:d=3", "-pix_fmt", "yuv420p"])
            .arg(&clip)
            .status();
        if !made.is_ok_and(|s| s.success()) {
            eprintln!("HYPRFORGE-SKIP: ffmpeg could not make the fixture");
            return;
        }
        let (tx, rx) = mpsc::channel();
        let options = Options { background: [25, 26, 33], size: (320, 180), paused: false, muted: true };
        let player = match Player::open(&clip, options, move |u| {
            let _ = tx.send(u);
        }) {
            Ok(p) => p,
            Err(MpvError::Missing) => {
                eprintln!("HYPRFORGE-SKIP: libmpv is not installed");
                return;
            }
            Err(e) => panic!("{e}"),
        };
        let deadline = Instant::now() + Duration::from_secs(10);
        let (mut frame, mut duration) = (None, None);
        while Instant::now() < deadline && (frame.is_none() || duration.is_none()) {
            match rx.recv_timeout(Duration::from_millis(200)) {
                Ok(Update::Frame(f)) => frame = Some(f),
                Ok(Update::Playback(p)) => duration = duration.or(p.duration),
                Ok(Update::Failed(e)) => panic!("{e}"),
                Err(_) => {}
            }
        }
        drop(player);
        let frame = frame.expect("no frame arrived");
        assert_eq!((frame.width, frame.height), (320, 180));
        let i = ((90 * 320 + 160) * 4) as usize;
        let (r, g, b, a) = (frame.pixels[i], frame.pixels[i + 1], frame.pixels[i + 2], frame.pixels[i + 3]);
        assert!(r > 200 && (100..180).contains(&g) && b < 60 && a == 255, "middle pixel {r},{g},{b},{a}");
        let d = duration.expect("no duration reported");
        assert!((d - 3.0).abs() < 0.2, "duration {d}");
    }

    /// mpv's fourth byte is garbage; left alone, iced reads it as alpha
    /// and a video with a zero there draws as nothing at all.
    #[test]
    fn every_pixel_handed_on_is_opaque_whatever_mpv_left_in_the_fourth_byte() {
        let rgb0 = [10, 20, 30, 0, 40, 50, 60, 7, /* padding */ 99, 99, 99, 99];
        let rgba = to_rgba(&rgb0, 2, 1, 12);
        assert_eq!(rgba, [10, 20, 30, 255, 40, 50, 60, 255]);
    }
}
