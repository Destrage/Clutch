//! Item motion in transitions: an item on its way between two placements, and how show and
//! hide animations change an item. Pure math; the video thread applies the results.

use estream_project::{AnimationKind, Direction};
use glam::{Affine2, Mat2, Vec2};

/// An item matrix (unit quad → canvas pixels) split into parts that interpolate well.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Parts {
    /// Where the quad's center lands.
    center: Vec2,
    /// Radians.
    angle: f32,
    /// Width and height (height negative when flipped).
    scale: Vec2,
    shear: f32,
}

fn decompose(m: Affine2) -> Parts {
    let x = m.matrix2.x_axis;
    let y = m.matrix2.y_axis;
    let angle = x.y.atan2(x.x);
    let (s, c) = angle.sin_cos();
    // The y axis in the rotated frame: R⁻¹·y = (shear, height).
    let ry = Vec2::new(c * y.x + s * y.y, -s * y.x + c * y.y);
    Parts {
        center: m.transform_point2(Vec2::splat(0.5)),
        angle,
        scale: Vec2::new(x.length(), ry.y),
        shear: ry.x,
    }
}

fn compose(p: Parts) -> Affine2 {
    let linear = Mat2::from_angle(p.angle)
        * Mat2::from_cols(Vec2::new(p.scale.x, 0.0), Vec2::new(p.shear, p.scale.y));
    Affine2::from_mat2_translation(linear, p.center - linear * Vec2::splat(0.5))
}

/// Sizes change by the same ratio each step (log space), so growing looks even.
fn scale_between(a: f32, b: f32, t: f32) -> f32 {
    if a > 1e-3 && b > 1e-3 {
        a * (b / a).powf(t)
    } else if a < -1e-3 && b < -1e-3 {
        -((-a) * (b / a).powf(t))
    } else {
        a + (b - a) * t
    }
}

/// The turn from `a` to `b` the short way round, in (-π, π].
fn short_turn(a: f32, b: f32) -> f32 {
    let tau = std::f32::consts::TAU;
    let d = (b - a).rem_euclid(tau);
    if d > std::f32::consts::PI { d - tau } else { d }
}

/// The item matrix at eased progress `t` (0 = `a`, 1 = `b`; a spring may go slightly past).
/// Its center travels on a straight line, or on an arc bulging upward when `curved`.
pub fn between(a: Affine2, b: Affine2, t: f32, curved: bool) -> Affine2 {
    if !t.is_finite() {
        return a;
    }
    let (pa, pb) = (decompose(a), decompose(b));
    let path = pb.center - pa.center;
    let straight = pa.center + path * t;
    let center = if curved && path.length() > 1.0 {
        // Quadratic Bézier through a control point off the middle of the path.
        let mut normal = path.perp().normalize();
        if normal.y > 0.0 {
            normal = -normal; // canvas y points down; bulge up
        }
        let control = (pa.center + pb.center) * 0.5 + normal * path.length() * 0.2;
        let u = 1.0 - t;
        pa.center * (u * u) + control * (2.0 * u * t) + pb.center * (t * t)
    } else {
        straight
    };
    compose(Parts {
        center,
        angle: pa.angle + short_turn(pa.angle, pb.angle) * t,
        scale: Vec2::new(
            scale_between(pa.scale.x, pb.scale.x, t),
            scale_between(pa.scale.y, pb.scale.y, t),
        ),
        shear: pa.shear + (pb.shear - pa.shear) * t,
    })
}

/// The change an animation makes to an item this frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Look {
    /// Applied in canvas space after the item's own matrix (so effect layers and fills follow).
    pub delta: Affine2,
    /// Multiplies the item's opacity.
    pub opacity: f32,
    /// The drawn part of the item's unit square.
    pub clip: [f32; 4],
}

impl Look {
    pub const NONE: Look = Look {
        delta: Affine2::IDENTITY,
        opacity: 1.0,
        clip: [0.0, 0.0, 1.0, 1.0],
    };
}

/// How an item with `matrix` looks at `presence` (0 = gone, 1 = in place) of a show
/// (`showing`) or hide animation of `kind`, on a canvas of `canvas` pixels.
pub fn animate(
    kind: AnimationKind,
    showing: bool,
    presence: f32,
    matrix: Affine2,
    canvas: Vec2,
) -> Look {
    let presence = if presence.is_finite() { presence } else { 1.0 };
    let p = presence.clamp(0.0, 1.0);
    let center = matrix.transform_point2(Vec2::splat(0.5));
    let around_center = |s: f32| {
        Affine2::from_translation(center)
            * Affine2::from_scale(Vec2::splat(s.max(0.0)))
            * Affine2::from_translation(-center)
    };
    match kind {
        AnimationKind::Fade => Look {
            opacity: p,
            ..Look::NONE
        },
        AnimationKind::Zoom => Look {
            delta: around_center(presence),
            ..Look::NONE
        },
        AnimationKind::Pop => {
            // Back-out curve: overshoots about 10 % before settling at full size.
            const C1: f32 = 1.701_58;
            let x = p - 1.0;
            let s = 1.0 + (C1 + 1.0) * x * x * x + C1 * x * x;
            Look {
                delta: around_center(s),
                opacity: (p * 3.0).min(1.0),
                ..Look::NONE
            }
        }
        AnimationKind::Slide { edge } => {
            let corners =
                [Vec2::ZERO, Vec2::X, Vec2::Y, Vec2::ONE].map(|c| matrix.transform_point2(c));
            let lo = corners.iter().fold(Vec2::INFINITY, |m, c| m.min(*c));
            let hi = corners.iter().fold(Vec2::NEG_INFINITY, |m, c| m.max(*c));
            // Fully off the canvas, with a little room for glows and shadows.
            let margin = canvas.max_element() * 0.02;
            let away = match edge {
                Direction::Left => Vec2::new(-(hi.x + margin), 0.0),
                Direction::Right => Vec2::new(canvas.x - lo.x + margin, 0.0),
                Direction::Up => Vec2::new(0.0, -(hi.y + margin)),
                Direction::Down => Vec2::new(0.0, canvas.y - lo.y + margin),
            };
            Look {
                delta: Affine2::from_translation(away * (1.0 - presence)),
                ..Look::NONE
            }
        }
        AnimationKind::Wipe { direction } => {
            // The edge travels in `direction`: showing reveals behind it, hiding removes ahead.
            let q = 1.0 - p;
            let clip = match (direction, showing) {
                (Direction::Right, true) | (Direction::Left, false) => [0.0, 0.0, p, 1.0],
                (Direction::Left, true) | (Direction::Right, false) => [q, 0.0, 1.0, 1.0],
                (Direction::Down, true) | (Direction::Up, false) => [0.0, 0.0, 1.0, p],
                (Direction::Up, true) | (Direction::Down, false) => [0.0, q, 1.0, 1.0],
            };
            Look { clip, ..Look::NONE }
        }
    }
}

/// `clip` (over an item's unit square) for a layer that covers the item grown by `pad` on each
/// side. Sides that aren't clipped stay open, so a glow past the item's edge isn't cut.
pub fn padded_clip(clip: [f32; 4], pad: Vec2) -> [f32; 4] {
    let inner = |v: f32, pad: f32| (v + pad) / (1.0 + 2.0 * pad);
    [
        if clip[0] <= 0.0 {
            0.0
        } else {
            inner(clip[0], pad.x)
        },
        if clip[1] <= 0.0 {
            0.0
        } else {
            inner(clip[1], pad.y)
        },
        if clip[2] >= 1.0 {
            1.0
        } else {
            inner(clip[2], pad.x)
        },
        if clip[3] >= 1.0 {
            1.0
        } else {
            inner(clip[3], pad.y)
        },
    ]
}

/// 0 below `lo`, 1 above `hi`, and a smooth S-curve between.
pub fn smoothstep(lo: f32, hi: f32, x: f32) -> f32 {
    let t = ((x - lo) / (hi - lo)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// The canvas-space change that takes `from` to `to` (identity when `from` is degenerate).
pub fn delta(from: Affine2, to: Affine2) -> Affine2 {
    if from.matrix2.determinant().abs() < 1e-9 {
        Affine2::IDENTITY
    } else {
        to * from.inverse()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn placed(x: f32, y: f32, w: f32, h: f32, degrees: f32) -> Affine2 {
        Affine2::from_translation(Vec2::new(x, y))
            * Affine2::from_angle(degrees.to_radians())
            * Affine2::from_scale(Vec2::new(w, h))
    }

    fn near(a: Affine2, b: Affine2) -> bool {
        a.abs_diff_eq(b, 1e-2)
    }

    #[test]
    fn ends_are_exact_and_the_middle_is_halfway() {
        let a = placed(40.0, 40.0, 480.0, 270.0, 0.0);
        let b = placed(720.0, 300.0, 960.0, 540.0, 0.0);
        assert!(near(between(a, b, 0.0, false), a));
        assert!(near(between(a, b, 1.0, false), b));
        let mid = between(a, b, 0.5, false);
        let center = mid.transform_point2(Vec2::splat(0.5));
        let expected =
            (a.transform_point2(Vec2::splat(0.5)) + b.transform_point2(Vec2::splat(0.5))) * 0.5;
        assert!(center.abs_diff_eq(expected, 1e-2), "{center} vs {expected}");
        // Log-space size: halfway between 480 and 960 wide is √(480·960).
        let width = mid.matrix2.x_axis.length();
        assert!((width - (480.0f32 * 960.0).sqrt()).abs() < 0.5, "{width}");
    }

    #[test]
    fn rotation_takes_the_short_way() {
        let a = placed(0.0, 0.0, 100.0, 100.0, 350.0);
        let b = placed(0.0, 0.0, 100.0, 100.0, 10.0);
        let mid = between(a, b, 0.5, false);
        let angle = mid
            .matrix2
            .x_axis
            .y
            .atan2(mid.matrix2.x_axis.x)
            .to_degrees();
        assert!(angle.abs() < 0.01, "turned through 0°, not 180°: {angle}");
    }

    #[test]
    fn flips_and_shear_survive_the_round_trip() {
        let flipped = placed(10.0, 20.0, 300.0, -200.0, 30.0);
        assert!(near(compose(decompose(flipped)), flipped));
        let sheared = Affine2::from_cols_array(&[2.0, 0.5, 0.3, 1.5, 7.0, 9.0]);
        assert!(near(compose(decompose(sheared)), sheared));
    }

    #[test]
    fn curved_paths_bulge_up() {
        let a = placed(0.0, 500.0, 100.0, 100.0, 0.0);
        let b = placed(1000.0, 500.0, 100.0, 100.0, 0.0);
        let straight = between(a, b, 0.5, false).transform_point2(Vec2::splat(0.5));
        let arc = between(a, b, 0.5, true).transform_point2(Vec2::splat(0.5));
        assert!((straight.x - arc.x).abs() < 1e-3);
        assert!(arc.y < straight.y - 50.0, "{arc} vs {straight}");
        assert!(near(between(a, b, 1.0, true), b));
    }

    #[test]
    fn slide_leaves_the_canvas_and_zoom_shrinks_to_the_center() {
        let canvas = Vec2::new(1920.0, 1080.0);
        let m = placed(100.0, 100.0, 400.0, 300.0, 0.0);
        let gone = animate(
            AnimationKind::Slide {
                edge: Direction::Left,
            },
            false,
            0.0,
            m,
            canvas,
        );
        let right_edge = (gone.delta * m).transform_point2(Vec2::ONE).x;
        assert!(right_edge < 0.0, "{right_edge}");
        assert_eq!(
            animate(
                AnimationKind::Slide {
                    edge: Direction::Left
                },
                true,
                1.0,
                m,
                canvas
            )
            .delta,
            Affine2::IDENTITY
        );
        let half = animate(AnimationKind::Zoom, true, 0.5, m, canvas);
        let moved = half.delta * m;
        assert!(
            moved
                .transform_point2(Vec2::splat(0.5))
                .abs_diff_eq(Vec2::new(300.0, 250.0), 1e-3)
        );
        assert!((moved.matrix2.x_axis.length() - 200.0).abs() < 1e-3);
        assert_eq!(
            animate(AnimationKind::Fade, false, 0.25, m, canvas).opacity,
            0.25
        );
    }

    #[test]
    fn pop_overshoots_and_wipes_clip_the_right_side() {
        let canvas = Vec2::new(100.0, 100.0);
        let m = placed(0.0, 0.0, 10.0, 10.0, 0.0);
        let sizes: Vec<f32> = (0..=20)
            .map(|i| {
                let look = animate(AnimationKind::Pop, true, i as f32 / 20.0, m, canvas);
                (look.delta * m).matrix2.x_axis.length()
            })
            .collect();
        assert!(sizes[0] < 1e-3 && (sizes[20] - 10.0).abs() < 1e-3);
        assert!(sizes.iter().any(|s| *s > 10.5), "{sizes:?}");
        let right = AnimationKind::Wipe {
            direction: Direction::Right,
        };
        assert_eq!(
            animate(right, true, 0.25, m, canvas).clip,
            [0.0, 0.0, 0.25, 1.0]
        );
        assert_eq!(
            animate(right, false, 0.25, m, canvas).clip,
            [0.75, 0.0, 1.0, 1.0]
        );
        assert_eq!(
            padded_clip([0.5, 0.0, 1.0, 1.0], Vec2::splat(0.25)),
            [0.5, 0.0, 1.0, 1.0]
        );
        assert_eq!(
            padded_clip([0.0, 0.0, 0.5, 1.0], Vec2::splat(0.5)),
            [0.0, 0.0, 0.5, 1.0]
        );
    }
}
