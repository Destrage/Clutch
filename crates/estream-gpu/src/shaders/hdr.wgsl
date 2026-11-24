// Maps an HDR capture (scRGB: linear BT.709, 1.0 = 80 nits) to SDR linear light. SDR white
// becomes 1.0; anything brighter is compressed towards 1.0 with a shoulder that reaches it at
// the display's peak. Scaling by the brightest channel keeps hues. Must match `map_pixel` in
// hdr.rs (the CPU reference the tests compare against).

struct Params {
    // 80 / SDR white level (nits)
    scale: f32,
    // display peak / SDR white, at least 1
    peak: f32,
    // where the shoulder starts (SDR-relative)
    knee: f32,
    pad: f32,
}

@group(0) @binding(0) var input: texture_2d<f32>;
@group(0) @binding(1) var<uniform> p: Params;

@vertex
fn vs_main(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
    let uv = vec2<f32>(f32((index << 1u) & 2u), f32(index & 2u));
    return vec4<f32>(uv * 2.0 - 1.0, 0.0, 1.0);
}

// Extended Reinhard over the part above the knee: slope 1 at the knee, 1.0 at the peak.
fn shoulder(m: f32) -> f32 {
    if m <= p.knee {
        return m;
    }
    let room = 1.0 - p.knee;
    let span = p.peak - p.knee;
    if span <= 0.0 {
        return min(m, 1.0);
    }
    let t = (m - p.knee) / room;
    let white = span / room;
    let r = t * (1.0 + t / (white * white)) / (1.0 + t);
    return min(p.knee + room * r, 1.0);
}

@fragment
fn fs_main(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let texel = textureLoad(input, vec2<i32>(pos.xy), 0);
    let c = max(texel.rgb, vec3<f32>(0.0)) * p.scale;
    let m = max(c.r, max(c.g, c.b));
    if m <= p.knee {
        return vec4<f32>(c, 1.0);
    }
    return vec4<f32>(c * (shoulder(m) / m), 1.0);
}
