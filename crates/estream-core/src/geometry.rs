//! Scene item geometry: transforms, crop, bounds, and the layout math that
//! maps a source onto the canvas.

use glam::{Affine2, Vec2};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Size {
    pub width: f32,
    pub height: f32,
}

impl Size {
    pub const fn new(width: f32, height: f32) -> Self {
        Self { width, height }
    }

    pub fn to_vec2(self) -> Vec2 {
        Vec2::new(self.width, self.height)
    }
}

/// Pixels removed from each edge of the source before any other transform.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Crop {
    pub left: u32,
    pub top: u32,
    pub right: u32,
    pub bottom: u32,
}

impl Crop {
    pub const NONE: Self = Self {
        left: 0,
        top: 0,
        right: 0,
        bottom: 0,
    };
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BoundsMode {
    /// No bounding box; size is the cropped source size times `scale`.
    #[default]
    None,
    /// Stretch to exactly fill the bounding box.
    Stretch,
    /// Scale uniformly to fit inside the bounding box.
    Fit,
    /// Scale uniformly to cover the bounding box.
    Fill,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Alignment {
    #[default]
    TopLeft,
    Top,
    TopRight,
    Left,
    Center,
    Right,
    BottomLeft,
    Bottom,
    BottomRight,
}

impl Alignment {
    /// Fractional anchor (0, 0.5 or 1 per axis).
    pub fn factors(self) -> Vec2 {
        use Alignment::*;
        let x = match self {
            TopLeft | Left | BottomLeft => 0.0,
            Top | Center | Bottom => 0.5,
            TopRight | Right | BottomRight => 1.0,
        };
        let y = match self {
            TopLeft | Top | TopRight => 0.0,
            Left | Center | Right => 0.5,
            BottomLeft | Bottom | BottomRight => 1.0,
        };
        Vec2::new(x, y)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BlendMode {
    #[default]
    Normal,
    Additive,
    Multiply,
    Screen,
}

impl BlendMode {
    pub const ALL: [BlendMode; 4] = [
        BlendMode::Normal,
        BlendMode::Additive,
        BlendMode::Multiply,
        BlendMode::Screen,
    ];
}

/// Placement of a scene item relative to its parent (canvas or group).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ItemTransform {
    /// Top-left corner of the item's box, in parent pixels.
    pub position: Vec2,
    /// Clockwise rotation in degrees around `position`.
    pub rotation: f32,
    /// Scale factor; ignored when a bounds mode is set.
    pub scale: Vec2,
    pub bounds_mode: BoundsMode,
    /// Bounding box size in parent pixels (used when `bounds_mode != None`).
    pub bounds: Vec2,
    /// Alignment of the scaled source inside the bounding box.
    pub bounds_alignment: Alignment,
}

impl Default for ItemTransform {
    fn default() -> Self {
        Self {
            position: Vec2::ZERO,
            rotation: 0.0,
            scale: Vec2::ONE,
            bounds_mode: BoundsMode::None,
            bounds: Vec2::ZERO,
            bounds_alignment: Alignment::Center,
        }
    }
}

/// Result of laying out a source within its item transform.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ItemLayout {
    /// Maps the unit quad (0..1, 0..1) to parent space.
    pub matrix: Affine2,
    /// Rendered size in parent pixels (before rotation).
    pub size: Vec2,
    /// Texture coordinates of the cropped region.
    pub uv_min: Vec2,
    pub uv_max: Vec2,
}

impl ItemTransform {
    /// Rotation + translation part of the transform (no scale), used for groups and anchors.
    pub fn placement(&self) -> Affine2 {
        Affine2::from_translation(self.position) * Affine2::from_angle(self.rotation.to_radians())
    }

    /// Matrix applied to the children of a group item.
    pub fn group_matrix(&self) -> Affine2 {
        self.placement() * Affine2::from_scale(self.scale)
    }

    /// Maps the unit square onto the bounding box (for fills behind a fitted source).
    pub fn bounds_matrix(&self) -> Affine2 {
        self.placement() * Affine2::from_scale(self.bounds.max(Vec2::ZERO))
    }

    /// The part of the cropped source that *covers* the bounding box (the middle of it, at the
    /// box's aspect), as texture coordinates. Zero when there is nothing to show.
    pub fn cover_uv(&self, crop: Crop, source_size: Size) -> (Vec2, Vec2) {
        let src = source_size.to_vec2().max(Vec2::ZERO);
        let crop_min = Vec2::new(crop.left as f32, crop.top as f32);
        let crop_max = Vec2::new(crop.right as f32, crop.bottom as f32);
        let cropped = (src - crop_min - crop_max).max(Vec2::ZERO);
        let bounds = self.bounds.max(Vec2::ZERO);
        if cropped.min_element() <= 0.0 || bounds.min_element() <= 0.0 {
            return (Vec2::ZERO, Vec2::ZERO);
        }
        let visible = bounds / (bounds / cropped).max_element();
        let min = crop_min + (cropped - visible) * 0.5;
        (min / src, (min + visible) / src)
    }

    /// Lays out a source of `source_size` pixels with `crop` applied first.
    pub fn layout(&self, crop: Crop, source_size: Size) -> ItemLayout {
        let src = source_size.to_vec2().max(Vec2::ZERO);
        let crop_min = Vec2::new(crop.left as f32, crop.top as f32);
        let crop_max = Vec2::new(crop.right as f32, crop.bottom as f32);
        let cropped = (src - crop_min - crop_max).max(Vec2::ZERO);

        let (uv_min, uv_max) = if src.x > 0.0 && src.y > 0.0 && cropped.x > 0.0 && cropped.y > 0.0 {
            (crop_min / src, (crop_min + cropped) / src)
        } else {
            (Vec2::ZERO, Vec2::ZERO)
        };

        let (size, offset) = match self.bounds_mode {
            BoundsMode::None => (cropped * self.scale, Vec2::ZERO),
            mode => {
                let bounds = self.bounds.max(Vec2::ZERO);
                let size = if cropped.x <= 0.0 || cropped.y <= 0.0 {
                    Vec2::ZERO
                } else {
                    match mode {
                        BoundsMode::Stretch => bounds,
                        BoundsMode::Fit => cropped * (bounds / cropped).min_element(),
                        BoundsMode::Fill => cropped * (bounds / cropped).max_element(),
                        BoundsMode::None => unreachable!(),
                    }
                };
                (size, (bounds - size) * self.bounds_alignment.factors())
            }
        };

        let matrix =
            self.placement() * Affine2::from_translation(offset) * Affine2::from_scale(size);
        ItemLayout {
            matrix,
            size,
            uv_min,
            uv_max,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approx(a: Vec2, b: Vec2) {
        assert!((a - b).abs().max_element() < 1e-3, "{a:?} != {b:?}");
    }

    #[test]
    fn cover_uv_takes_the_middle_at_the_box_aspect() {
        let t = ItemTransform {
            bounds_mode: BoundsMode::Fit,
            bounds: Vec2::new(1080.0, 1920.0),
            ..Default::default()
        };
        // A 16:9 game behind a 9:16 box: the full height, the middle 9/16 of 9/16 of the width.
        let (min, max) = t.cover_uv(Crop::NONE, Size::new(1920.0, 1080.0));
        let w = 1080.0 * 1080.0 / 1920.0 / 1920.0;
        approx(min, Vec2::new((1.0 - w) / 2.0, 0.0));
        approx(max, Vec2::new((1.0 + w) / 2.0, 1.0));
        // Crop is respected.
        let crop = Crop {
            left: 100,
            top: 0,
            right: 0,
            bottom: 80,
        };
        let (min, max) = t.cover_uv(crop, Size::new(1920.0, 1080.0));
        assert!(min.x > 100.0 / 1920.0 && max.y <= 1000.0 / 1080.0 + 1e-6);
        // Nothing to cover.
        let none = ItemTransform::default().cover_uv(Crop::NONE, Size::new(10.0, 10.0));
        assert_eq!(none, (Vec2::ZERO, Vec2::ZERO));
    }

    #[test]
    fn bounds_matrix_maps_the_unit_square_to_the_box() {
        let t = ItemTransform {
            position: Vec2::new(10.0, 20.0),
            bounds: Vec2::new(100.0, 50.0),
            bounds_mode: BoundsMode::Fit,
            ..Default::default()
        };
        approx(
            t.bounds_matrix().transform_point2(Vec2::ONE),
            Vec2::new(110.0, 70.0),
        );
    }

    #[test]
    fn fit_bounds_preserve_aspect_and_center() {
        let t = ItemTransform {
            bounds_mode: BoundsMode::Fit,
            bounds: Vec2::new(1000.0, 1000.0),
            bounds_alignment: Alignment::Center,
            ..Default::default()
        };
        let l = t.layout(Crop::NONE, Size::new(1920.0, 1080.0));
        approx(l.size, Vec2::new(1000.0, 562.5));
        approx(
            l.matrix.transform_point2(Vec2::ZERO),
            Vec2::new(0.0, 218.75),
        );
        approx(
            l.matrix.transform_point2(Vec2::ONE),
            Vec2::new(1000.0, 781.25),
        );
    }

    #[test]
    fn fill_covers_bounds() {
        let t = ItemTransform {
            bounds_mode: BoundsMode::Fill,
            bounds: Vec2::new(1000.0, 1000.0),
            ..Default::default()
        };
        let l = t.layout(Crop::NONE, Size::new(1920.0, 1080.0));
        approx(l.size, Vec2::new(1_777.777_8, 1000.0));
    }

    #[test]
    fn crop_applies_before_scale() {
        let t = ItemTransform {
            scale: Vec2::splat(2.0),
            ..Default::default()
        };
        let crop = Crop {
            left: 100,
            ..Crop::NONE
        };
        let l = t.layout(crop, Size::new(400.0, 200.0));
        approx(l.size, Vec2::new(600.0, 400.0));
        approx(l.uv_min, Vec2::new(0.25, 0.0));
        approx(l.uv_max, Vec2::new(1.0, 1.0));
    }

    #[test]
    fn rotation_is_around_position() {
        let t = ItemTransform {
            position: Vec2::new(100.0, 100.0),
            rotation: 90.0,
            ..Default::default()
        };
        let l = t.layout(Crop::NONE, Size::new(10.0, 20.0));
        approx(
            l.matrix.transform_point2(Vec2::ZERO),
            Vec2::new(100.0, 100.0),
        );
        // The item's right edge (x = 10) rotates to point down (+y) in screen space.
        approx(
            l.matrix.transform_point2(Vec2::new(1.0, 0.0)),
            Vec2::new(100.0, 110.0),
        );
    }

    #[test]
    fn over_crop_collapses_to_nothing() {
        let crop = Crop {
            left: 300,
            right: 300,
            ..Crop::NONE
        };
        let l = ItemTransform::default().layout(crop, Size::new(400.0, 200.0));
        assert_eq!(l.size.x, 0.0);
    }
}
