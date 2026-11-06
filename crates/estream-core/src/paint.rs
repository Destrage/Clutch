//! Solid colors and gradients, shared by the Color source, item fill and canvas backgrounds.

use crate::color::{Rgba8, linear_to_srgb, srgb_to_linear};
use serde::{Deserialize, Serialize};

/// Most stops a gradient keeps.
pub const MAX_STOPS: usize = 8;

/// One color along a gradient. `at` runs from 0 (start) to 1 (end).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct GradientStop {
    pub color: Rgba8,
    pub at: f32,
}

impl GradientStop {
    pub const fn new(color: Rgba8, at: f32) -> Self {
        Self { color, at }
    }
}

/// A fill: one color, or a gradient through 2 to [`MAX_STOPS`] stops.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Paint {
    Solid {
        color: Rgba8,
    },
    /// `angle` in degrees, measured like CSS: 0 runs bottom to top, 90 left to right.
    Linear {
        angle: f32,
        stops: Vec<GradientStop>,
    },
    /// A circle around `center` (0..1 of the width and height). `radius` 1 reaches the
    /// farthest corner.
    Radial {
        center: [f32; 2],
        radius: f32,
        stops: Vec<GradientStop>,
    },
}

impl Default for Paint {
    fn default() -> Self {
        Self::Solid {
            color: Rgba8::BLACK,
        }
    }
}

impl Paint {
    pub const fn solid(color: Rgba8) -> Self {
        Self::Solid { color }
    }

    /// A left-to-right gradient between two colors.
    pub fn linear(from: Rgba8, to: Rgba8) -> Self {
        Self::Linear {
            angle: 90.0,
            stops: vec![GradientStop::new(from, 0.0), GradientStop::new(to, 1.0)],
        }
    }

    pub fn is_gradient(&self) -> bool {
        !matches!(self, Self::Solid { .. })
    }

    pub fn stops(&self) -> &[GradientStop] {
        match self {
            Self::Solid { .. } => &[],
            Self::Linear { stops, .. } | Self::Radial { stops, .. } => stops,
        }
    }

    /// The solid color, or the first stop's: what a single color stands in for this paint
    /// (older versions, small swatches).
    pub fn first_color(&self) -> Rgba8 {
        match self {
            Self::Solid { color } => *color,
            _ => self.stops().first().map_or(Rgba8::BLACK, |s| s.color),
        }
    }

    /// The same stops as a linear gradient, keeping the angle when it already is one. A solid
    /// paint becomes a gradient from its color to itself.
    pub fn to_linear(&self) -> Self {
        match self {
            Self::Linear { .. } => self.clone(),
            _ => Self::Linear {
                angle: 90.0,
                stops: self.stops_or_solid(),
            },
        }
    }

    /// The same stops as a radial gradient from the center, keeping center and radius when it
    /// already is one.
    pub fn to_radial(&self) -> Self {
        match self {
            Self::Radial { .. } => self.clone(),
            _ => Self::Radial {
                center: [0.5, 0.5],
                radius: 1.0,
                stops: self.stops_or_solid(),
            },
        }
    }

    fn stops_or_solid(&self) -> Vec<GradientStop> {
        match self {
            Self::Solid { color } => {
                vec![
                    GradientStop::new(*color, 0.0),
                    GradientStop::new(*color, 1.0),
                ]
            }
            _ => self.stops().to_vec(),
        }
    }

    /// The same paint made safe to render: stops clamped to 0..1, sorted, at most
    /// [`MAX_STOPS`]; angle, center and radius finite and in range. A gradient with fewer than
    /// two stops becomes solid.
    pub fn normalized(&self) -> Self {
        let tidy = |stops: &[GradientStop]| {
            let mut stops: Vec<GradientStop> = stops
                .iter()
                .map(|s| GradientStop::new(s.color, finite_or(s.at, 0.0).clamp(0.0, 1.0)))
                .collect();
            stops.sort_by(|a, b| a.at.total_cmp(&b.at));
            stops.truncate(MAX_STOPS);
            stops
        };
        match self {
            Self::Solid { color } => Self::Solid { color: *color },
            Self::Linear { angle, stops } => {
                let stops = tidy(stops);
                if stops.len() < 2 {
                    return Self::solid(stops.first().map_or(Rgba8::BLACK, |s| s.color));
                }
                Self::Linear {
                    angle: finite_or(*angle, 90.0).rem_euclid(360.0),
                    stops,
                }
            }
            Self::Radial {
                center,
                radius,
                stops,
            } => {
                let stops = tidy(stops);
                if stops.len() < 2 {
                    return Self::solid(stops.first().map_or(Rgba8::BLACK, |s| s.color));
                }
                Self::Radial {
                    center: center.map(|c| finite_or(c, 0.5).clamp(-1.0, 2.0)),
                    radius: finite_or(*radius, 1.0).clamp(0.01, 4.0),
                    stops,
                }
            }
        }
    }

    /// The color at `t` (0..1) along the gradient, blended in linear light like the renderer.
    /// A solid paint is the same everywhere.
    pub fn color_at(&self, t: f32) -> Rgba8 {
        let stops = self.stops();
        match stops {
            [] => self.first_color(),
            [only] => only.color,
            _ => {
                let t = t.clamp(0.0, 1.0);
                let i = stops.iter().position(|s| s.at >= t).unwrap_or(stops.len());
                if i == 0 {
                    return stops[0].color;
                }
                if i == stops.len() {
                    return stops[i - 1].color;
                }
                let (a, b) = (stops[i - 1], stops[i]);
                let span = b.at - a.at;
                let f = if span <= f32::EPSILON {
                    1.0
                } else {
                    (t - a.at) / span
                };
                from_premul(mix(to_premul(a.color), to_premul(b.color), f))
            }
        }
    }
}

fn finite_or(v: f32, fallback: f32) -> f32 {
    if v.is_finite() { v } else { fallback }
}

/// Linear-light, premultiplied components of a straight sRGB color.
pub fn to_premul(c: Rgba8) -> [f32; 4] {
    let a = c.a as f32 / 255.0;
    let l = |v: u8| srgb_to_linear(v as f32 / 255.0) * a;
    [l(c.r), l(c.g), l(c.b), a]
}

fn from_premul(p: [f32; 4]) -> Rgba8 {
    let a = p[3].clamp(0.0, 1.0);
    let e = |v: f32| {
        let straight = if a > 0.0 { v / a } else { 0.0 };
        (linear_to_srgb(straight.clamp(0.0, 1.0)) * 255.0).round() as u8
    };
    Rgba8::new(e(p[0]), e(p[1]), e(p[2]), (a * 255.0).round() as u8)
}

fn mix(a: [f32; 4], b: [f32; 4], f: f32) -> [f32; 4] {
    std::array::from_fn(|i| a[i] + (b[i] - a[i]) * f)
}

#[cfg(test)]
mod tests {
    use super::*;

    const RED: Rgba8 = Rgba8::new(255, 0, 0, 255);
    const BLUE: Rgba8 = Rgba8::new(0, 0, 255, 255);

    #[test]
    fn serde_round_trip_and_shape() {
        let p = Paint::linear(RED, BLUE);
        let json = serde_json::to_value(&p).unwrap();
        assert_eq!(json["kind"], "linear");
        assert_eq!(json["stops"][1]["at"], 1.0);
        assert_eq!(serde_json::from_value::<Paint>(json).unwrap(), p);
        let solid =
            serde_json::json!({ "kind": "solid", "color": { "r": 1, "g": 2, "b": 3, "a": 4 } });
        assert_eq!(
            serde_json::from_value::<Paint>(solid).unwrap(),
            Paint::solid(Rgba8::new(1, 2, 3, 4))
        );
    }

    #[test]
    fn normalized_sorts_clamps_and_limits() {
        let stops: Vec<_> = (0..12)
            .map(|i| GradientStop::new(RED, 1.5 - i as f32 * 0.2))
            .collect();
        let p = Paint::Linear {
            angle: -90.0,
            stops,
        }
        .normalized();
        let Paint::Linear { angle, stops } = &p else {
            panic!("still linear")
        };
        assert_eq!(*angle, 270.0);
        assert_eq!(stops.len(), MAX_STOPS);
        assert!(stops.windows(2).all(|w| w[0].at <= w[1].at));
        assert!(stops.iter().all(|s| (0.0..=1.0).contains(&s.at)));
    }

    #[test]
    fn one_stop_is_solid() {
        let p = Paint::Radial {
            center: [f32::NAN, 0.5],
            radius: 1.0,
            stops: vec![GradientStop::new(BLUE, 0.3)],
        };
        assert_eq!(p.normalized(), Paint::solid(BLUE));
    }

    #[test]
    fn color_at_blends_in_linear_light() {
        let p = Paint::linear(RED, BLUE);
        assert_eq!(p.color_at(0.0), RED);
        assert_eq!(p.color_at(1.0), BLUE);
        // Halfway in linear light is 0.5 per channel, 188 in sRGB (not 128).
        assert_eq!(p.color_at(0.5), Rgba8::new(188, 0, 188, 255));
    }

    #[test]
    fn color_at_with_transparency_keeps_the_color() {
        let clear_red = Rgba8::new(255, 0, 0, 0);
        let p = Paint::linear(clear_red, RED);
        let mid = p.color_at(0.5);
        assert_eq!((mid.r, mid.g, mid.b), (255, 0, 0), "no dark fringe");
        assert!((126..=129).contains(&mid.a));
    }

    #[test]
    fn switching_shape_keeps_the_stops() {
        let linear = Paint::linear(RED, BLUE);
        let radial = linear.to_radial();
        assert_eq!(radial.stops(), linear.stops());
        assert!(matches!(radial, Paint::Radial { radius: 1.0, .. }));
        assert_eq!(radial.to_linear().stops(), linear.stops());
        assert_eq!(linear.to_linear(), linear);
        assert_eq!(Paint::solid(RED).to_linear().stops().len(), 2);
    }

    #[test]
    fn first_color_stands_in_for_old_versions() {
        assert_eq!(Paint::linear(RED, BLUE).first_color(), RED);
        assert_eq!(Paint::solid(BLUE).first_color(), BLUE);
    }
}
