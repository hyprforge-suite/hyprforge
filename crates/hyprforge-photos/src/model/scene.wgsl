// Scene shaders, carried from view3d (github.com/adamrpostjr/view3d), which
// ported them from fstl's GLSL 1.20 shaders in gl/*. The fragment shaders
// and their colour constants are translations of fstl's source, MIT
// licensed, Copyright (c) 2014-2017 Matthew Keeter and the fstl
// contributors — crates/hyprforge-mesh/LICENSE has the terms and the detail.
//
// Scene shaders, ported from fstl's GLSL 1.20 shaders in gl/*.
//
// As in fstl, `ec_pos` is the *clip-space* position (pre-divide) and the face
// normal is recovered per-fragment with screen-space derivatives, so no normal
// attribute is needed.

struct Uniforms {
    mvp: mat4x4<f32>,
    ambient: vec4<f32>,    // rgb + factor
    directive: vec4<f32>,  // rgb + factor
    light_dir: vec4<f32>,  // xyz direction, w = 1 / zoom
    flags: vec4<f32>,      // x = 1 when the file supplied colors
    backdrop: vec4<f32>,   // the theme's root surface, in the target's space
};

@group(0) @binding(0) var<uniform> u: Uniforms;

struct MeshOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) ec_pos: vec3<f32>,
    @location(1) color: vec4<f32>,
};

@vertex
fn vs_mesh(@location(0) pos: vec3<f32>, @location(1) color: vec4<f32>) -> MeshOut {
    var out: MeshOut;
    out.clip = u.mvp * vec4<f32>(pos, 1.0);
    out.ec_pos = out.clip.xyz;
    out.color = color;
    return out;
}

fn face_normal(ec_pos: vec3<f32>) -> vec3<f32> {
    // GL window coordinates have y up, wgpu framebuffer coordinates have y
    // down, so the derivative pair is swapped relative to fstl's GLSL to keep
    // the recovered normal pointing at the viewer.
    var n = normalize(cross(dpdy(ec_pos), dpdx(ec_pos)));
    // Compensate for z-flattening when zooming (fstl passes 1 / zoom).
    n.z = n.z * u.light_dir.w;
    return normalize(n);
}

@fragment
fn fs_shaded(in: MeshOut) -> @location(0) vec4<f32> {
    let base3 = vec3<f32>(0.99, 0.96, 0.89);
    let base2 = vec3<f32>(0.92, 0.91, 0.83);
    let base00 = vec3<f32>(0.40, 0.48, 0.51);

    let n = face_normal(in.ec_pos);
    let a = dot(n, vec3<f32>(0.0, 0.0, 1.0));
    let b = dot(n, vec3<f32>(-0.57, -0.57, 0.57));

    let c = (a * base2 + (1.0 - a) * base00) * 0.5 + (b * base3 + (1.0 - b) * base00) * 0.5;
    return vec4<f32>(c, 1.0);
}

@fragment
fn fs_surface_angle(in: MeshOut) -> @location(0) vec4<f32> {
    let n = face_normal(in.ec_pos);
    // Rotated 10 degrees about the red axis for a better color match.
    let x = dot(n, vec3<f32>(1.0, 0.0, 0.0));
    let y = dot(n, vec3<f32>(0.0, 0.985, 0.174));
    let z = dot(n, vec3<f32>(0.0, -0.174, 0.985));
    return vec4<f32>(0.5 - 0.5 * x, 0.5 - 0.5 * y, 0.5 + 0.5 * z, 1.0);
}

@fragment
fn fs_mesh_light(in: MeshOut) -> @location(0) vec4<f32> {
    let dir = normalize(u.light_dir.xyz);
    let n = face_normal(in.ec_pos);
    let c = u.ambient.w * u.ambient.xyz + u.directive.w * dot(n, dir) * u.directive.xyz;
    return vec4<f32>(c, 1.0);
}

@fragment
fn fs_wireframe(in: MeshOut) -> @location(0) vec4<f32> {
    return vec4<f32>(1.0, 1.0, 1.0, 1.0);
}

// Material color: the file's own color, lit with a soft head-on term so shape
// is still readable. Files without colors fall back to a neutral gray.
@fragment
fn fs_material(in: MeshOut) -> @location(0) vec4<f32> {
    let n = face_normal(in.ec_pos);
    let base = select(vec3<f32>(0.75, 0.75, 0.75), in.color.rgb, u.flags.x > 0.5);
    let a = clamp(dot(n, vec3<f32>(0.0, 0.0, 1.0)), 0.0, 1.0);
    let b = clamp(dot(n, vec3<f32>(-0.57, -0.57, 0.57)), 0.0, 1.0);
    return vec4<f32>(base * (0.35 + 0.45 * a + 0.30 * b), 1.0);
}

// ---------------------------------------------------------------- backdrop

// Not fstl's teal gradient: the backdrop is the theme's own root surface,
// passed in as a uniform, so a model sits on the same plane a photograph
// does. A colour constant here would be an app with its own palette, which
// is the one thing this suite does not allow.
struct QuadOut {
    @builtin(position) clip: vec4<f32>,
};

@vertex
fn vs_backdrop(@builtin(vertex_index) idx: u32) -> QuadOut {
    var pos = array<vec2<f32>, 4>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>(-1.0, 1.0),
        vec2<f32>(1.0, -1.0),
        vec2<f32>(1.0, 1.0),
    );
    var out: QuadOut;
    // Sits at the far end of the depth range so the mesh always wins.
    out.clip = vec4<f32>(pos[idx], 0.95, 1.0);
    return out;
}

@fragment
fn fs_backdrop(in: QuadOut) -> @location(0) vec4<f32> {
    return u.backdrop;
}
