// NV12 (Y plane + interleaved CbCr plane) → RGB, written to an *Srgb target.

struct Params {
    // Kr, Kb luma coefficients (BT.601: 0.299/0.114, BT.709: 0.2126/0.0722).
    kr: f32,
    kb: f32,
    // 1 = full range (0-255), 0 = limited range (16-235 / 16-240).
    full_range: u32,
    _pad: u32,
};

@group(0) @binding(0) var y_plane: texture_2d<f32>;
@group(0) @binding(1) var uv_plane: texture_2d<f32>;
@group(0) @binding(2) var<uniform> params: Params;

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
    let p = vec2<i32>(pos.xy);
    let y_raw = textureLoad(y_plane, p, 0).r * 255.0;
    let c_raw = textureLoad(uv_plane, p / 2, 0).rg * 255.0;
    var y: f32;
    var c: vec2<f32>;
    if params.full_range == 1u {
        y = y_raw / 255.0;
        c = (c_raw - 128.0) / 255.0;
    } else {
        y = (y_raw - 16.0) / 219.0;
        c = (c_raw - 128.0) / 224.0;
    }
    let kr = params.kr;
    let kb = params.kb;
    let r = y + 2.0 * (1.0 - kr) * c.y;
    let b = y + 2.0 * (1.0 - kb) * c.x;
    let g = (y - kr * r - kb * b) / (1.0 - kr - kb);
    let encoded = clamp(vec3<f32>(r, g, b), vec3<f32>(0.0), vec3<f32>(1.0));
    return vec4<f32>(srgb_to_linear(encoded), 1.0);
}
