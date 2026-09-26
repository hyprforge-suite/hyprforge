//! A video frame on screen: one GPU texture, rewritten per frame.
//!
//! Not an `image::Handle` per frame, which is the obvious way and does not
//! work: `iced_wgpu` uploads an image of 2MB or more on a worker thread
//! and draws nothing until it lands (`MAX_SYNC_SIZE` in its
//! `image/cache.rs`), so a 1056x798 frame — 3.3MB — replaced thirty times
//! a second was never drawn at all, and the pane stayed empty until the
//! video stopped. This `shader` primitive keeps one texture and writes
//! each new frame into it with `queue.write_texture` in `prepare`, before
//! the frame is drawn, so what arrives is what is shown, and nothing is
//! allocated per frame on the GPU side.
//!
//! The texture is `Rgba8UnormSrgb` when the target is sRGB, so the frame's
//! sRGB bytes are decoded on sampling and encoded again on writing — the
//! same round trip a photograph's pixels make — and plain `Rgba8Unorm`
//! otherwise, where no conversion happens at either end.
//!
//! The whole pane is painted first in the theme's backdrop — a 1x1
//! texture of that colour, stretched — and the frame over it at its own
//! shape. That backdrop is also what covers the sentence `view.rs` puts
//! behind the widget for iced's software renderer, where a shader draws
//! nothing: with the GPU the sentence is hidden, without it, it shows.

use hyprforge_video::Frame;
use iced::widget::shader;
use iced::wgpu;
use iced::{mouse, Rectangle};
use std::sync::Arc;

/// The frame to show, and which one it is.
pub struct FilmProgram {
    pub frame: Arc<Frame>,
    /// Bumped per delivered frame, so the texture is written exactly when
    /// the picture changed.
    pub serial: u64,
    /// The theme's root surface, as sRGB bytes.
    pub backdrop: [u8; 4],
}

impl<Message> shader::Program<Message> for FilmProgram {
    type State = ();
    type Primitive = FilmPrimitive;

    fn draw(&self, _state: &(), _cursor: mouse::Cursor, _bounds: Rectangle) -> FilmPrimitive {
        FilmPrimitive { frame: self.frame.clone(), serial: self.serial, backdrop: self.backdrop }
    }
}

pub struct FilmPrimitive {
    frame: Arc<Frame>,
    serial: u64,
    backdrop: [u8; 4],
}

impl std::fmt::Debug for FilmPrimitive {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "FilmPrimitive({}x{} #{})", self.frame.width, self.frame.height, self.serial)
    }
}

/// The texture, its sampler and the one pipeline that draws it.
pub struct Film {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    texture_format: wgpu::TextureFormat,
    texture: Option<(wgpu::Texture, wgpu::BindGroup, u32, u32)>,
    serial: Option<u64>,
    /// The backdrop's 1x1 texture and the colour in it.
    backdrop: Option<(wgpu::Texture, wgpu::BindGroup, [u8; 4])>,
    /// The widget's whole rectangle, physical pixels.
    bounds: [f32; 4],
    /// Where the frame is drawn this frame, physical pixels: the widget's
    /// bounds narrowed to the frame's own shape.
    rect: [f32; 4],
}

const SHADER: &str = r#"
struct Out {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs(@builtin(vertex_index) i: u32) -> Out {
    var pos = array<vec2<f32>, 4>(vec2(-1.0, -1.0), vec2(-1.0, 1.0), vec2(1.0, -1.0), vec2(1.0, 1.0));
    var uv = array<vec2<f32>, 4>(vec2(0.0, 1.0), vec2(0.0, 0.0), vec2(1.0, 1.0), vec2(1.0, 0.0));
    var out: Out;
    out.clip = vec4<f32>(pos[i], 0.0, 1.0);
    out.uv = uv[i];
    return out;
}

@group(0) @binding(0) var frame: texture_2d<f32>;
@group(0) @binding(1) var linear_sampler: sampler;

@fragment
fn fs(in: Out) -> @location(0) vec4<f32> {
    return textureSample(frame, linear_sampler, in.uv);
}
"#;

impl shader::Pipeline for Film {
    fn new(device: &wgpu::Device, _queue: &wgpu::Queue, format: wgpu::TextureFormat) -> Self {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("photos film"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("photos film"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("photos film"),
            bind_group_layouts: &[&layout],
            push_constant_ranges: &[],
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("photos film"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs"),
                targets: &[Some(format.into())],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleStrip, ..Default::default() },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("photos film"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        Film {
            pipeline,
            layout,
            sampler,
            texture_format: if format.is_srgb() { wgpu::TextureFormat::Rgba8UnormSrgb } else { wgpu::TextureFormat::Rgba8Unorm },
            texture: None,
            serial: None,
            backdrop: None,
            bounds: [0.0; 4],
            rect: [0.0; 4],
        }
    }
}

impl shader::Primitive for FilmPrimitive {
    type Pipeline = Film;

    fn prepare(
        &self,
        film: &mut Film,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        bounds: &Rectangle,
        viewport: &shader::Viewport,
    ) {
        if !matches!(film.backdrop, Some((_, _, c)) if c == self.backdrop) {
            let (texture, bind_group) = film.texture_of(device, 1, 1);
            write(queue, &texture, &self.backdrop, 1, 1);
            film.backdrop = Some((texture, bind_group, self.backdrop));
        }
        let (w, h) = (self.frame.width.max(1), self.frame.height.max(1));
        if !matches!(film.texture, Some((_, _, tw, th)) if tw == w && th == h) {
            let (texture, bind_group) = film.texture_of(device, w, h);
            film.texture = Some((texture, bind_group, w, h));
            film.serial = None;
        }
        if film.serial != Some(self.serial) && self.frame.pixels.len() >= (w * h * 4) as usize {
            if let Some((texture, _, _, _)) = &film.texture {
                write(queue, texture, &self.frame.pixels, w, h);
            }
            film.serial = Some(self.serial);
        }
        let s = viewport.scale_factor();
        film.bounds = [bounds.x * s, bounds.y * s, bounds.width * s, bounds.height * s];
        film.rect = fitted(*bounds, (w, h), s);
    }

    fn render(
        &self,
        film: &Film,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        clip: &Rectangle<u32>,
    ) {
        let Some((_, bind_group, _, _)) = &film.texture else { return };
        if clip.width == 0 || clip.height == 0 {
            return;
        }
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("photos film"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });
        pass.set_scissor_rect(clip.x, clip.y, clip.width, clip.height);
        pass.set_pipeline(&film.pipeline);
        if let Some((_, backdrop, _)) = &film.backdrop {
            let [x, y, w, h] = film.bounds;
            pass.set_viewport(x, y, w.max(1.0), h.max(1.0), 0.0, 1.0);
            pass.set_bind_group(0, backdrop, &[]);
            pass.draw(0..4, 0..1);
        }
        let [x, y, w, h] = film.rect;
        pass.set_viewport(x, y, w.max(1.0), h.max(1.0), 0.0, 1.0);
        pass.set_bind_group(0, bind_group, &[]);
        pass.draw(0..4, 0..1);
    }
}

impl Film {
    /// A sampled texture of `w` x `h` and its bind group.
    fn texture_of(&self, device: &wgpu::Device, w: u32, h: u32) -> (wgpu::Texture, wgpu::BindGroup) {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("photos film frame"),
            size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: self.texture_format,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("photos film frame"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&self.sampler) },
            ],
        });
        (texture, bind_group)
    }
}

/// RGBA rows, tight, into the whole of `texture`.
fn write(queue: &wgpu::Queue, texture: &wgpu::Texture, pixels: &[u8], w: u32, h: u32) {
    queue.write_texture(
        wgpu::TexelCopyTextureInfo { texture, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
        pixels,
        wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(w * 4), rows_per_image: Some(h) },
        wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
    );
}

/// `bounds` (logical) narrowed to a frame's shape and centred, in
/// physical pixels — the frame never stretched, whatever size it arrived.
pub fn fitted(bounds: Rectangle, frame: (u32, u32), scale: f32) -> [f32; 4] {
    let (fw, fh) = (frame.0.max(1) as f32, frame.1.max(1) as f32);
    let (bw, bh) = (bounds.width * scale, bounds.height * scale);
    let k = (bw / fw).min(bh / fh);
    let (w, h) = (fw * k, fh * k);
    [bounds.x * scale + (bw - w) / 2.0, bounds.y * scale + (bh - h) / 2.0, w, h]
}

/// Parses and validates a WGSL source with naga, the compiler wgpu uses,
/// panicking with naga's own report. Shared with the model view's tests.
#[cfg(test)]
pub fn assert_valid_wgsl(name: &str, source: &str) {
    let module = naga::front::wgsl::parse_str(source)
        .unwrap_or_else(|e| panic!("{name} does not parse:\n{}", e.emit_to_string(source)));
    naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all())
        .validate(&module)
        .unwrap_or_else(|e| panic!("{name} does not validate: {e:?}"));
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A shader wgpu refuses is fatal at run time — `smooth`, a reserved
    /// word, once took the whole window down the first time a video
    /// played. Caught here instead.
    #[test]
    fn the_film_shader_compiles() {
        assert_valid_wgsl("film", SHADER);
    }

    /// The check above can fail: the shader as it was first written, with
    /// its sampler named `smooth`, is refused here as wgpu refused it.
    #[test]
    #[should_panic(expected = "reserved keyword")]
    fn a_shader_with_a_reserved_word_is_caught() {
        assert_valid_wgsl("film as first written", &SHADER.replace("linear_sampler", "smooth"));
    }

    #[test]
    fn a_frame_is_fitted_and_centred_without_stretching() {
        let bounds = Rectangle { x: 10.0, y: 20.0, width: 1000.0, height: 800.0 };
        let [x, y, w, h] = fitted(bounds, (1920, 1080), 1.0);
        assert!((w / h - 1920.0 / 1080.0).abs() < 1e-3);
        assert!((w - 1000.0).abs() < 1e-3, "{w}");
        assert!((y - (20.0 + (800.0 - h) / 2.0)).abs() < 1e-3 && (x - 10.0).abs() < 1e-3, "{x},{y}");
    }

    #[test]
    fn the_fit_is_in_physical_pixels() {
        let bounds = Rectangle { x: 0.0, y: 0.0, width: 100.0, height: 100.0 };
        assert_eq!(fitted(bounds, (50, 50), 1.6), [0.0, 0.0, 160.0, 160.0]);
    }
}
