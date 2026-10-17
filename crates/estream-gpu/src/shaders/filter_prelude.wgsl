// Shared by every filter shader (built-in and custom). Colors are straight alpha, linear light.
// uv (0,0) is the top-left of the input.

struct Params {
    in_size: vec2<f32>,
    out_size: vec2<f32>,
    // Crop/pad: input pixel = output pixel + offset.
    offset: vec2<f32>,
    time_s: f32,
    kind: u32,
    p: array<vec4<f32>, 3>,
};

@group(0) @binding(0) var<uniform> params: Params;
@group(1) @binding(0) var src_tex: texture_2d<f32>;
@group(1) @binding(1) var src_sampler: sampler;
@group(2) @binding(0) var lut_tex: texture_3d<f32>;
@group(2) @binding(1) var mask_tex: texture_2d<f32>;
@group(2) @binding(2) var aux_sampler: sampler;

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) i: u32) -> VsOut {
    let raw = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    var out: VsOut;
    out.pos = vec4<f32>(raw * 2.0 - 1.0, 0.0, 1.0);
    out.uv = vec2<f32>(raw.x, 1.0 - raw.y);
    return out;
}

/// Filter parameter `i` (0..11).
fn param(i: u32) -> f32 {
    let v = params.p[min(i, 11u) / 4u];
    return v[min(i, 11u) % 4u];
}

/// Input size in pixels.
fn size() -> vec2<f32> {
    return params.in_size;
}

/// Seconds since the engine started.
fn time() -> f32 {
    return params.time_s;
}

/// The input at `uv` (straight alpha, linear).
fn source(uv: vec2<f32>) -> vec4<f32> {
    return textureSampleLevel(src_tex, src_sampler, uv, 0.0);
}

fn to_srgb(c: vec3<f32>) -> vec3<f32> {
    let x = max(c, vec3<f32>(0.0));
    return select(1.055 * pow(x, vec3<f32>(1.0 / 2.4)) - 0.055, x * 12.92, x <= vec3<f32>(0.0031308));
}

fn to_linear(c: vec3<f32>) -> vec3<f32> {
    let x = max(c, vec3<f32>(0.0));
    return select(pow((x + 0.055) / 1.055, vec3<f32>(2.4)), x / 12.92, x <= vec3<f32>(0.04045));
}

/// Rec. 709 luma of an (encoded or linear) color.
fn luma(c: vec3<f32>) -> f32 {
    return dot(c, vec3<f32>(0.2126, 0.7152, 0.0722));
}
