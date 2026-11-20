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
    // Drawn part of the unit square: u_min, v_min, u_max, v_max.
    clip: vec4<f32>,
    // Shape morph: t, feather (px), from kind, to kind (0 = none, 1 = field, 2 = rectangle).
    morph: vec4<f32>,
};

@group(0) @binding(0) var<uniform> item: Item;
@group(1) @binding(0) var source_tex: texture_2d<f32>;
@group(1) @binding(1) var source_sampler: sampler;
// Item mask (R8; a 1x1 white texture for unmasked items), over the quad's unit square.
@group(2) @binding(0) var mask_tex: texture_2d<f32>;
@group(2) @binding(1) var mask_sampler: sampler;
// The second distance field of a shape morph.
@group(3) @binding(0) var morph_tex: texture_2d<f32>;
@group(3) @binding(1) var morph_sampler: sampler;

struct VsOut {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) unit: vec2<f32>,
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
    out.unit = unit;
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
    // Sampled before branching so the samples stay in uniform control flow.
    let first = textureSample(mask_tex, mask_sampler, in.unit).r;
    let second = textureSample(morph_tex, morph_sampler, in.unit).r;
    var mask = first;
    if (item.morph.z > 0.5) {
        let size = vec2<f32>(length(item.m0), length(item.m1));
        let long = max(size.x, size.y);
        let d = mix(
            end_distance(item.morph.z, first, in.unit, size, long),
            end_distance(item.morph.w, second, in.unit, size, long),
            item.morph.x,
        );
        mask = clamp(0.5 - d / item.morph.y, 0.0, 1.0);
    }
    let inside = step(item.clip.xy, in.unit) * step(in.unit, item.clip.zw);
    return vec4<f32>(c.rgb * c.a, c.a) * (item.opacity * mask * inside.x * inside.y);
}

// Distance (canvas pixels, negative inside) from one end of a morph: a baked field in units of
// the long side, or the quad's own rectangle.
fn end_distance(kind: f32, field: f32, unit: vec2<f32>, size: vec2<f32>, long: f32) -> f32 {
    if (kind > 1.5) {
        let q = abs((unit - 0.5) * size) - 0.5 * size;
        return length(max(q, vec2<f32>(0.0))) + min(max(q.x, q.y), 0.0);
    }
    return field * long;
}
