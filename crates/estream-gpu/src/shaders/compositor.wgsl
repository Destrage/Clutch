// Draws one scene item as a textured quad onto a linear-light, premultiplied-alpha canvas.

struct Item {
    // Affine transform mapping the unit quad to canvas pixels.
    m0: vec2<f32>,
    m1: vec2<f32>,
    t: vec2<f32>,
    canvas: vec2<f32>,
    uv_min: vec2<f32>,
    uv_max: vec2<f32>,
    opacity: f32,
    _pad0: f32,
    _pad1: vec2<f32>,
};

@group(0) @binding(0) var<uniform> item: Item;
@group(1) @binding(0) var source_tex: texture_2d<f32>;
@group(1) @binding(1) var source_sampler: sampler;

struct VsOut {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) index: u32) -> VsOut {
    // Triangle strip: (0,0) (1,0) (0,1) (1,1)
    let unit = vec2<f32>(f32(index & 1u), f32(index >> 1u));
    let pixel = item.m0 * unit.x + item.m1 * unit.y + item.t;
    let ndc = vec2<f32>(pixel.x / item.canvas.x * 2.0 - 1.0, 1.0 - pixel.y / item.canvas.y * 2.0);
    var out: VsOut;
    out.position = vec4<f32>(ndc, 0.0, 1.0);
    out.uv = mix(item.uv_min, item.uv_max, unit);
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    // Source textures hold straight alpha; sRGB formats are decoded to linear by the sampler.
    // Keep bilinear taps inside the cropped region so cropped-away texels never bleed in.
    let half_texel = 0.5 / vec2<f32>(textureDimensions(source_tex));
    let lo = item.uv_min + half_texel;
    let hi = max(lo, item.uv_max - half_texel);
    let c = textureSample(source_tex, source_sampler, clamp(in.uv, lo, hi));
    return vec4<f32>(c.rgb * c.a, c.a) * item.opacity;
}
