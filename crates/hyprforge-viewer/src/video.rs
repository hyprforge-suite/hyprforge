//! A playing video, as a piece of a window: the player, its latest
//! frame, and the bar beneath it — play, the clock, the seek bar, sound.
//!
//! Media's viewer and Files' Quick Look both show one, and showing it two
//! ways would be two players to keep right: the scrubbing below was found
//! and fixed once, in Media, and lives here now so Quick Look has it too.
//!
//! The pane owns the [`Player`] and everything it reports; the window
//! hands it [`VideoMessage`]s and draws [`VideoPane::film`] and
//! [`VideoPane::bar`] where it wants them, with whatever else of its own
//! around them (Media's arrows, Quick Look's card). Dropping the pane
//! stops the video: the player's thread ends with it.
//!
//! # Scrubbing
//!
//! While the bar is dragged the picture follows by fast keyframe seeks —
//! the player collapses a run of them into the last — and lands exactly
//! where it is let go. The bar then holds where it was let go until mpv
//! reports a position near it: mpv's last report is from before the seek,
//! and showing it would snap the handle back for a moment before it
//! jumped forward again.

use hyprforge_ui::theme::{self, spacing, surface, FontScale};
use hyprforge_ui::widgets::{meta_text, segment_style, SegmentLook};
use hyprforge_video::{Command, Frame, MpvError, Options, Playback, Player, Update};
use iced::widget::{button, container, mouse_area, row, stack, text, Space};
use iced::{Element, Length, Size, Task};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// How close mpv's reported position must come to where the bar was let
/// go before the bar follows mpv again, and how long it waits at most.
const SEEK_SETTLE_SECONDS: f64 = 0.75;
const SEEK_SETTLE_WAIT: Duration = Duration::from_millis(1500);

/// What the pane is told.
#[derive(Debug, Clone, PartialEq)]
pub enum VideoMessage {
    /// From the player, tagged with the pane's generation so a report from
    /// a video since replaced is dropped.
    Player(u64, Update),
    /// The seek bar, dragged to here.
    SeekTo(f64),
    /// The seek bar, let go.
    SeekRelease,
    /// A click on the picture, the play button, or Space where the window
    /// gives Space to the video.
    PlayPause,
    ToggleMute,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Seeking {
    target: f64,
    dragging: bool,
    since: Instant,
}

/// A video in a window.
pub struct VideoPane {
    path: PathBuf,
    generation: u64,
    /// `None` when it could not start — `error` says why.
    player: Option<Player>,
    /// The latest frame and its serial, drawn by [`crate::film::FilmProgram`].
    frame: Option<(Arc<Frame>, u64)>,
    playback: Playback,
    error: Option<String>,
    seeking: Option<Seeking>,
    /// The pane's size in physical pixels, as last told.
    pixels: (u32, u32),
}

impl std::fmt::Debug for VideoPane {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VideoPane").field("path", &self.path).field("generation", &self.generation).finish_non_exhaustive()
    }
}

/// The colour the player paints behind a picture that does not fill the
/// pane: the theme's root surface, so the bars are the window's and not
/// mpv's black.
fn backdrop() -> [u8; 3] {
    let root = surface::root();
    let byte = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    [byte(root.r), byte(root.g), byte(root.b)]
}

/// What a person is told when mpv is not there, and what to do instead.
/// `alternative` is the window's own way out ("Shift+Enter opens this one
/// in another player"), or empty.
pub fn missing_mpv(alternative: &str) -> String {
    let mut said = "Videos play through mpv, which isn't installed here.".to_string();
    if !alternative.is_empty() {
        said.push(' ');
        said.push_str(alternative);
    }
    said
}

impl VideoPane {
    /// Starts playing `path`, drawn at `pixels` (the pane's physical size)
    /// until the video's own shape is known. `generation` tells this video
    /// from the one before it; the window bumps it for each. The task
    /// carries the player's reports for as long as the player lives.
    ///
    /// A machine without libmpv gets a pane that says so — `alternative`
    /// is added to the sentence — and everything else still works.
    pub fn open(
        path: PathBuf,
        generation: u64,
        pixels: (u32, u32),
        paused: bool,
        muted: bool,
        alternative: &str,
    ) -> (VideoPane, Task<VideoMessage>) {
        let options = Options { background: backdrop(), size: pixels, paused, muted };
        let (tx, mut rx) = iced::futures::channel::mpsc::unbounded();
        let mut pane = VideoPane {
            path: path.clone(),
            generation,
            player: None,
            frame: None,
            playback: Playback::default(),
            error: None,
            seeking: None,
            pixels,
        };
        match Player::open(&path, options, move |update| {
            let _ = tx.unbounded_send(update);
        }) {
            Ok(player) => {
                pane.player = Some(player);
                // Until the player is dropped and its thread lets go of
                // the sender, which ends the stream.
                let stream = iced::futures::stream::poll_fn(move |cx| {
                    use iced::futures::StreamExt;
                    rx.poll_next_unpin(cx)
                });
                (pane, Task::run(stream, move |update| VideoMessage::Player(generation, update)))
            }
            Err(e) => {
                pane.error = Some(match e {
                    MpvError::Missing => missing_mpv(alternative),
                    other => other.to_string(),
                });
                (pane, Task::none())
            }
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn playback(&self) -> &Playback {
        &self.playback
    }

    /// Why it is not playing, if it is not.
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// Whether a player is running — false when mpv could not start.
    pub fn playing(&self) -> bool {
        self.player.is_some()
    }

    /// Whether a frame can be drawn yet: one has arrived, and the video's
    /// shape is known — until then mpv draws black pre-roll frames the
    /// size of the pane, and a black pane reads as a broken video.
    pub fn has_picture(&self) -> bool {
        self.frame.is_some() && shaped(&self.playback)
    }

    pub fn update(&mut self, message: VideoMessage) {
        match message {
            VideoMessage::Player(generation, _) if generation != self.generation => {}
            VideoMessage::Player(_, Update::Frame(frame)) => {
                let serial = self.frame.as_ref().map_or(0, |(_, s)| s + 1);
                self.frame = Some((frame, serial));
            }
            VideoMessage::Player(_, Update::Playback(playback)) => {
                let first_size = !shaped(&self.playback) && shaped(&playback);
                if let Some(seek) = self.seeking.filter(|s| !s.dragging) {
                    if (playback.position - seek.target).abs() < SEEK_SETTLE_SECONDS
                        || seek.since.elapsed() > SEEK_SETTLE_WAIT
                    {
                        self.seeking = None;
                    }
                }
                self.playback = playback;
                // Now the shape is known, frames can be drawn at it and
                // mpv adds no bars of its own.
                if first_size {
                    self.send_size();
                }
            }
            VideoMessage::Player(_, Update::Failed(e)) => self.error = Some(e),
            VideoMessage::SeekTo(seconds) => {
                self.seeking = Some(Seeking { target: seconds, dragging: true, since: Instant::now() });
                self.send(Command::Seek { seconds, relative: false, exact: false });
            }
            VideoMessage::SeekRelease => {
                if let Some(seek) = self.seeking {
                    self.send(Command::Seek { seconds: seek.target, relative: false, exact: true });
                    self.seeking = Some(Seeking { dragging: false, since: Instant::now(), ..seek });
                }
            }
            VideoMessage::PlayPause => {
                let command = if self.playback.ended { Command::Restart } else { Command::TogglePause };
                self.send(command);
            }
            VideoMessage::ToggleMute => self.send(Command::ToggleMute),
        }
    }

    /// A relative seek, exact — the arrow keys' five seconds.
    pub fn seek_by(&self, seconds: f64) {
        self.send(Command::Seek { seconds, relative: true, exact: true });
    }

    fn send(&self, command: Command) {
        if let Some(player) = &self.player {
            player.send(command);
        }
    }

    /// The pane changed size. Frames are drawn at the video's own shape
    /// fitted inside it, once the shape is known, so mpv letterboxes
    /// nothing — its bars are black whatever it is told — and the themed
    /// pane shows around the picture instead.
    pub fn resize(&mut self, pixels: (u32, u32)) {
        self.pixels = pixels;
        self.send_size();
    }

    fn send_size(&self) {
        let (w, h) = fitted_pixels(self.pixels, self.playback.video_size);
        self.send(Command::Size(w, h));
    }

    /// The picture, `size` logical pixels, and a click on it plays or
    /// pauses. `None` until there is a picture to draw — see
    /// [`Self::has_picture`] — so the window shows its own "Loading…" or
    /// the error meanwhile.
    pub fn film(&self, size: Size, scale: FontScale) -> Option<Element<'_, VideoMessage>> {
        let (frame, serial) = self.frame.as_ref().filter(|_| shaped(&self.playback))?;
        let [r, g, b] = backdrop();
        let film = iced::widget::shader(crate::film::FilmProgram { frame: frame.clone(), serial: *serial, backdrop: [r, g, b, 255] })
            .width(Length::Fixed(size.width))
            .height(Length::Fixed(size.height));
        // Behind the film, for iced's software renderer, where a shader
        // draws nothing; with the GPU the film's own backdrop covers it.
        let fallback = container(meta_text(
            "Videos need GPU rendering, and this window is drawing without it.",
            theme::BASE_TEXT_SIZE,
            scale,
        ))
        .center(Length::Fill)
        .padding(spacing::LG)
        .style(|_t: &iced::Theme| container::Style {
            background: Some(surface::root().into()),
            ..container::Style::default()
        });
        Some(mouse_area(stack![fallback, film]).on_press(VideoMessage::PlayPause).into())
    }

    /// Play, the clock, the seek bar, sound — on a floating strip, at most
    /// 720 logical pixels wide.
    pub fn bar(&self, scale: FontScale) -> Element<'_, VideoMessage> {
        let p = &self.playback;
        let duration = p.duration.unwrap_or(0.0);
        let at = self.seeking.map_or(p.position, |s| s.target).min(duration.max(0.0));
        let play_label = if p.ended {
            "\u{21BA}"
        } else if p.paused {
            "\u{25B6}"
        } else {
            "\u{275A}\u{275A}"
        };
        let small = |label: &'static str, message: VideoMessage, lit: bool| -> Element<'_, VideoMessage> {
            button(text(label).size(scale.apply(hyprforge_ui::density::META_TEXT_BASE)))
                .padding([3.0, 10.0])
                .on_press(message)
                .style(segment_style(SegmentLook::Quiet, lit))
                .into()
        };
        let clock = text(format!("{} / {}", clock(at), clock(duration)))
            .font(theme::mono_font())
            .size(scale.apply(hyprforge_ui::density::META_TEXT_BASE * 0.85))
            .color(theme::text_dim());
        let seek: Element<'_, VideoMessage> = if duration > 0.0 {
            iced::widget::slider(0.0..=duration, at, VideoMessage::SeekTo)
                .step(0.1)
                .on_release(VideoMessage::SeekRelease)
                .style(hyprforge_ui::widgets::slider_style)
                .width(Length::Fill)
                .into()
        } else {
            Space::new().width(Length::Fill).into()
        };
        container(
            row![
                small(play_label, VideoMessage::PlayPause, false),
                clock,
                seek,
                small(if p.muted { "Muted" } else { "Sound" }, VideoMessage::ToggleMute, p.muted),
            ]
            .spacing(spacing::SM)
            .align_y(iced::Alignment::Center),
        )
        .padding([4, 8])
        .max_width(scale.apply(720.0))
        .style(|_t: &iced::Theme| floating())
        .into()
    }
}

/// The video's shape is known: both sides — `(1280, 0)` is not a shape,
/// and width and height arrive as separate properties.
fn shaped(p: &Playback) -> bool {
    p.video_size.is_some_and(|(w, h)| w > 0 && h > 0)
}

/// What the player draws frames at in a pane of `pane` physical pixels:
/// the video's shape fitted inside, or the pane before the shape is known.
pub fn fitted_pixels(pane: (u32, u32), video: Option<(u32, u32)>) -> (u32, u32) {
    let (pw, ph) = pane;
    let (w, h) = match video.filter(|(w, h)| *w > 0 && *h > 0) {
        Some((vw, vh)) => {
            let k = (pw as f32 / vw as f32).min(ph as f32 / vh as f32);
            ((vw as f32 * k).round() as u32, (vh as f32 * k).round() as u32)
        }
        None => (pw, ph),
    };
    (w.max(16), h.max(1))
}

/// `m:ss`, or `h:mm:ss` past an hour.
pub fn clock(seconds: f64) -> String {
    let total = seconds.max(0.0).floor() as u64;
    let (h, m, s) = (total / 3600, (total / 60) % 60, total % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

/// The floating strip's look: the window's raised surface, rounded, with
/// the card's border — a control over the picture, not part of it.
fn floating() -> container::Style {
    container::Style {
        background: Some(surface::sidebar().into()),
        border: iced::Border {
            color: surface::card_border(),
            width: 1.0,
            radius: hyprforge_ui::density::inner_radius().into(),
        },
        ..container::Style::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_video_is_drawn_at_its_own_shape_inside_the_pane() {
        assert_eq!(fitted_pixels((1600, 900), Some((1920, 1080))), (1600, 900));
        assert_eq!(fitted_pixels((1600, 1600), Some((1920, 1080))), (1600, 900));
        assert_eq!(fitted_pixels((800, 600), None), (800, 600), "the pane until the shape is known");
        assert_eq!(fitted_pixels((800, 600), Some((1280, 0))), (800, 600), "half a shape is no shape");
    }

    #[test]
    fn the_clock_says_hours_only_when_there_are_some() {
        assert_eq!(clock(65.9), "1:05", "a second is shown once it has passed, as Media always did");
        assert_eq!(clock(3725.0), "1:02:05");
        assert_eq!(clock(-3.0), "0:00");
    }

    #[test]
    fn mpv_missing_says_what_to_do_instead() {
        assert_eq!(missing_mpv(""), "Videos play through mpv, which isn't installed here.");
        assert!(missing_mpv("Enter opens it in another player.").ends_with("another player."));
    }
}
