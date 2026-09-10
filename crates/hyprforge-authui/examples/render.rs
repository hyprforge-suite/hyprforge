//! Renders the auth screen to a PNG, so the design can be looked at
//! without locking anything.
use hyprforge_authui::conversation::{Prompt, State};
use hyprforge_look::Theme;
use iced_runtime::core::{Color, Pixels, Rectangle, Size, mouse, renderer::Style};
use iced_runtime::user_interface::{Cache, UserInterface};
use iced_tiny_skia::Renderer;
use iced_tiny_skia::graphics::Viewport;

#[derive(Debug, Clone)]
enum Message {}

/// Where the previews land. Not the repo — they are generated.
fn out_dir() -> std::path::PathBuf {
    std::env::var_os("OUT_PREVIEW_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
}

fn shot(name: &str, state: State, theme: &Theme) {
    let (w, h) = (900u32, 620u32);
    let mut renderer = Renderer::new(hyprforge_authui::screen::font(theme), Pixels(theme.font_size));
    let now = chrono::Local::now();

    let view = hyprforge_authui::screen::view::<Message, Renderer>(&state, "apost", theme, now);
    let mut ui = UserInterface::<Message, iced_widget::Theme, Renderer>::build(
        view,
        Size::new(w as f32, h as f32),
        Cache::default(),
        &mut renderer,
    );
    ui.draw(&mut renderer, &iced_widget::Theme::Dark, &Style { text_color: Color::WHITE }, mouse::Cursor::Unavailable);

    let mut pixmap = tiny_skia::Pixmap::new(w, h).unwrap();
    let mut mask = tiny_skia::Mask::new(w, h).unwrap();
    renderer.draw(
        &mut pixmap.as_mut(),
        &mut mask,
        &Viewport::with_physical_size(Size::new(w, h), 1.0),
        &[Rectangle::with_size(Size::new(w as f32, h as f32))],
        Color::BLACK,
    );
    // iced_tiny_skia writes BGRA (its into_color swaps R and B on
    // purpose, because that is what a Wayland Argb8888 buffer wants).
    // PNG wants RGBA, so this preview swaps back. The lock screen does
    // NOT — for it, BGRA is already the right answer.
    let mut rgba = pixmap.data().to_vec();
    for px in rgba.chunks_exact_mut(4) {
        px.swap(0, 2);
    }
    let path = out_dir().join(name);
    let file = std::fs::File::create(&path).unwrap();
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), w, h);
    enc.set_color(png::ColorType::Rgba);
    enc.write_header().unwrap().write_image_data(&rgba).unwrap();
    println!("wrote {}", path.display());
}

fn main() {
    // A wallpaper if one was named, so the dim and the fit can be seen.
    let theme = Theme {
        wallpaper: std::env::var_os("PREVIEW_WALLPAPER").map(std::path::PathBuf::from),
        ..Theme::default()
    };
    let theme = hyprforge_authui::screen::with_drawable_wallpaper(theme);
    println!("wallpaper in use: {:?}", theme.wallpaper);
    shot("auth-asking.png", State::Asking { prompt: Prompt::secret("Password:"), entered: "hunter2!".into() }, &theme);
    shot("auth-failed.png", State::Failed { reason: "Incorrect password".into() }, &theme);
    shot("auth-working.png", State::Working, &theme);
}
