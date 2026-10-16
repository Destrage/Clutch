// Converts the linear-light canvas into output formats:
//  - fs_srgb: same-size copy into an *Srgb target (hardware does the encode)
//  - fs_y / fs_uv: BT.709 limited-range NV12 planes (Y full size, interleaved UV half size)

@group(0) @binding(0) var canvas: texture_2d<f32>;

@vertex
fn vs_fullscreen(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
    let uv = vec2<f32>(f32((index << 1u) & 2u), f32(index & 2u));
    return vec4<f32>(uv * 2.0 - 1.0, 0.0, 1.0);
}

fn linear_to_srgb(c: vec3<f32>) -> vec3<f32> {
    let lo = c * 12.92;
    let hi = 1.055 * pow(c, vec3<f32>(1.0 / 2.4)) - 0.055;
    return select(hi, lo, c <= vec3<f32>(0.0031308));
}

// Premultiplied canvas composited over black, then sRGB-encoded.
fn encoded(p: vec2<i32>) -> vec3<f32> {
    let c = textureLoad(canvas, p, 0);
    return linear_to_srgb(clamp(c.rgb, vec3<f32>(0.0), vec3<f32>(1.0)));
}

const KR: f32 = 0.2126;
const KB: f32 = 0.0722;

fn luma(rgb: vec3<f32>) -> f32 {
    return KR * rgb.r + (1.0 - KR - KB) * rgb.g + KB * rgb.b;
}

@fragment
fn fs_srgb(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    return textureLoad(canvas, vec2<i32>(pos.xy), 0);
}

@fragment
fn fs_y(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let y = luma(encoded(vec2<i32>(pos.xy)));
    return vec4<f32>((16.0 + 219.0 * y) / 255.0, 0.0, 0.0, 1.0);
}

@fragment
fn fs_uv(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let base = vec2<i32>(pos.xy) * 2;
    let rgb = (encoded(base) + encoded(base + vec2<i32>(1, 0))
             + encoded(base + vec2<i32>(0, 1)) + encoded(base + vec2<i32>(1, 1))) * 0.25;
    let y = luma(rgb);
    let cb = (rgb.b - y) / (2.0 * (1.0 - KB));
    let cr = (rgb.r - y) / (2.0 * (1.0 - KR));
    return vec4<f32>((128.0 + 224.0 * cb) / 255.0, (128.0 + 224.0 * cr) / 255.0, 0.0, 1.0);
}
