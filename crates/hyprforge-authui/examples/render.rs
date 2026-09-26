//! Renders the auth screen to PNGs — one per state in the lock screen
//! mockup — so the design can be looked at without locking anything.
//!
//! `PREVIEW_WALLPAPER` names a wallpaper, `PREVIEW_THEME` a `lock.toml`
//! to read the rest of the look from, and `OUT_PREVIEW_DIR` where the
//! files land. `PREVIEW_SCALE` renders at a fractional scale, the way
//! the lock does on a scaled output.
use hyprforge_authui::conversation::{Prompt, State};
use hyprforge_authui::scene::{
    self, Battery, Fingerprint, Media, Mode, NotificationCount, PowerAction, PowerMenu, Role, Scene,
    Status,
};
use hyprforge_look::Theme;
use iced_runtime::core::{Color, Pixels, Rectangle, Size, mouse, renderer::Style};
use iced_runtime::user_interface::{Cache, UserInterface};
use iced_tiny_skia::Renderer;
use iced_tiny_skia::graphics::Viewport;

/// Where the previews land. Not the repo — they are generated.
fn out_dir() -> std::path::PathBuf {
    std::env::var_os("OUT_PREVIEW_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
}

fn render(name: &str, scene: Scene<'_>, (w, h): (u32, u32)) {
    let scale: f32 = std::env::var("PREVIEW_SCALE").ok().and_then(|s| s.parse().ok()).unwrap_or(1.0);
    let (pw, ph) = ((w as f32 * scale).round() as u32, (h as f32 * scale).round() as u32);
    let theme = scene.theme;
    let mut renderer = Renderer::new(hyprforge_authui::screen::font(theme), Pixels(theme.font_size));
    let mut scene = scene;
    scene.output = (w as f32, h as f32);
    let mut ui = UserInterface::<scene::Action, iced_widget::Theme, Renderer>::build(
        hyprforge_authui::screen::view::<Renderer>(scene),
        Size::new(w as f32, h as f32),
        Cache::default(),
        &mut renderer,
    );
    ui.draw(&mut renderer, &iced_widget::Theme::Dark, &Style { text_color: Color::WHITE }, mouse::Cursor::Unavailable);

    let mut pixmap = tiny_skia::Pixmap::new(pw, ph).unwrap();
    let mut mask = tiny_skia::Mask::new(pw, ph).unwrap();
    renderer.draw(
        &mut pixmap.as_mut(),
        &mut mask,
        &Viewport::with_physical_size(Size::new(pw, ph), scale),
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
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), pw, ph);
    enc.set_color(png::ColorType::Rgba);
    enc.write_header().unwrap().write_image_data(&rgba).unwrap();
    println!("wrote {}", path.display());
}

fn main() {
    let mut theme = std::env::var_os("PREVIEW_THEME")
        .map(|p| Theme::load(std::path::Path::new(&p)).expect("PREVIEW_THEME"))
        .unwrap_or_default();
    if let Some(wallpaper) = std::env::var_os("PREVIEW_WALLPAPER") {
        theme.wallpaper = Some(wallpaper.into());
    }
    let theme = hyprforge_authui::screen::renderable(theme);
    println!("wallpaper in use: {:?}", theme.wallpaper);

    let size = (1440, 900);
    let now = chrono::Local::now();
    let status = Status {
        layout: Some("us".into()),
        network: Some("studio-5G".into()),
        battery: Some(Battery { percent: 82, charging: false, low: false }),
    };
    let charging = Status { battery: Some(Battery { percent: 82, charging: true, low: false }), ..status.clone() };
    let low = Status { battery: Some(Battery { percent: 7, charging: false, low: true }), ..status.clone() };
    let ready = Fingerprint::Ready;
    let menu = PowerMenu {
        actions: vec![PowerAction::Suspend, PowerAction::Hibernate, PowerAction::Reboot, PowerAction::PowerOff],
        selected: 0,
    };
    let media = Media {
        title: "Nightcall".into(),
        artist: "Kavinsky".into(),
        player: "Spotify".into(),
        playing: true,
        progress: Some(0.38),
        can_previous: true,
        can_next: true,
    };
    let counts = vec![
        NotificationCount { app: "Signal".into(), count: 3 },
        NotificationCount { app: "Mail".into(), count: 12 },
        NotificationCount { app: "Calendar".into(), count: 1 },
    ];

    let empty = State::Asking { prompt: Prompt::secret("Password:"), entered: String::new().into() };
    let seven = State::Asking { prompt: Prompt::secret("Password:"), entered: "hunter2".to_string().into() };
    let failed = State::Failed { reason: "Wrong password".into() };

    let base = |state: &'static State| {
        let mut s = Scene::new(state, "apost", &theme, now);
        s.status = &status;
        s.fingerprint = &ready;
        s.power = Some(None);
        s
    };
    let leak = |s: State| -> &'static State { Box::leak(Box::new(s)) };
    let (empty, seven, failed) = (leak(empty), leak(seven), leak(failed));

    let mut idle = base(empty);
    idle.mode = Mode::Idle;
    render("1a-idle.png", idle, size);

    render("1b-entry.png", base(seven), size);

    let mut wrong = base(failed);
    wrong.rejection = Some(scene::rejection(std::time::Duration::from_millis(40), 9, 2));
    render("1c-wrong.png", wrong, size);

    render("1d-fingerprint.png", base(empty), size);

    let mut extras = base(empty);
    extras.mode = Mode::Idle;
    extras.status = &charging;
    extras.media = Some(&media);
    extras.notifications = &counts;
    render("1i-extras.png", extras, size);

    let mut battery = base(seven);
    battery.status = &low;
    render("1j-battery.png", battery, size);

    let mut power = base(seven);
    power.power = Some(Some(&menu));
    render("1k-power.png", power, size);

    let mut secondary = base(empty);
    secondary.role = Role::Secondary;
    render("1l-secondary.png", secondary, size);
    let mut portrait = base(empty);
    portrait.role = Role::Secondary;
    render("1l-portrait.png", portrait, (900, 1600));

    // The greeter's shape: no extras at all.
    render("greeter.png", Scene::new(seven, "apost", &theme, now), size);
}
