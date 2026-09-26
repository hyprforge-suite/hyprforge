//! Draws a model's thumbnail to a PNG — for looking at what the grid will
//! show without running the viewer.
//!
//! `cargo run -p hyprforge-mesh --example thumbnail -- model.stl out.png [edge]`

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (Some(model), Some(out)) = (args.first(), args.get(1)) else {
        eprintln!("usage: thumbnail MODEL OUT.png [EDGE]");
        std::process::exit(2);
    };
    let edge = args.get(2).and_then(|e| e.parse().ok()).unwrap_or(128);
    let (mesh, _) = hyprforge_mesh::load(std::path::Path::new(model), true).unwrap_or_else(|e| {
        eprintln!("{e:#}");
        std::process::exit(1);
    });
    let start = std::time::Instant::now();
    let img = hyprforge_mesh::thumbnail::render(&mesh, edge);
    eprintln!("{} triangles drawn at {edge}px in {:?}", mesh.tri_count(), start.elapsed());
    let file = std::io::BufWriter::new(std::fs::File::create(out).expect("create output"));
    let mut encoder = png::Encoder::new(file, img.width, img.height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.write_header().and_then(|mut w| w.write_image_data(&img.pixels)).expect("write png");
}
