// Built-in filters (appended to filter_prelude.wgsl).
// kind: 0 pass-through, 1 color correction, 2 LUT, 3 chroma key, 4 luma key, 5 crop/pad,
//       6 blur (horizontal), 7 blur (vertical), 8 sharpen, 9 image mask.

fn rotate_hue(c: vec3<f32>, degrees: f32) -> vec3<f32> {
    let a = radians(degrees);
    let k = vec3<f32>(0.57735027);
    let cos_a = cos(a);
    return c * cos_a + cross(k, c) * sin(a) + k * dot(k, c) * (1.0 - cos_a);
}

fn hue_rgb(degrees: f32) -> vec3<f32> {
    let h = fract(degrees / 360.0) * 6.0;
    return clamp(vec3<f32>(abs(h - 3.0) - 1.0, 2.0 - abs(h - 2.0), 2.0 - abs(h - 4.0)), vec3<f32>(0.0), vec3<f32>(1.0));
}

fn cbcr(c: vec3<f32>) -> vec2<f32> {
    let y = luma(c);
    return vec2<f32>((c.b - y) / 1.8556, (c.r - y) / 1.5748);
}

fn blur(uv: vec2<f32>, dir: vec2<f32>) -> vec4<f32> {
    let radius = clamp(param(0u), 0.0, 64.0);
    if radius < 0.5 {
        return source(uv);
    }
    let sigma = radius / 2.0;
    let taps = i32(ceil(radius));
    var sum = vec4<f32>(0.0);
    var total = 0.0;
    for (var i = -taps; i <= taps; i++) {
        let w = exp(-f32(i * i) / (2.0 * sigma * sigma));
        let s = source(uv + dir * f32(i));
        // Average premultiplied colors so transparent pixels don't darken edges.
        sum += vec4<f32>(s.rgb * s.a, s.a) * w;
        total += w;
    }
    let avg = sum / total;
    return vec4<f32>(select(vec3<f32>(0.0), avg.rgb / avg.a, avg.a > 0.0001), avg.a);
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let uv = in.uv;
    switch params.kind {
        case 1u: {
            // Color correction in perceptual (sRGB-encoded) space.
            let c = source(uv);
            var s = to_srgb(c.rgb);
            s = pow(max(s, vec3<f32>(0.0)), vec3<f32>(exp2(-param(3u))));
            s = (s - 0.5) * (1.0 + param(1u)) + 0.5 + param(0u);
            s = mix(vec3<f32>(luma(s)), s, param(2u));
            s = rotate_hue(s, param(4u));
            return vec4<f32>(to_linear(clamp(s, vec3<f32>(0.0), vec3<f32>(1.0))), c.a * param(5u));
        }
        case 2u: {
            // 3D LUT on sRGB-encoded values; param 1 is the LUT size.
            let c = source(uv);
            let n = max(param(1u), 2.0);
            let s = clamp(to_srgb(c.rgb), vec3<f32>(0.0), vec3<f32>(1.0));
            let coord = s * ((n - 1.0) / n) + 0.5 / n;
            let graded = textureSampleLevel(lut_tex, aux_sampler, coord, 0.0).rgb;
            return vec4<f32>(to_linear(mix(s, graded, param(0u))), c.a);
        }
        case 3u: {
            // Chroma key: distance from the key color in CbCr.
            let c = source(uv);
            let s = to_srgb(c.rgb);
            let key = cbcr(hue_rgb(param(0u)));
            let d = distance(cbcr(s), key);
            let similarity = param(1u);
            let smoothness = max(param(2u), 0.0001);
            let alpha = smoothstep(similarity, similarity + smoothness, d);
            // Spill: pull key-colored fringes towards grey.
            let spill = (1.0 - smoothstep(similarity, similarity + smoothness + 0.2, d)) * param(3u);
            let despilled = mix(s, vec3<f32>(luma(s)), clamp(spill, 0.0, 1.0));
            return vec4<f32>(to_linear(despilled), c.a * alpha);
        }
        case 4u: {
            let c = source(uv);
            let l = luma(to_srgb(c.rgb));
            let smooth_w = max(param(2u), 0.0001);
            let keep = smoothstep(param(0u) - smooth_w, param(0u), l)
                * (1.0 - smoothstep(param(1u), param(1u) + smooth_w, l));
            return vec4<f32>(c.rgb, c.a * keep);
        }
        case 5u: {
            // Crop/pad: map output pixels back to the input; outside it is transparent.
            let px = uv * params.out_size + params.offset;
            if any(px < vec2<f32>(0.0)) || any(px > params.in_size) {
                return vec4<f32>(0.0);
            }
            return source(px / params.in_size);
        }
        case 6u: {
            return blur(uv, vec2<f32>(1.0 / params.in_size.x, 0.0));
        }
        case 7u: {
            return blur(uv, vec2<f32>(0.0, 1.0 / params.in_size.y));
        }
        case 8u: {
            // Unsharp mask with a 3x3 box.
            let t = 1.0 / params.in_size;
            let c = source(uv);
            var avg = vec3<f32>(0.0);
            for (var y = -1; y <= 1; y++) {
                for (var x = -1; x <= 1; x++) {
                    avg += source(uv + vec2<f32>(f32(x), f32(y)) * t).rgb;
                }
            }
            avg /= 9.0;
            return vec4<f32>(max(c.rgb + (c.rgb - avg) * param(0u), vec3<f32>(0.0)), c.a);
        }
        case 9u: {
            let c = source(uv);
            let m = textureSampleLevel(mask_tex, aux_sampler, uv, 0.0);
            let f = select(m.a, luma(m.rgb) * m.a, param(0u) > 0.5);
            return vec4<f32>(c.rgb, c.a * f);
        }
        default: {
            return source(uv);
        }
    }
}
