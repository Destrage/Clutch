// Built-in transitions (appended to transition_prelude.wgsl).
// kind: 0 cut, 1 fade, 2 fade to color, 3 slide, 4 swipe, 5 wipe.

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let uv = in.uv;
    let p = clamp(params.progress, 0.0, 1.0);
    let d = params.direction;
    switch params.kind {
        case 1u: {
            return mix(from_color(uv), to_color(uv), p);
        }
        case 2u: {
            if p < 0.5 {
                return mix(from_color(uv), params.color, p * 2.0);
            }
            return mix(params.color, to_color(uv), (p - 0.5) * 2.0);
        }
        case 3u: {
            // The new scene pushes the old one out along `d`.
            let e = ease(p);
            return over(to_color(uv + d * (1.0 - e)), from_color(uv - d * e));
        }
        case 4u: {
            // The new scene moves in over the static old one.
            let e = ease(p);
            return over(to_color(uv + d * (1.0 - e)), from_color(uv));
        }
        case 5u: {
            // A soft edge travels along `d`; 0 is the side it starts from.
            let s = dot(uv - vec2<f32>(0.5), d) + 0.5;
            let w = max(params.softness, 0.0001);
            let edge = ease(p) * (1.0 + w);
            let t = 1.0 - smoothstep(edge - w, edge, s);
            return mix(from_color(uv), to_color(uv), t);
        }
        default: {
            return to_color(uv);
        }
    }
}
