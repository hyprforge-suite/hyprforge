//! A drop shadow that a partial redraw cannot smear.
//!
//! iced draws a quad's `Shadow` itself, and `iced_tiny_skia` 0.14.1 paints
//! that shadow with **no clip mask** (`engine.rs`, `draw_pixmap(.., None)`).
//! On a renderer that repaints the whole surface every frame — the lock
//! screen's — that is invisible: the shadow lands on pixels that were just
//! cleared. iced's own window compositor does not repaint the whole
//! surface. It diffs the layer stack against the frame the buffer last
//! held and repaints only the rectangles that changed, so a keystroke in
//! the greeter repaints the password field — and the card around it, which
//! intersects that rectangle, is drawn again, shadow and all. The shadow
//! lands outside the rectangle being repainted, on pixels nobody cleared,
//! and darkens them a little more every keystroke: the nested outlines a
//! photo of the greeter showed after a dozen characters.
//!
//! Upstream fixed it on 2026-02-11 (iced `0553559827`, "Clip quad shadows
//! in `tiny-skia` renderer"), after the 0.14 branch; 0.14.1 does not have
//! it. Until a release does, a shadow here is not a `Shadow` at all: it
//! is an image, made once per size with iced's own formula and drawn in a
//! layer beneath the content. Images *are* clipped in 0.14.1, so a partial
//! repaint keeps to its rectangle, and the shadow's whole extent is part
//! of the image's bounds, so a panel that changes size damages everything
//! its old shadow covered.
//!
//! The same widget serves the lock screen, so both hosts keep one look;
//! `an_image_shadow_looks_like_the_one_iced_draws` holds the two side by
//! side at the scale this machine runs at.

use iced_runtime::core::image::{self, Handle};
use iced_runtime::core::layout;
use iced_runtime::core::mouse;
use iced_runtime::core::overlay;
use iced_runtime::core::renderer;
use iced_runtime::core::widget::{self, tree, Operation};
use iced_runtime::core::{
    Clipboard, Element, Event, Layout, Length, Rectangle, Shadow, Shell, Size, Vector,
    Widget,
};
use std::cell::RefCell;

/// How many shadow images are kept. A screen has five kinds of panel and
/// each is one size at a time; the rest is the size a panel had a frame
/// ago, which a resize hands back almost at once.
const KEPT: usize = 16;

/// The largest image a shadow may be, in pixels. A panel cannot be
/// bigger than the window, so this is only ever reached by a theme or a
/// layout gone wrong — and then the panel draws without a shadow rather
/// than allocating whatever it was asked for. Ask what it allocates, not
/// only whether it works.
const MAX_PIXELS: u64 = 4096 * 4096;

/// `content`, with `shadow` drawn beneath it as if `content` were a
/// rounded box of `radius`.
pub fn shadowed<'a, Message, Theme, R>(
    content: impl Into<Element<'a, Message, Theme, R>>,
    radius: f32,
    shadow: Shadow,
) -> Element<'a, Message, Theme, R>
where
    Message: 'a,
    Theme: 'a,
    R: image::Renderer<Handle = Handle> + 'a,
{
    Element::new(Shadowed { content: content.into(), radius, shadow })
}

struct Shadowed<'a, Message, Theme, R> {
    content: Element<'a, Message, Theme, R>,
    radius: f32,
    shadow: Shadow,
}

/// What identifies one shadow image. Quarter-pixel steps, so a layout
/// that lands a hair differently frame to frame reuses the same image
/// rather than minting a new one each time.
#[derive(Clone, Copy, PartialEq, Eq)]
struct Key {
    width: u32,
    height: u32,
    radius: u32,
    blur: u32,
    rgba: [u8; 4],
}

impl Key {
    fn new(size: Size, radius: f32, shadow: &Shadow) -> Key {
        let quarter = |v: f32| (v.max(0.0) * 4.0).round() as u32;
        Key {
            width: quarter(size.width),
            height: quarter(size.height),
            radius: quarter(radius),
            blur: quarter(shadow.blur_radius),
            rgba: shadow.color.into_rgba8(),
        }
    }
}

thread_local! {
    static IMAGES: RefCell<Vec<(Key, Handle)>> = const { RefCell::new(Vec::new()) };
}

/// The image for this shadow, made on first use and kept.
fn image_for(size: Size, radius: f32, shadow: &Shadow) -> Option<Handle> {
    let key = Key::new(size, radius, shadow);
    IMAGES.with(|images| {
        let mut images = images.borrow_mut();
        if let Some(at) = images.iter().position(|(k, _)| *k == key) {
            let entry = images.remove(at);
            let handle = entry.1.clone();
            images.insert(0, entry);
            return Some(handle);
        }
        let (width, height, pixels) = paint(size, radius, shadow)?;
        let handle = Handle::from_rgba(width, height, pixels);
        images.insert(0, (key, handle.clone()));
        images.truncate(KEPT);
        Some(handle)
    })
}

/// The shadow's pixels: one per logical point, covering the box grown by
/// the blur on every side — exactly the area iced's own shadow covers.
///
/// The formula is iced's (`iced_tiny_skia` `engine.rs`, `draw_quad`): a
/// rounded-box signed distance, and an alpha that falls from one half at
/// the box's edge to nothing a blur radius out. Copied rather than
/// approximated so the lock screen, which never had this bug, looks the
/// same after the fix as before it.
fn paint(size: Size, radius: f32, shadow: &Shadow) -> Option<(u32, u32, Vec<u8>)> {
    let blur = shadow.blur_radius.max(0.0);
    if !(size.width > 0.0 && size.height > 0.0 && blur.is_finite() && shadow.color.a > 0.0) {
        return None;
    }
    let width = (size.width + 2.0 * blur).ceil() as u32;
    let height = (size.height + 2.0 * blur).ceil() as u32;
    if width == 0 || height == 0 || u64::from(width) * u64::from(height) > MAX_PIXELS {
        return None;
    }

    // iced clamps every corner to half the box, so a pill's 999 is a
    // semicircle rather than a shape the distance function cannot mean.
    let radius = radius.max(0.0).min(size.width / 2.0).min(size.height / 2.0);
    let (half_w, half_h) = (size.width / 2.0, size.height / 2.0);
    let [r, g, b, a] = shadow.color.into_rgba8();

    let mut pixels = Vec::with_capacity(width as usize * height as usize * 4);
    for y in 0..height {
        for x in 0..width {
            // From the pixel's centre to the box's, in the image's own
            // coordinates: the box sits `blur` in from the image's edge.
            let to_center = Vector::new(
                x as f32 + 0.5 - blur - half_w,
                y as f32 + 0.5 - blur - half_h,
            );
            let distance = rounded_box_sdf(to_center, half_w, half_h, radius).max(0.0);
            let alpha = 1.0 - smoothstep(-blur, blur, distance);
            pixels.extend_from_slice(&[r, g, b, (f32::from(a) * alpha).round() as u8]);
        }
    }
    Some((width, height, pixels))
}

fn smoothstep(a: f32, b: f32, x: f32) -> f32 {
    if b <= a {
        return if x < a { 0.0 } else { 1.0 };
    }
    let x = ((x - a) / (b - a)).clamp(0.0, 1.0);
    x * x * (3.0 - 2.0 * x)
}

fn rounded_box_sdf(to_center: Vector, half_w: f32, half_h: f32, radius: f32) -> f32 {
    let x = (to_center.x.abs() - half_w + radius).max(0.0);
    let y = (to_center.y.abs() - half_h + radius).max(0.0);
    (x * x + y * y).sqrt() - radius
}

impl<Message, Theme, R> Widget<Message, Theme, R> for Shadowed<'_, Message, Theme, R>
where
    R: image::Renderer<Handle = Handle>,
{
    fn tag(&self) -> tree::Tag {
        self.content.as_widget().tag()
    }

    fn state(&self) -> tree::State {
        self.content.as_widget().state()
    }

    fn children(&self) -> Vec<tree::Tree> {
        self.content.as_widget().children()
    }

    fn diff(&self, tree: &mut widget::Tree) {
        self.content.as_widget().diff(tree);
    }

    fn size(&self) -> Size<Length> {
        self.content.as_widget().size()
    }

    fn size_hint(&self) -> Size<Length> {
        self.content.as_widget().size_hint()
    }

    fn layout(&mut self, tree: &mut widget::Tree, renderer: &R, limits: &layout::Limits) -> layout::Node {
        self.content.as_widget_mut().layout(tree, renderer, limits)
    }

    fn draw(
        &self,
        tree: &widget::Tree,
        renderer: &mut R,
        theme: &Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        let bounds = layout.bounds();
        if let Some(handle) = image_for(bounds.size(), self.radius, &self.shadow) {
            let blur = self.shadow.blur_radius.max(0.0);
            let at = Rectangle {
                x: bounds.x + self.shadow.offset.x - blur,
                y: bounds.y + self.shadow.offset.y - blur,
                width: (bounds.width + 2.0 * blur).ceil(),
                height: (bounds.height + 2.0 * blur).ceil(),
            };
            // A layer of its own, before the content's: within one layer
            // `iced_tiny_skia` draws every quad before any image, so a
            // shadow sharing the panel's layer would land on top of it.
            renderer.with_layer(*viewport, |renderer| {
                renderer.draw_image(image::Image::new(handle), at, *viewport);
            });
        }
        renderer.with_layer(*viewport, |renderer| {
            self.content
                .as_widget()
                .draw(tree, renderer, theme, style, layout, cursor, viewport);
        });
    }

    fn update(
        &mut self,
        tree: &mut widget::Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &R,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
        viewport: &Rectangle,
    ) {
        self.content
            .as_widget_mut()
            .update(tree, event, layout, cursor, renderer, clipboard, shell, viewport);
    }

    fn mouse_interaction(
        &self,
        tree: &widget::Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &R,
    ) -> mouse::Interaction {
        self.content
            .as_widget()
            .mouse_interaction(tree, layout, cursor, viewport, renderer)
    }

    fn operate(&mut self, tree: &mut widget::Tree, layout: Layout<'_>, renderer: &R, operation: &mut dyn Operation) {
        self.content.as_widget_mut().operate(tree, layout, renderer, operation);
    }

    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut widget::Tree,
        layout: Layout<'b>,
        renderer: &R,
        viewport: &Rectangle,
        translation: Vector,
    ) -> Option<overlay::Element<'b, Message, Theme, R>> {
        self.content
            .as_widget_mut()
            .overlay(tree, layout, renderer, viewport, translation)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use iced_runtime::core::Color;

    fn shadow(blur: f32) -> Shadow {
        Shadow { color: Color::from_rgba8(16, 22, 30, 0.45), offset: Vector::new(0.0, 20.0), blur_radius: blur }
    }

    #[test]
    fn a_shadow_image_covers_the_box_grown_by_the_blur() {
        let (w, h, pixels) = paint(Size::new(100.0, 40.0), 12.0, &shadow(50.0)).unwrap();
        assert_eq!((w, h), (200, 140));
        assert_eq!(pixels.len(), 200 * 140 * 4);
        // Half strength under the box, nothing at the image's corner.
        let at = |x: usize, y: usize| pixels[(y * w as usize + x) * 4 + 3];
        let full = shadow(50.0).color.into_rgba8()[3];
        assert_eq!(at(100, 70), (f32::from(full) * 0.5).round() as u8);
        assert_eq!(at(0, 0), 0);
    }

    #[test]
    fn a_shadow_the_size_of_a_building_is_not_allocated() {
        assert!(paint(Size::new(1.0e6, 1.0e6), 12.0, &shadow(50.0)).is_none());
        assert!(paint(Size::new(0.0, 40.0), 12.0, &shadow(50.0)).is_none());
        assert!(paint(Size::new(100.0, 40.0), 12.0, &shadow(f32::INFINITY)).is_none());
    }

    #[test]
    fn the_same_panel_reuses_its_shadow_image() {
        let size = Size::new(380.0, 300.0);
        let first = image_for(size, 18.0, &shadow(50.0)).unwrap();
        let again = image_for(Size::new(380.1, 300.0), 18.0, &shadow(50.0)).unwrap();
        assert_eq!(first.id(), again.id(), "a hair's difference minted a new image");
        let other = image_for(Size::new(380.0, 320.0), 18.0, &shadow(50.0)).unwrap();
        assert_ne!(first.id(), other.id());
    }

    /// The lock screen never had the smear — it repaints everything every
    /// frame — so moving its shadows into images must not change how it
    /// looks. One glass panel, drawn by iced with a `Shadow` and then by
    /// `shadowed` with none, at this machine's 1.6 and at 1.0, full
    /// damage both times, over a mid-grey that shows a shadow plainly.
    #[test]
    fn an_image_shadow_looks_like_the_one_iced_draws() {
        use iced_runtime::core::{Border, Pixels};
        use iced_runtime::user_interface::{Cache, UserInterface};
        use iced_widget::container;

        let panel = Size::new(380.0, 300.0);
        let shadow = Shadow { color: Color::from_rgba8(22, 22, 30, 0.45), offset: Vector::new(0.0, 20.0), blur_radius: 50.0 };
        let radius = 18.0;
        let style = move |with_shadow: bool| {
            move |_: &iced_widget::Theme| container::Style {
                background: Some(Color::from_rgba8(22, 22, 30, 0.55).into()),
                border: Border { color: Color::from_rgba8(248, 248, 242, 0.10), width: 1.0, radius: radius.into() },
                shadow: if with_shadow { shadow } else { Shadow::default() },
                ..Default::default()
            }
        };

        for scale in [1.6_f32, 1.0] {
            let size = Size::new(800.0, 600.0);
            let (w, h) = ((size.width * scale) as u32, (size.height * scale) as u32);
            let render = |iced_shadow: bool| {
                let mut renderer = iced_tiny_skia::Renderer::new(iced_runtime::core::Font::DEFAULT, Pixels(16.0));
                let card = container(iced_widget::Space::new())
                    .width(Length::Fixed(panel.width))
                    .height(Length::Fixed(panel.height))
                    .style(style(iced_shadow));
                let content: Element<'_, (), iced_widget::Theme, iced_tiny_skia::Renderer> =
                    if iced_shadow { card.into() } else { shadowed(card, radius, shadow) };
                let root = container(content).center_x(Length::Fill).center_y(Length::Fill);
                let mut ui = UserInterface::build(root, size, Cache::default(), &mut renderer);
                ui.draw(
                    &mut renderer,
                    &iced_widget::Theme::Dark,
                    &renderer::Style { text_color: Color::WHITE },
                    mouse::Cursor::Unavailable,
                );
                let mut pixmap = tiny_skia::Pixmap::new(w, h).unwrap();
                let mut mask = tiny_skia::Mask::new(w, h).unwrap();
                renderer.draw(
                    &mut pixmap.as_mut(),
                    &mut mask,
                    &iced_tiny_skia::graphics::Viewport::with_physical_size(Size::new(w, h), scale),
                    &[Rectangle::with_size(size)],
                    Color::from_rgb8(120, 120, 130),
                );
                pixmap
            };
            let (iced, ours) = (render(true), render(false));
            let worst = iced.data().iter().zip(ours.data()).map(|(a, b)| a.abs_diff(*b)).max().unwrap();
            let shaded = iced.data().iter().zip(render(false).data()).filter(|(a, _)| **a != 120 && **a != 130 && **a != 255).count();
            assert!(shaded > 1000, "nothing was shaded at {scale} — the comparison compared two blanks");
            assert!(worst <= 3, "at scale {scale} the image shadow differs from iced's by up to {worst}/255");
        }
    }
}
