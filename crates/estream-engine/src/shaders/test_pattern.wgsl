// 75% SMPTE-style color bars (top 2/3), a grey ramp (bottom third), and a white
// marker that sweeps left to right every 2 seconds. Writes sRGB-encoded values
// into an *Srgb target, so colors are converted to linear before output.

struct Pattern {
    size: vec2<f32>,
    time: f32,
    _pad: f32,
};

@group(0) @binding(0) var<uniform> pattern: Pattern;

@vertex
fn vs_main(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
    let uv = vec2<f32>(f32((index << 1u) & 2u), f32(index & 2u));
    return vec4<f32>(uv * 2.0 - 1.0, 0.0, 1.0);
}

fn srgb_to_linear(c: vec3<f32>) -> vec3<f32> {
    let lo = c / 12.92;
    let hi = pow((c + 0.055) / 1.055, vec3<f32>(2.4));
    return select(hi, lo, c <= vec3<f32>(0.04045));
}

@fragment
fn fs_main(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let uv = pos.xy / pattern.size;
    var encoded: vec3<f32>;
    if uv.y < 2.0 / 3.0 {
        let bars = array<vec3<f32>, 7>(
            vec3<f32>(0.75, 0.75, 0.75), // white
            vec3<f32>(0.75, 0.75, 0.0),  // yellow
            vec3<f32>(0.0, 0.75, 0.75),  // cyan
            vec3<f32>(0.0, 0.75, 0.0),   // green
            vec3<f32>(0.75, 0.0, 0.75),  // magenta
            vec3<f32>(0.75, 0.0, 0.0),   // red
            vec3<f32>(0.0, 0.0, 0.75),   // blue
        );
        encoded = bars[min(u32(uv.x * 7.0), 6u)];
    } else {
        encoded = vec3<f32>(uv.x);
    }

    // Moving marker in the bottom band.
    let marker_x = fract(pattern.time * 0.5);
    let marker_w = 0.02;
    if uv.y > 0.8 && uv.y < 0.95 && abs(uv.x - marker_x) < marker_w {
        encoded = vec3<f32>(1.0);
    }
    return vec4<f32>(srgb_to_linear(encoded), 1.0);
}
