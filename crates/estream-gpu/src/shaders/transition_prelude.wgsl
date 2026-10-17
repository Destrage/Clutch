// Shared by every transition shader (built-in and custom). Colors are linear light,
// premultiplied alpha. uv (0,0) is the top-left of the canvas.

struct Params {
    progress: f32,
    kind: u32,
    softness: f32,
    _pad: f32,
    direction: vec2<f32>,
    size: vec2<f32>,
    color: vec4<f32>,
};

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var from_tex: texture_2d<f32>;
@group(0) @binding(2) var to_tex: texture_2d<f32>;
@group(0) @binding(3) var tex_sampler: sampler;

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) i: u32) -> VsOut {
    // One triangle covering the target.
    let raw = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    var out: VsOut;
    out.pos = vec4<f32>(raw * 2.0 - 1.0, 0.0, 1.0);
    out.uv = vec2<f32>(raw.x, 1.0 - raw.y);
    return out;
}

fn inside(uv: vec2<f32>) -> bool {
    return all(uv >= vec2<f32>(0.0)) && all(uv <= vec2<f32>(1.0));
}

/// The outgoing scene at `uv` (transparent outside the canvas).
fn from_color(uv: vec2<f32>) -> vec4<f32> {
    let c = textureSampleLevel(from_tex, tex_sampler, clamp(uv, vec2<f32>(0.0), vec2<f32>(1.0)), 0.0);
    return select(vec4<f32>(0.0), c, inside(uv));
}

/// The incoming scene at `uv` (transparent outside the canvas).
fn to_color(uv: vec2<f32>) -> vec4<f32> {
    let c = textureSampleLevel(to_tex, tex_sampler, clamp(uv, vec2<f32>(0.0), vec2<f32>(1.0)), 0.0);
    return select(vec4<f32>(0.0), c, inside(uv));
}

/// Canvas size in pixels.
fn resolution() -> vec2<f32> {
    return params.size;
}

/// Premultiplied "a over b".
fn over(a: vec4<f32>, b: vec4<f32>) -> vec4<f32> {
    return a + b * (1.0 - a.a);
}

fn ease(t: f32) -> f32 {
    let x = clamp(t, 0.0, 1.0);
    return x * x * (3.0 - 2.0 * x);
}
