// Carried from view3d, which ported it from fstl (MIT); see
// crates/hyprforge-mesh/LICENSE.
//
// Colored line shaders (model axes and the HUD axis flower), ported from
// fstl's gl/colored_lines.{vert,frag}. Kept in its own module so it can own
// @group(0) @binding(0) for its own, much smaller, uniform block.

struct LineUniforms {
    mvp: mat4x4<f32>,
};

@group(0) @binding(0) var<uniform> lu: LineUniforms;

struct LineOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) color: vec4<f32>,
};

@vertex
fn vs_line(@location(0) pos: vec3<f32>, @location(1) color: vec4<f32>) -> LineOut {
    var out: LineOut;
    out.clip = lu.mvp * vec4<f32>(pos, 1.0);
    out.color = color;
    return out;
}

@fragment
fn fs_line(in: LineOut) -> @location(0) vec4<f32> {
    return in.color;
}
