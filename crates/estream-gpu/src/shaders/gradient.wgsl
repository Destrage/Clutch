// Draws a linear or radial gradient once into an Rgba8Unorm texture that is sampled through an
// sRGB view. Stops blend in linear light (premultiplied), then each pixel is sRGB-encoded with a
// small per-pixel dither so the 8-bit result (and the encoder after it) shows no bands.

struct Uniform {
    // 0 linear, 1 radial
    kind: u32,
    count: u32,
    pad0: u32,
    pad1: u32,
    // linear: direction x, y and half the gradient length in pixels
    // radial: center x, y and radius in pixels
    geom: vec4<f32>,
    // width, height
    size: vec4<f32>,
    // linear-light, premultiplied
    colors: array<vec4<f32>, 8>,
    // stop positions, four per vector
    at: array<vec4<f32>, 2>,
}

@group(0) @binding(0) var<uniform> u: Uniform;

@vertex
fn vs_main(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
    let uv = vec2<f32>(f32((index << 1u) & 2u), f32(index & 2u));
    return vec4<f32>(uv * 2.0 - 1.0, 0.0, 1.0);
}

fn stop_at(i: u32) -> f32 {
    return u.at[i / 4u][i % 4u];
}

fn linear_to_srgb(c: vec3<f32>) -> vec3<f32> {
    let lo = c * 12.92;
    let hi = 1.055 * pow(c, vec3<f32>(1.0 / 2.4)) - 0.055;
    return select(hi, lo, c <= vec3<f32>(0.0031308));
}

@fragment
fn fs_main(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let p = pos.xy;
    var t: f32;
    if (u.kind == 0u) {
        t = dot(p - u.size.xy * 0.5, u.geom.xy) / (2.0 * u.geom.z) + 0.5;
    } else {
        t = length(p - u.geom.xy) / u.geom.z;
    }
    t = clamp(t, 0.0, 1.0);

    let last = u.count - 1u;
    var c = u.colors[0];
    if (t >= stop_at(last)) {
        c = u.colors[last];
    } else if (t > stop_at(0u)) {
        for (var i = 1u; i <= last; i = i + 1u) {
            let b = stop_at(i);
            if (t <= b) {
                let a = stop_at(i - 1u);
                let f = select((t - a) / (b - a), 1.0, b - a <= 1e-6);
                c = mix(u.colors[i - 1u], u.colors[i], f);
                break;
            }
        }
    }

    let alpha = c.a;
    var rgb = select(vec3<f32>(0.0), c.rgb / alpha, alpha > 0.0);
    rgb = linear_to_srgb(clamp(rgb, vec3<f32>(0.0), vec3<f32>(1.0)));
    // Interleaved gradient noise: +-0.5 of an 8-bit step, different for neighbouring pixels.
    let noise = fract(52.9829189 * fract(dot(p, vec2<f32>(0.06711056, 0.00583715)))) - 0.5;
    rgb = clamp(rgb + noise / 255.0, vec3<f32>(0.0), vec3<f32>(1.0));
    return vec4<f32>(rgb, alpha);
}
