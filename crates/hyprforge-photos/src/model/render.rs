//! The wgpu half of the model view: pipelines, buffers, and the pass that
//! draws a mesh.
//!
//! Carried from view3d's `render/mod.rs`, whose axis and axis-flower
//! geometry is a translation of fstl's `axis.cpp` (MIT; see
//! `crates/hyprforge-mesh/LICENSE`). Three things changed on the way in:
//!
//! - **wgpu 27, not 30.** It has to be the wgpu iced was built with, so the
//!   descriptors use 27's field names (`push_constant_ranges`, `multiview`,
//!   a plain `bool` for depth writes).
//! - **Its own pass, with its own depth buffer.** iced hands a primitive
//!   its shared render pass, and that pass has no depth attachment; a mesh
//!   drawn without one shows its back faces through its front. So the
//!   primitive draws in `render` instead of `draw`, into a pass of its own
//!   that loads what iced already painted and clips to the widget.
//! - **The theme's backdrop.** fstl's teal gradient became a flat quad in
//!   the theme's root surface colour, handed in each frame.

use glam::Mat4;
use hyprforge_mesh::mesh::{Mesh, Vertex};
use hyprforge_mesh::style::DrawMode;
use iced::wgpu;
use iced::wgpu::util::DeviceExt as _;

/// One 256-byte aligned slot per line draw (model axes, the flower, and
/// its three letters).
pub const LINE_SLOTS: usize = 5;
const LINE_SLOT_SIZE: u64 = 256;
const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
struct Uniforms {
    mvp: [[f32; 4]; 4],
    ambient: [f32; 4],
    directive: [f32; 4],
    light_dir: [f32; 4],
    flags: [f32; 4],
    backdrop: [f32; 4],
}

/// Everything one frame needs, computed from the camera and the style.
#[derive(Clone, Debug)]
pub struct SceneParams {
    pub mvp: Mat4,
    pub ambient: [f32; 4],
    pub directive: [f32; 4],
    pub light_dir: [f32; 3],
    pub zoom_inv: f32,
    pub has_colors: bool,
    pub draw_mode: DrawMode,
    pub draw_axes: bool,
    /// Model axes, the flower, then its X/Y/Z letters.
    pub line_mvps: [Mat4; LINE_SLOTS],
    /// The theme's root surface, as sRGB — converted to the target's
    /// space in `write_uniforms`.
    pub backdrop: iced::Color,
}

struct GpuMesh {
    verts: wgpu::Buffer,
    indices: wgpu::Buffer,
    index_count: u32,
    edges: Option<(wgpu::Buffer, u32)>,
}

/// The pipelines and buffers, created once and shared by every frame.
pub struct Scene {
    backdrop: wgpu::RenderPipeline,
    /// Filled modes, indexed by `DrawMode::fill_index`.
    filled: Vec<wgpu::RenderPipeline>,
    wireframe: wgpu::RenderPipeline,
    line: wgpu::RenderPipeline,
    line_hud: wgpu::RenderPipeline,

    uniforms: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    line_uniforms: wgpu::Buffer,
    line_bind_group: wgpu::BindGroup,

    hud_lines: wgpu::Buffer,
    axis_lines: wgpu::Buffer,

    mesh: Option<GpuMesh>,
    /// Which mesh is on the GPU, so a frame re-uploads only when the
    /// window has loaded a different one.
    pub generation: Option<u64>,
    depth: Option<(wgpu::TextureView, u32, u32)>,
    /// The widget's rectangle this frame, physical pixels.
    pub frame: [f32; 4],
    /// Whether the target stores sRGB, in which case colours are handed to
    /// it linear — the conversion iced itself makes with `web-colors` off.
    srgb: bool,
}

const VERTEX_LAYOUT: wgpu::VertexBufferLayout<'static> = wgpu::VertexBufferLayout {
    array_stride: std::mem::size_of::<Vertex>() as u64,
    step_mode: wgpu::VertexStepMode::Vertex,
    attributes: &[
        wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x3, offset: 0, shader_location: 0 },
        wgpu::VertexAttribute { format: wgpu::VertexFormat::Unorm8x4, offset: 12, shader_location: 1 },
    ],
};

fn depth_state(write: bool, test: bool) -> Option<wgpu::DepthStencilState> {
    Some(wgpu::DepthStencilState {
        format: DEPTH_FORMAT,
        depth_write_enabled: write,
        depth_compare: if test { wgpu::CompareFunction::Less } else { wgpu::CompareFunction::Always },
        stencil: wgpu::StencilState::default(),
        bias: wgpu::DepthBiasState::default(),
    })
}

impl Scene {
    pub fn new(device: &wgpu::Device, target_format: wgpu::TextureFormat) -> Self {
        let scene_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("photos model scene"),
            source: wgpu::ShaderSource::Wgsl(include_str!("scene.wgsl").into()),
        });
        let line_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("photos model lines"),
            source: wgpu::ShaderSource::Wgsl(include_str!("lines.wgsl").into()),
        });

        let uniform_entry = |dynamic: bool, min: u64| wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: dynamic,
                min_binding_size: std::num::NonZeroU64::new(min),
            },
            count: None,
        };
        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("photos model uniforms"),
            entries: &[uniform_entry(false, std::mem::size_of::<Uniforms>() as u64)],
        });
        let line_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("photos model line uniforms"),
            entries: &[uniform_entry(true, 64)],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("photos model scene layout"),
            bind_group_layouts: &[&bgl],
            push_constant_ranges: &[],
        });
        let line_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("photos model line layout"),
            bind_group_layouts: &[&line_bgl],
            push_constant_ranges: &[],
        });

        let make_pipeline = |label: &str,
                             shader: &wgpu::ShaderModule,
                             pipeline_layout: &wgpu::PipelineLayout,
                             vs: &str,
                             fs: &str,
                             buffers: &[wgpu::VertexBufferLayout<'static>],
                             topology: wgpu::PrimitiveTopology,
                             depth: Option<wgpu::DepthStencilState>| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(pipeline_layout),
                vertex: wgpu::VertexState {
                    module: shader,
                    entry_point: Some(vs),
                    buffers,
                    compilation_options: Default::default(),
                },
                fragment: Some(wgpu::FragmentState {
                    module: shader,
                    entry_point: Some(fs),
                    targets: &[Some(target_format.into())],
                    compilation_options: Default::default(),
                }),
                primitive: wgpu::PrimitiveState { topology, ..Default::default() },
                depth_stencil: depth,
                multisample: wgpu::MultisampleState::default(),
                multiview: None,
                cache: None,
            })
        };

        let backdrop = make_pipeline(
            "photos model backdrop",
            &scene_shader,
            &layout,
            "vs_backdrop",
            "fs_backdrop",
            &[],
            wgpu::PrimitiveTopology::TriangleStrip,
            depth_state(true, false),
        );
        let filled = DrawMode::FILLED
            .iter()
            .map(|mode| {
                make_pipeline(
                    mode.label(),
                    &scene_shader,
                    &layout,
                    "vs_mesh",
                    mode.entry_point(),
                    &[VERTEX_LAYOUT],
                    wgpu::PrimitiveTopology::TriangleList,
                    depth_state(true, true),
                )
            })
            .collect();
        let wireframe = make_pipeline(
            "photos model wireframe",
            &scene_shader,
            &layout,
            "vs_mesh",
            "fs_wireframe",
            &[VERTEX_LAYOUT],
            wgpu::PrimitiveTopology::LineList,
            depth_state(true, true),
        );
        let line = make_pipeline(
            "photos model axes",
            &line_shader,
            &line_layout,
            "vs_line",
            "fs_line",
            &[VERTEX_LAYOUT],
            wgpu::PrimitiveTopology::LineList,
            depth_state(true, true),
        );
        // The flower sits over everything, so it ignores depth.
        let line_hud = make_pipeline(
            "photos model axis flower",
            &line_shader,
            &line_layout,
            "vs_line",
            "fs_line",
            &[VERTEX_LAYOUT],
            wgpu::PrimitiveTopology::LineList,
            depth_state(false, false),
        );

        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("photos model uniforms"),
            size: std::mem::size_of::<Uniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("photos model uniforms"),
            layout: &bgl,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: uniforms.as_entire_binding() }],
        });
        let line_uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("photos model line uniforms"),
            size: LINE_SLOTS as u64 * LINE_SLOT_SIZE,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let line_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("photos model line uniforms"),
            layout: &line_bgl,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                    buffer: &line_uniforms,
                    offset: 0,
                    size: std::num::NonZeroU64::new(64),
                }),
            }],
        });
        let hud_lines = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("photos model axis flower"),
            contents: bytemuck::cast_slice(&hud_line_verts()),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let axis_lines = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("photos model axes"),
            size: (6 * std::mem::size_of::<Vertex>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        Self {
            backdrop,
            filled,
            wireframe,
            line,
            line_hud,
            uniforms,
            bind_group,
            line_uniforms,
            line_bind_group,
            hud_lines,
            axis_lines,
            mesh: None,
            generation: None,
            depth: None,
            frame: [0.0; 4],
            srgb: target_format.is_srgb(),
        }
    }

    /// Puts `mesh` on the GPU, replacing whatever was there. The old
    /// buffers are dropped here, so only one model's geometry is ever
    /// held on the card — the window holds one on the CPU side too.
    pub fn upload_mesh(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, mesh: &Mesh) {
        let verts = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("photos model vertices"),
            contents: bytemuck::cast_slice(&mesh.verts),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let indices = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("photos model indices"),
            contents: bytemuck::cast_slice(&mesh.indices),
            usage: wgpu::BufferUsages::INDEX,
        });
        self.mesh = Some(GpuMesh { verts, indices, index_count: mesh.indices.len() as u32, edges: None });

        // Model-space axes, extended past the model like fstl's
        // `Axis::setScale`.
        let (min, max) = (mesh.bounds.min, mesh.bounds.max);
        let margin = 0.25 * mesh.bounds.size().max_element();
        let mut verts = [Vertex::default(); 6];
        for axis in 0..3 {
            let color = 0xff00_0000 | 0xffu32 << (8 * axis);
            let (mut a, mut b) = ([0.0f32; 3], [0.0f32; 3]);
            a[axis] = min[axis] - margin;
            b[axis] = max[axis] + margin;
            verts[axis * 2] = Vertex { pos: a, color };
            verts[axis * 2 + 1] = Vertex { pos: b, color };
        }
        queue.write_buffer(&self.axis_lines, 0, bytemuck::cast_slice(&verts));
    }

    /// The wireframe's edge list, built the first time it is drawn — most
    /// models are never looked at as a wireframe, and the list is as big
    /// as the index buffer again.
    pub fn ensure_edges(&mut self, device: &wgpu::Device, mesh: &Mesh) {
        let Some(gpu) = self.mesh.as_mut() else { return };
        if gpu.edges.is_some() {
            return;
        }
        let edges = mesh.edge_indices();
        let buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("photos model edges"),
            contents: bytemuck::cast_slice(&edges),
            usage: wgpu::BufferUsages::INDEX,
        });
        gpu.edges = Some((buf, edges.len() as u32));
    }

    /// A depth buffer the size of the target, made again only when that
    /// size changes. The target's size, not the widget's: a depth
    /// attachment must match the colour attachment it is paired with.
    pub fn ensure_depth(&mut self, device: &wgpu::Device, width: u32, height: u32) {
        let (width, height) = (width.max(1), height.max(1));
        if matches!(self.depth, Some((_, w, h)) if w == width && h == height) {
            return;
        }
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("photos model depth"),
            size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: DEPTH_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        self.depth = Some((texture.create_view(&wgpu::TextureViewDescriptor::default()), width, height));
    }

    pub fn write_uniforms(&self, queue: &wgpu::Queue, p: &SceneParams) {
        let backdrop = if self.srgb {
            p.backdrop.into_linear()
        } else {
            [p.backdrop.r, p.backdrop.g, p.backdrop.b, p.backdrop.a]
        };
        let u = Uniforms {
            mvp: p.mvp.to_cols_array_2d(),
            ambient: p.ambient,
            directive: p.directive,
            light_dir: [p.light_dir[0], p.light_dir[1], p.light_dir[2], p.zoom_inv],
            flags: [if p.has_colors { 1.0 } else { 0.0 }, 0.0, 0.0, 0.0],
            backdrop,
        };
        queue.write_buffer(&self.uniforms, 0, bytemuck::bytes_of(&u));
        for (i, m) in p.line_mvps.iter().enumerate() {
            queue.write_buffer(&self.line_uniforms, i as u64 * LINE_SLOT_SIZE, bytemuck::cast_slice(&m.to_cols_array()));
        }
    }

    /// Draws into `target` over what iced already painted, clipped to
    /// `clip` — see the module doc for why this is its own pass.
    pub fn render(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        clip: &iced::Rectangle<u32>,
        p: &SceneParams,
    ) {
        let Some((depth, _, _)) = &self.depth else { return };
        if clip.width == 0 || clip.height == 0 {
            return;
        }
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("photos model"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: depth,
                depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(1.0), store: wgpu::StoreOp::Discard }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
        });
        let [x, y, w, h] = self.frame;
        pass.set_viewport(x, y, w.max(1.0), h.max(1.0), 0.0, 1.0);
        pass.set_scissor_rect(clip.x, clip.y, clip.width, clip.height);
        self.paint(&mut pass, p);
    }

    fn paint(&self, rp: &mut wgpu::RenderPass<'_>, p: &SceneParams) {
        rp.set_pipeline(&self.backdrop);
        rp.set_bind_group(0, &self.bind_group, &[]);
        rp.draw(0..4, 0..1);

        if let Some(gpu) = &self.mesh {
            rp.set_vertex_buffer(0, gpu.verts.slice(..));
            match (p.draw_mode, &gpu.edges) {
                (DrawMode::Wireframe, Some((edges, count))) => {
                    rp.set_pipeline(&self.wireframe);
                    rp.set_index_buffer(edges.slice(..), wgpu::IndexFormat::Uint32);
                    rp.draw_indexed(0..*count, 0, 0..1);
                }
                (mode, _) => {
                    let idx = mode.fill_index().unwrap_or(0);
                    rp.set_pipeline(&self.filled[idx]);
                    rp.set_index_buffer(gpu.indices.slice(..), wgpu::IndexFormat::Uint32);
                    rp.draw_indexed(0..gpu.index_count, 0, 0..1);
                }
            }
        }

        if p.draw_axes {
            rp.set_pipeline(&self.line);
            rp.set_bind_group(0, &self.line_bind_group, &[0]);
            rp.set_vertex_buffer(0, self.axis_lines.slice(..));
            rp.draw(0..6, 0..1);

            rp.set_pipeline(&self.line_hud);
            rp.set_vertex_buffer(0, self.hud_lines.slice(..));
            for (slot, range) in [(1u32, 0..6u32), (2, 6..10), (3, 10..16), (4, 16..22)] {
                rp.set_bind_group(0, &self.line_bind_group, &[slot * LINE_SLOT_SIZE as u32]);
                rp.draw(range, 0..1);
            }
        }
    }
}

/// Unit axis lines plus the little X/Y/Z letters, matching fstl's
/// `axis.cpp`.
fn hud_line_verts() -> Vec<Vertex> {
    const X_LET: [f32; 12] = [-0.1, -0.2, 0.0, 0.1, 0.2, 0.0, 0.1, -0.2, 0.0, -0.1, 0.2, 0.0];
    const Y_LET: [f32; 18] = [0.0, -0.2, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.1, 0.2, 0.0, 0.0, 0.0, 0.0, -0.1, 0.2, 0.0];
    const Z_LET: [f32; 18] = [
        -0.1, -0.2, 0.0, 0.1, -0.2, 0.0, 0.1, -0.2, 0.0, -0.1, 0.2, 0.0, -0.1, 0.2, 0.0, 0.1, 0.2, 0.0,
    ];
    let mut out = Vec::with_capacity(22);
    for axis in 0..3usize {
        let color = 0xff00_0000 | 0xffu32 << (8 * axis);
        let mut end = [0.0f32; 3];
        end[axis] = 1.0;
        out.push(Vertex { pos: [0.0; 3], color });
        out.push(Vertex { pos: end, color });
    }
    for (axis, letter) in [&X_LET[..], &Y_LET[..], &Z_LET[..]].iter().enumerate() {
        let color = 0xff00_0000 | 0xffu32 << (8 * axis);
        for p in letter.chunks_exact(3) {
            out.push(Vertex { pos: [p[0], p[1], p[2]], color });
        }
    }
    out
}
