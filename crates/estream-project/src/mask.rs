//! Item masks: shapes that make parts of a scene item transparent, and edge effects (border,
//! glow, shadow) that follow the visible shape.
//!
//! Shape coordinates are 0..1 over the item's visible source (after crop), so a mask moves,
//! scales and rotates with its item. Lengths (corner radius, feather, effect sizes) are canvas
//! pixels at the item's current size.

use estream_core::Rgba8;
use glam::Vec2;
use serde::{Deserialize, Serialize};

/// Most shapes one mask may hold.
pub const MAX_SHAPES: usize = 64;
/// Most points one polygon may hold (freehand strokes are simplified well below this).
pub const MAX_POINTS: usize = 4096;
/// Largest length (radius, feather, effect size or offset) in canvas pixels.
pub const MAX_LENGTH: f32 = 1000.0;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ItemMask {
    /// Off draws the item whole (effects then follow its rectangle).
    #[serde(default = "yes")]
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub invert: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub shapes: Vec<MaskShape>,
    #[serde(default, skip_serializing_if = "MaskEffects::is_empty")]
    pub effects: MaskEffects,
}

impl Default for ItemMask {
    fn default() -> Self {
        Self {
            enabled: true,
            invert: false,
            shapes: Vec::new(),
            effects: MaskEffects::default(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MaskShape {
    #[serde(flatten)]
    pub geometry: MaskGeometry,
    #[serde(default)]
    pub mode: MaskMode,
    /// Width of the soft edge, in canvas pixels.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub feather: f32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "shape", rename_all = "snake_case")]
pub enum MaskGeometry {
    /// Corner `radius` in canvas pixels.
    Rect {
        min: Vec2,
        max: Vec2,
        radius: f32,
    },
    Ellipse {
        center: Vec2,
        radii: Vec2,
    },
    /// Clicked points or a simplified freehand stroke; closed.
    Polygon {
        points: Vec<Vec2>,
    },
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MaskMode {
    #[default]
    Keep,
    Cut,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct MaskEffects {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub border: Option<Border>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub glow: Option<Glow>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shadow: Option<Shadow>,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Border {
    pub color: Rgba8,
    pub width: f32,
    #[serde(default)]
    pub placement: BorderPlacement,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BorderPlacement {
    Inside,
    Center,
    #[default]
    Outside,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Glow {
    pub color: Rgba8,
    /// How far the glow reaches from the edge, in canvas pixels.
    pub size: f32,
    /// 0..1: at 1 the glow is fully opaque at the edge.
    pub strength: f32,
    #[serde(default)]
    pub placement: GlowPlacement,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GlowPlacement {
    #[default]
    Outside,
    Inside,
    Both,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Shadow {
    pub color: Rgba8,
    /// Canvas pixels; positive is right and down.
    pub offset: Vec2,
    /// Blur reach in canvas pixels.
    pub softness: f32,
    /// 0..1.
    pub opacity: f32,
}

impl MaskEffects {
    pub fn is_empty(&self) -> bool {
        self.border.is_none() && self.glow.is_none() && self.shadow.is_none()
    }

    /// How far the effects reach outside the shape, in canvas pixels.
    pub fn extent(&self) -> f32 {
        let border = self.border.map_or(0.0, |b| match b.placement {
            BorderPlacement::Inside => 0.0,
            BorderPlacement::Center => b.width * 0.5,
            BorderPlacement::Outside => b.width,
        });
        let glow = self.glow.map_or(0.0, |g| match g.placement {
            GlowPlacement::Inside => 0.0,
            _ => g.size,
        });
        let shadow = self
            .shadow
            .map_or(0.0, |s| s.offset.abs().max_element() + s.softness);
        border.max(glow).max(shadow).clamp(0.0, 2.0 * MAX_LENGTH)
    }

    /// Effects drawn below the item (shadow, outer glow).
    pub fn has_under(&self) -> bool {
        self.shadow.is_some()
            || self
                .glow
                .is_some_and(|g| g.placement != GlowPlacement::Inside)
    }

    /// Effects drawn above the item (inner glow, border).
    pub fn has_over(&self) -> bool {
        self.border.is_some()
            || self
                .glow
                .is_some_and(|g| g.placement != GlowPlacement::Outside)
    }

    /// Soft shadow style: a dark shadow just below the shape.
    pub fn soft_shadow() -> Self {
        Self {
            shadow: Some(Shadow {
                color: Rgba8::BLACK,
                offset: Vec2::new(0.0, 8.0),
                softness: 24.0,
                opacity: 0.6,
            }),
            ..Self::default()
        }
    }

    /// Neon glow style: a bright cyan glow outside a thin line of the same color.
    pub fn neon_glow() -> Self {
        let color = Rgba8::new(0, 229, 255, 255);
        Self {
            glow: Some(Glow {
                color,
                size: 24.0,
                strength: 1.0,
                placement: GlowPlacement::Outside,
            }),
            border: Some(Border {
                color,
                width: 2.0,
                placement: BorderPlacement::Center,
            }),
            shadow: None,
        }
    }

    /// Thin border style: a white line just outside the shape.
    pub fn thin_border() -> Self {
        Self {
            border: Some(Border {
                color: Rgba8::WHITE,
                width: 3.0,
                placement: BorderPlacement::Outside,
            }),
            ..Self::default()
        }
    }
}

impl MaskShape {
    pub fn keep(geometry: MaskGeometry) -> Self {
        Self {
            geometry,
            mode: MaskMode::Keep,
            feather: 0.0,
        }
    }

    pub fn cut(geometry: MaskGeometry) -> Self {
        Self {
            mode: MaskMode::Cut,
            ..Self::keep(geometry)
        }
    }
}

impl MaskGeometry {
    /// The whole item.
    pub const FULL: Self = Self::Rect {
        min: Vec2::ZERO,
        max: Vec2::ONE,
        radius: 0.0,
    };

    /// The smallest box holding the shape, in item units.
    pub fn bounds(&self) -> (Vec2, Vec2) {
        match self {
            Self::Rect { min, max, .. } => (min.min(*max), min.max(*max)),
            Self::Ellipse { center, radii } => {
                let r = radii.abs();
                (*center - r, *center + r)
            }
            Self::Polygon { points } => points.iter().fold(
                (Vec2::splat(f32::MAX), Vec2::splat(f32::MIN)),
                |(lo, hi), p| (lo.min(*p), hi.max(*p)),
            ),
        }
    }
}

impl ItemMask {
    /// The shapes cut the item (so it needs a mask texture).
    pub fn cuts(&self) -> bool {
        self.enabled && !self.shapes.is_empty()
    }

    /// Anything to draw differently from a plain item.
    pub fn draws(&self) -> bool {
        self.cuts() || !self.effects.is_empty()
    }

    /// Circle preset: the largest circle centered in an item of `aspect` (width / height).
    pub fn circle(aspect: f32) -> Self {
        let aspect = if aspect.is_finite() && aspect > 0.0 {
            aspect
        } else {
            1.0
        };
        let radii = if aspect >= 1.0 {
            Vec2::new(0.5 / aspect, 0.5)
        } else {
            Vec2::new(0.5, 0.5 * aspect)
        };
        Self::with_shapes(vec![MaskShape::keep(MaskGeometry::Ellipse {
            center: Vec2::splat(0.5),
            radii,
        })])
    }

    /// Rounded corners preset: the whole item with `radius` canvas pixels corners.
    pub fn rounded(radius: f32) -> Self {
        Self::with_shapes(vec![MaskShape::keep(MaskGeometry::Rect {
            min: Vec2::ZERO,
            max: Vec2::ONE,
            radius: radius.max(0.0),
        })])
    }

    /// Cut a corner preset: the top right corner cut away (a minimap or HUD corner).
    pub fn cut_corner() -> Self {
        Self::with_shapes(vec![MaskShape::cut(MaskGeometry::Rect {
            min: Vec2::new(0.75, 0.0),
            max: Vec2::new(1.0, 0.3),
            radius: 0.0,
        })])
    }

    /// Diagonal split preset: the left part kept up to a slanted line.
    pub fn diagonal_split() -> Self {
        Self::with_shapes(vec![MaskShape::keep(MaskGeometry::Polygon {
            points: vec![
                Vec2::new(0.0, 0.0),
                Vec2::new(0.6, 0.0),
                Vec2::new(0.4, 1.0),
                Vec2::new(0.0, 1.0),
            ],
        })])
    }

    fn with_shapes(shapes: Vec<MaskShape>) -> Self {
        Self {
            shapes,
            ..Self::default()
        }
    }

    /// Checks that every number is usable. Returns what is wrong.
    pub fn validate(&self) -> Result<(), String> {
        if self.shapes.len() > MAX_SHAPES {
            return Err(format!("a mask holds at most {MAX_SHAPES} shapes"));
        }
        let length = |what: &str, v: f32| {
            if v.is_finite() && (0.0..=MAX_LENGTH).contains(&v) {
                Ok(())
            } else {
                Err(format!("{what} {v} is outside 0..={MAX_LENGTH}"))
            }
        };
        let point = |p: Vec2| {
            if p.is_finite() && p.abs().max_element() <= 100.0 {
                Ok(())
            } else {
                Err(format!("point {p} is out of range"))
            }
        };
        let unit = |what: &str, v: f32| {
            if v.is_finite() && (0.0..=1.0).contains(&v) {
                Ok(())
            } else {
                Err(format!("{what} {v} is outside 0..=1"))
            }
        };
        for shape in &self.shapes {
            length("feather", shape.feather)?;
            match &shape.geometry {
                MaskGeometry::Rect { min, max, radius } => {
                    point(*min)?;
                    point(*max)?;
                    length("radius", *radius)?;
                }
                MaskGeometry::Ellipse { center, radii } => {
                    point(*center)?;
                    point(*radii)?;
                }
                MaskGeometry::Polygon { points } => {
                    if !(3..=MAX_POINTS).contains(&points.len()) {
                        return Err(format!("a polygon needs 3..={MAX_POINTS} points"));
                    }
                    points.iter().try_for_each(|p| point(*p))?;
                }
            }
        }
        let e = &self.effects;
        if let Some(b) = e.border {
            length("border width", b.width)?;
        }
        if let Some(g) = e.glow {
            length("glow size", g.size)?;
            unit("glow strength", g.strength)?;
        }
        if let Some(s) = e.shadow {
            length("shadow softness", s.softness)?;
            length("shadow offset", s.offset.x.abs())?;
            length("shadow offset", s.offset.y.abs())?;
            unit("shadow opacity", s.opacity)?;
        }
        Ok(())
    }
}

/// Coverage before the first shape: nothing, or everything when the first shape cuts.
pub fn coverage_start(shapes: &[MaskShape]) -> f32 {
    match shapes.first() {
        Some(s) if s.mode == MaskMode::Cut => 1.0,
        _ => 0.0,
    }
}

/// Combines one shape's coverage (0..1 per pixel) into `acc`: keep = max, cut = acc × (1 − shape).
pub fn combine(acc: &mut [f32], shape: &[f32], mode: MaskMode) {
    match mode {
        MaskMode::Keep => acc.iter_mut().zip(shape).for_each(|(a, s)| *a = a.max(*s)),
        MaskMode::Cut => acc
            .iter_mut()
            .zip(shape)
            .for_each(|(a, s)| *a *= 1.0 - s.clamp(0.0, 1.0)),
    }
}

/// Flips the coverage for an inverted mask.
pub fn invert(acc: &mut [f32]) {
    acc.iter_mut().for_each(|a| *a = 1.0 - a.clamp(0.0, 1.0));
}

fn yes() -> bool {
    true
}

fn is_false(v: &bool) -> bool {
    !*v
}

fn is_zero(v: &f32) -> bool {
    *v == 0.0
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Coverage of an axis-aligned box on a `n`×`n` grid of cell centers (hard edges).
    fn grid(n: usize, min: Vec2, max: Vec2) -> Vec<f32> {
        (0..n * n)
            .map(|i| {
                let p = (Vec2::new((i % n) as f32, (i / n) as f32) + 0.5) / n as f32;
                f32::from(u8::from(p.cmpge(min).all() && p.cmplt(max).all()))
            })
            .collect()
    }

    fn mask_of(shapes: &[(MaskMode, Vec2, Vec2)], n: usize, inverted: bool) -> Vec<f32> {
        let shapes: Vec<MaskShape> = shapes
            .iter()
            .map(|(mode, min, max)| MaskShape {
                mode: *mode,
                ..MaskShape::keep(MaskGeometry::Rect {
                    min: *min,
                    max: *max,
                    radius: 0.0,
                })
            })
            .collect();
        let mut acc = vec![coverage_start(&shapes); n * n];
        for s in &shapes {
            let MaskGeometry::Rect { min, max, .. } = s.geometry else {
                unreachable!()
            };
            combine(&mut acc, &grid(n, min, max), s.mode);
        }
        if inverted {
            invert(&mut acc);
        }
        acc
    }

    fn rows(acc: &[f32], n: usize) -> Vec<String> {
        acc.chunks(n)
            .map(|r| r.iter().map(|v| if *v > 0.5 { '#' } else { '.' }).collect())
            .collect()
    }

    #[test]
    fn shapes_keep_and_cut_in_order() {
        let (k, c) = (MaskMode::Keep, MaskMode::Cut);
        let v = Vec2::new;
        // Keep the middle, then cut its center.
        let acc = mask_of(
            &[
                (k, v(0.25, 0.25), v(0.75, 0.75)),
                (c, v(0.5, 0.5), v(0.75, 0.75)),
            ],
            4,
            false,
        );
        assert_eq!(rows(&acc, 4), ["....", ".##.", ".#..", "...."]);
        // Order matters: keeping after the cut brings it back.
        let acc = mask_of(
            &[
                (k, v(0.25, 0.25), v(0.75, 0.75)),
                (c, v(0.5, 0.5), v(0.75, 0.75)),
                (k, v(0.5, 0.5), v(1.0, 1.0)),
            ],
            4,
            false,
        );
        assert_eq!(rows(&acc, 4), ["....", ".##.", ".###", "..##"]);
        // A first cut starts from the whole item.
        let acc = mask_of(&[(c, v(0.75, 0.0), v(1.0, 0.25))], 4, false);
        assert_eq!(rows(&acc, 4), ["###.", "####", "####", "####"]);
        // Invert flips the result.
        let acc = mask_of(&[(k, v(0.0, 0.0), v(0.5, 0.5))], 4, true);
        assert_eq!(rows(&acc, 4), ["..##", "..##", "####", "####"]);
    }

    #[test]
    fn soft_coverage_blends() {
        let mut acc = vec![0.0, 0.5, 1.0];
        combine(&mut acc, &[0.25, 0.25, 0.25], MaskMode::Keep);
        assert_eq!(acc, [0.25, 0.5, 1.0]);
        combine(&mut acc, &[0.5, 0.5, 0.5], MaskMode::Cut);
        assert_eq!(acc, [0.125, 0.25, 0.5]);
    }

    #[test]
    fn masks_round_trip_and_older_files_still_load() {
        let mut mask = ItemMask::rounded(40.0);
        mask.shapes.push(MaskShape {
            feather: 12.0,
            ..MaskShape::cut(MaskGeometry::Ellipse {
                center: Vec2::new(0.8, 0.2),
                radii: Vec2::splat(0.1),
            })
        });
        mask.shapes.push(MaskShape::keep(MaskGeometry::Polygon {
            points: vec![Vec2::ZERO, Vec2::X, Vec2::Y],
        }));
        mask.effects = MaskEffects::neon_glow();
        mask.effects.shadow = MaskEffects::soft_shadow().shadow;
        let json = serde_json::to_value(&mask).unwrap();
        assert_eq!(json["shapes"][0]["shape"], "rect");
        assert_eq!(json["shapes"][1]["mode"], "cut");
        assert!(json.get("invert").is_none(), "defaults aren't written");
        assert_eq!(serde_json::from_value::<ItemMask>(json).unwrap(), mask);

        let bare: ItemMask = serde_json::from_str("{}").unwrap();
        assert_eq!(bare, ItemMask::default());
        assert!(bare.enabled && !bare.draws());
    }

    #[test]
    fn effects_reach_and_layers() {
        let mut e = MaskEffects::thin_border();
        assert_eq!(e.extent(), 3.0);
        assert!(e.has_over() && !e.has_under());
        e.shadow = MaskEffects::soft_shadow().shadow;
        assert_eq!(e.extent(), 32.0, "offset plus softness");
        assert!(e.has_under());
        let inner = MaskEffects {
            glow: Some(Glow {
                placement: GlowPlacement::Inside,
                ..MaskEffects::neon_glow().glow.unwrap()
            }),
            ..MaskEffects::default()
        };
        assert_eq!(inner.extent(), 0.0);
        assert!(inner.has_over() && !inner.has_under());
    }

    #[test]
    fn presets_and_validation() {
        let MaskGeometry::Ellipse { radii, .. } = ItemMask::circle(16.0 / 9.0).shapes[0].geometry
        else {
            panic!("circle is an ellipse")
        };
        assert!(
            (radii.x * 16.0 - radii.y * 9.0).abs() < 1e-5,
            "round on screen"
        );
        for preset in [
            ItemMask::circle(0.5),
            ItemMask::rounded(40.0),
            ItemMask::cut_corner(),
            ItemMask::diagonal_split(),
        ] {
            assert!(preset.cuts());
            preset.validate().unwrap();
        }
        let mut bad = ItemMask::rounded(f32::NAN);
        bad.shapes[0].geometry = MaskGeometry::Rect {
            min: Vec2::ZERO,
            max: Vec2::ONE,
            radius: f32::NAN,
        };
        assert!(bad.validate().is_err());
        let two = ItemMask::with_shapes(vec![MaskShape::keep(MaskGeometry::Polygon {
            points: vec![Vec2::ZERO, Vec2::ONE],
        })]);
        assert!(two.validate().is_err());
        let mut glow = ItemMask::default();
        glow.effects.glow = Some(Glow {
            strength: 3.0,
            ..MaskEffects::neon_glow().glow.unwrap()
        });
        assert!(glow.validate().is_err());
    }
}
