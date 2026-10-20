//! On-canvas editing: item geometry, hit testing, handles, and the drag math for
//! move / resize / crop / rotate with snapping. Pure functions, no UI types.

use estream_core::{BoundsMode, Crop, ItemId, ItemTransform, Size, SourceId};
use estream_project::{ItemContent, Scene, SceneItem};
use glam::{Affine2, Vec2};

/// Axis-aligned rectangle in canvas pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub min: Vec2,
    pub max: Vec2,
}

impl Rect {
    pub fn from_points(points: impl IntoIterator<Item = Vec2>) -> Option<Self> {
        let mut it = points.into_iter();
        let first = it.next()?;
        let (min, max) = it.fold((first, first), |(lo, hi), p| (lo.min(p), hi.max(p)));
        Some(Self { min, max })
    }

    pub fn union(self, other: Self) -> Self {
        Self {
            min: self.min.min(other.min),
            max: self.max.max(other.max),
        }
    }

    pub fn center(self) -> Vec2 {
        (self.min + self.max) * 0.5
    }

    pub fn size(self) -> Vec2 {
        self.max - self.min
    }

    pub fn offset(self, d: Vec2) -> Self {
        Self {
            min: self.min + d,
            max: self.max + d,
        }
    }
}

/// Geometry of one scene item on the canvas.
#[derive(Clone, Debug)]
pub struct ItemGeom {
    pub id: ItemId,
    /// Top-level ancestor (the item itself when at the root).
    pub top: ItemId,
    /// Chain from the top-level ancestor down to this item.
    pub path: Vec<ItemId>,
    pub is_group: bool,
    pub locked: bool,
    pub visible: bool,
    /// Accumulated transform of enclosing groups (child space → canvas).
    pub parent: Affine2,
    /// The item's editable box (bounds box, or scaled cropped content) mapped
    /// from the unit square to canvas space. For groups: the children's union.
    pub world: Affine2,
    /// Box size in the item's own (unrotated) frame.
    pub box_size: Vec2,
    /// Cropped source size in source pixels (zero for groups or sourceless items).
    pub content: Vec2,
    pub source_size: Vec2,
    pub aabb: Rect,
}

impl ItemGeom {
    /// True if `p` (canvas) lies inside the item's (possibly rotated) box.
    pub fn contains(&self, p: Vec2) -> bool {
        if self.world.matrix2.determinant().abs() < 1e-9 {
            return false;
        }
        let u = self.world.inverse().transform_point2(p);
        (0.0..=1.0).contains(&u.x) && (0.0..=1.0).contains(&u.y)
    }

    /// World-space rotation in degrees, size and center for drawing the selection box.
    pub fn display_box(&self) -> (Vec2, Vec2, f32) {
        let x_axis = self.world.matrix2.x_axis;
        let y_axis = self.world.matrix2.y_axis;
        let center = self.world.transform_point2(Vec2::splat(0.5));
        let rotation = x_axis.y.atan2(x_axis.x).to_degrees();
        (
            center,
            Vec2::new(x_axis.length(), y_axis.length()),
            rotation,
        )
    }

    /// Whether resize/crop/rotate handles apply (source items; groups only move).
    pub fn editable_box(&self) -> bool {
        !self.is_group && self.content.x > 0.0 && self.content.y > 0.0
    }
}

fn source_box(transform: &ItemTransform, crop: Crop, size: Size) -> (Vec2, Vec2) {
    let src = size.to_vec2().max(Vec2::ZERO);
    let content = (src
        - Vec2::new(crop.left as f32, crop.top as f32)
        - Vec2::new(crop.right as f32, crop.bottom as f32))
    .max(Vec2::ZERO);
    let box_size = match transform.bounds_mode {
        BoundsMode::None => content * transform.scale,
        _ => transform.bounds.max(Vec2::ZERO),
    };
    (box_size, content)
}

/// Computes geometry for every visible item of `scene`, bottom-most first.
pub fn scene_geometry(scene: &Scene, size_of: &dyn Fn(SourceId) -> Option<Size>) -> Vec<ItemGeom> {
    let mut out = Vec::new();
    walk(&scene.items, Affine2::IDENTITY, &[], size_of, &mut out);
    out
}

fn walk(
    items: &[SceneItem],
    parent: Affine2,
    path: &[ItemId],
    size_of: &dyn Fn(SourceId) -> Option<Size>,
    out: &mut Vec<ItemGeom>,
) -> Option<Rect> {
    let mut union: Option<Rect> = None;
    for item in items {
        let mut item_path = path.to_vec();
        item_path.push(item.id);
        let top = item_path[0];
        let geom = match &item.content {
            ItemContent::Source { source } => {
                let source_size = size_of(*source).unwrap_or_default();
                let (box_size, content) = source_box(&item.transform, item.crop, source_size);
                let world = parent * item.transform.placement() * Affine2::from_scale(box_size);
                let aabb = Rect::from_points(
                    [Vec2::ZERO, Vec2::X, Vec2::ONE, Vec2::Y].map(|p| world.transform_point2(p)),
                )
                .unwrap_or(Rect {
                    min: Vec2::ZERO,
                    max: Vec2::ZERO,
                });
                ItemGeom {
                    id: item.id,
                    top,
                    path: item_path,
                    is_group: false,
                    locked: item.locked,
                    visible: item.visible,
                    parent,
                    world,
                    box_size,
                    content,
                    source_size: source_size.to_vec2(),
                    aabb,
                }
            }
            ItemContent::Group { items: children } => {
                let child_parent = parent * item.transform.group_matrix();
                let index = out.len();
                // Reserve the group's slot so it draws/hits below its children.
                out.push(ItemGeom {
                    id: item.id,
                    top,
                    path: item_path.clone(),
                    is_group: true,
                    locked: item.locked,
                    visible: item.visible,
                    parent,
                    world: Affine2::ZERO,
                    box_size: Vec2::ZERO,
                    content: Vec2::ZERO,
                    source_size: Vec2::ZERO,
                    aabb: Rect {
                        min: Vec2::ZERO,
                        max: Vec2::ZERO,
                    },
                });
                let aabb = walk(children, child_parent, &item_path, size_of, out).unwrap_or(Rect {
                    min: Vec2::ZERO,
                    max: Vec2::ZERO,
                });
                let g = &mut out[index];
                g.aabb = aabb;
                g.box_size = aabb.size();
                g.world = Affine2::from_translation(aabb.min) * Affine2::from_scale(aabb.size());
                if item.visible {
                    union = Some(union.map_or(aabb, |u| u.union(aabb)));
                }
                continue;
            }
        };
        if item.visible {
            union = Some(union.map_or(geom.aabb, |u| u.union(geom.aabb)));
        }
        out.push(geom);
    }
    union
}

/// Finds the item to select at `p`. Prefers keeping `current` selected when it is
/// under the pointer (so a child picked in the layers list can be dragged);
/// otherwise selects the top-level item.
pub fn hit_test(geoms: &[ItemGeom], p: Vec2, current: Option<ItemId>) -> Option<ItemId> {
    let hit = geoms
        .iter()
        .rev()
        .filter(|g| g.visible && !g.is_group && hidden_ancestors_ok(geoms, g))
        .find(|g| g.contains(p))?;
    if let Some(cur) = current
        && hit.path.contains(&cur)
    {
        return Some(cur);
    }
    Some(hit.top)
}

fn hidden_ancestors_ok(geoms: &[ItemGeom], g: &ItemGeom) -> bool {
    g.path[..g.path.len() - 1]
        .iter()
        .all(|id| geoms.iter().find(|x| x.id == *id).is_none_or(|x| x.visible))
}

/// The eight resize handles plus rotation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Handle {
    Resize { fx: u8, fy: u8 },
    Rotate,
}

impl Handle {
    pub const RESIZE: [Handle; 8] = [
        Handle::Resize { fx: 0, fy: 0 },
        Handle::Resize { fx: 1, fy: 0 },
        Handle::Resize { fx: 2, fy: 0 },
        Handle::Resize { fx: 2, fy: 1 },
        Handle::Resize { fx: 2, fy: 2 },
        Handle::Resize { fx: 1, fy: 2 },
        Handle::Resize { fx: 0, fy: 2 },
        Handle::Resize { fx: 0, fy: 1 },
    ];

    /// Unit-square position of a resize handle.
    pub fn unit(self) -> Vec2 {
        match self {
            Handle::Resize { fx, fy } => Vec2::new(fx as f32 * 0.5, fy as f32 * 0.5),
            Handle::Rotate => Vec2::new(0.5, 0.0),
        }
    }
}

/// Distance (screen px) of the rotation handle above the box's top edge.
pub const ROTATE_OFFSET_PX: f32 = 28.0;

/// Finds the handle of `g` under `p`; `px` is canvas pixels per screen pixel.
pub fn handle_at(g: &ItemGeom, p: Vec2, px: f32) -> Option<Handle> {
    if !g.editable_box() || g.locked {
        return None;
    }
    let radius = 7.0 * px;
    let rot = rotate_handle_pos(g, px);
    if rot.distance(p) <= radius {
        return Some(Handle::Rotate);
    }
    Handle::RESIZE
        .into_iter()
        .find(|h| g.world.transform_point2(h.unit()).distance(p) <= radius)
}

pub fn rotate_handle_pos(g: &ItemGeom, px: f32) -> Vec2 {
    let top = g.world.transform_point2(Vec2::new(0.5, 0.0));
    let center = g.world.transform_point2(Vec2::splat(0.5));
    let up = (top - center).normalize_or(Vec2::NEG_Y);
    top + up * ROTATE_OFFSET_PX * px
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Guide {
    pub vertical: bool,
    /// Canvas x (vertical guide) or y (horizontal guide).
    pub pos: f32,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Modifiers {
    pub shift: bool,
    pub alt: bool,
    pub ctrl: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum DragKind {
    Move,
    Resize { fx: u8, fy: u8 },
    Rotate,
}

/// An in-progress drag on one item. All maths is relative to the start state,
/// so applying `update` repeatedly never accumulates error.
#[derive(Clone, Debug)]
pub struct Drag {
    pub kind: DragKind,
    pub item: ItemId,
    start_pointer: Vec2,
    start: ItemTransform,
    start_crop: Crop,
    parent: Affine2,
    box_size: Vec2,
    content: Vec2,
    source_size: Vec2,
    start_aabb: Rect,
    /// Snap lines: (vertical lines, horizontal lines) in canvas space.
    snap_x: Vec<f32>,
    snap_y: Vec<f32>,
}

/// Result of a drag update.
#[derive(Clone, Debug, PartialEq)]
pub struct DragResult {
    pub transform: ItemTransform,
    pub crop: Crop,
    pub guides: Vec<Guide>,
}

impl Drag {
    pub fn new(
        kind: DragKind,
        geom: &ItemGeom,
        transform: ItemTransform,
        crop: Crop,
        pointer: Vec2,
        canvas: Vec2,
        others: impl Iterator<Item = Rect>,
    ) -> Self {
        let mut snap_x = vec![0.0, canvas.x * 0.5, canvas.x];
        let mut snap_y = vec![0.0, canvas.y * 0.5, canvas.y];
        for r in others {
            snap_x.extend([r.min.x, r.center().x, r.max.x]);
            snap_y.extend([r.min.y, r.center().y, r.max.y]);
        }
        Self {
            kind,
            item: geom.id,
            start_pointer: pointer,
            start: transform,
            start_crop: crop,
            parent: geom.parent,
            box_size: geom.box_size,
            content: geom.content,
            source_size: geom.source_size,
            start_aabb: geom.aabb,
            snap_x,
            snap_y,
        }
    }

    /// `px` = canvas pixels per screen pixel (for screen-constant snap distance).
    pub fn update(&self, pointer: Vec2, mods: Modifiers, px: f32, snapping: bool) -> DragResult {
        match self.kind {
            DragKind::Move => self.update_move(pointer, px, snapping && !mods.ctrl),
            DragKind::Resize { fx, fy } => self.update_resize(fx, fy, pointer, mods),
            DragKind::Rotate => self.update_rotate(pointer, mods),
        }
    }

    fn update_move(&self, pointer: Vec2, px: f32, snapping: bool) -> DragResult {
        let mut delta = pointer - self.start_pointer;
        let mut guides = Vec::new();
        if snapping {
            let threshold = 8.0 * px;
            let moved = self.start_aabb.offset(delta);
            if let Some((adjust, line)) = best_snap(
                [moved.min.x, moved.center().x, moved.max.x],
                &self.snap_x,
                threshold,
            ) {
                delta.x += adjust;
                guides.push(Guide {
                    vertical: true,
                    pos: line,
                });
            }
            if let Some((adjust, line)) = best_snap(
                [moved.min.y, moved.center().y, moved.max.y],
                &self.snap_y,
                threshold,
            ) {
                delta.y += adjust;
                guides.push(Guide {
                    vertical: false,
                    pos: line,
                });
            }
        }
        let local = invert_linear(self.parent).mul_vec2(delta);
        DragResult {
            transform: ItemTransform {
                position: self.start.position + local,
                ..self.start
            },
            crop: self.start_crop,
            guides,
        }
    }

    /// Item frame: parent × placement (translation + rotation), unscaled.
    fn frame(&self) -> Affine2 {
        self.parent * self.start.placement()
    }

    fn update_resize(&self, fx: u8, fy: u8, pointer: Vec2, mods: Modifiers) -> DragResult {
        let frame = self.frame();
        let local = frame.inverse().transform_point2(pointer);
        let w = self.box_size;
        let (mut min, mut max) = (Vec2::ZERO, w);
        match fx {
            0 => min.x = local.x,
            2 => max.x = local.x,
            _ => {}
        }
        match fy {
            0 => min.y = local.y,
            2 => max.y = local.y,
            _ => {}
        }

        if mods.alt && self.start.bounds_mode == BoundsMode::None {
            return self.update_crop(fx, fy, min, max);
        }

        let mut size = (max - min).max(Vec2::ONE);
        let corner = fx != 1 && fy != 1;
        if mods.shift && corner && w.x > 0.0 && w.y > 0.0 {
            let s = (size.x / w.x).max(size.y / w.y);
            size = w * s;
        }
        // Keep the edge(s) opposite the dragged handle fixed.
        let new_min = Vec2::new(
            if fx == 0 { w.x - size.x } else { 0.0 },
            if fy == 0 { w.y - size.y } else { 0.0 },
        );
        let mut t = self.start;
        t.position = self
            .parent
            .inverse()
            .transform_point2(frame.transform_point2(new_min));
        match t.bounds_mode {
            BoundsMode::None => {
                if self.content.x > 0.0 && self.content.y > 0.0 {
                    t.scale = size / self.content;
                }
            }
            _ => t.bounds = size,
        }
        DragResult {
            transform: t,
            crop: self.start_crop,
            guides: Vec::new(),
        }
    }

    fn update_crop(&self, fx: u8, fy: u8, min: Vec2, max: Vec2) -> DragResult {
        let scale = self.start.scale;
        let mut crop = self.start_crop;
        let src = self.source_size;
        let to_src = |local: f32, s: f32| if s.abs() > 1e-6 { local / s } else { 0.0 };
        let mut shift = Vec2::ZERO;
        if fx == 0 {
            let max_left = src.x - crop.right as f32 - 1.0;
            let left = (self.start_crop.left as f32 + to_src(min.x, scale.x))
                .clamp(0.0, max_left.max(0.0));
            shift.x = (left - self.start_crop.left as f32) * scale.x;
            crop.left = left.round() as u32;
        } else if fx == 2 {
            let max_right = src.x - crop.left as f32 - 1.0;
            let right = (self.start_crop.right as f32 + to_src(self.box_size.x - max.x, scale.x))
                .clamp(0.0, max_right.max(0.0));
            crop.right = right.round() as u32;
        }
        if fy == 0 {
            let max_top = src.y - crop.bottom as f32 - 1.0;
            let top =
                (self.start_crop.top as f32 + to_src(min.y, scale.y)).clamp(0.0, max_top.max(0.0));
            shift.y = (top - self.start_crop.top as f32) * scale.y;
            crop.top = top.round() as u32;
        } else if fy == 2 {
            let max_bottom = src.y - crop.top as f32 - 1.0;
            let bottom = (self.start_crop.bottom as f32 + to_src(self.box_size.y - max.y, scale.y))
                .clamp(0.0, max_bottom.max(0.0));
            crop.bottom = bottom.round() as u32;
        }
        let frame = self.frame();
        let mut t = self.start;
        t.position = self
            .parent
            .inverse()
            .transform_point2(frame.transform_point2(shift));
        DragResult {
            transform: t,
            crop,
            guides: Vec::new(),
        }
    }

    fn update_rotate(&self, pointer: Vec2, mods: Modifiers) -> DragResult {
        // Rotate around the box center, measured in parent space.
        let parent_inv = self.parent.inverse();
        let half = self.box_size * 0.5;
        let center = self.start.placement().transform_point2(half);
        let p0 = parent_inv.transform_point2(self.start_pointer) - center;
        let p1 = parent_inv.transform_point2(pointer) - center;
        let delta = (p1.y.atan2(p1.x) - p0.y.atan2(p0.x)).to_degrees();
        let mut rotation = self.start.rotation + delta;
        if mods.shift {
            rotation = (rotation / 15.0).round() * 15.0;
        }
        rotation = rotation.rem_euclid(360.0);
        let mut t = self.start;
        t.rotation = rotation;
        let r = glam::Mat2::from_angle(rotation.to_radians());
        t.position = center - r * half;
        DragResult {
            transform: t,
            crop: self.start_crop,
            guides: Vec::new(),
        }
    }
}

fn invert_linear(m: Affine2) -> glam::Mat2 {
    if m.matrix2.determinant().abs() < 1e-9 {
        glam::Mat2::IDENTITY
    } else {
        m.matrix2.inverse()
    }
}

/// Finds the smallest adjustment that aligns any of `edges` with any `line`.
fn best_snap(edges: [f32; 3], lines: &[f32], threshold: f32) -> Option<(f32, f32)> {
    let mut best: Option<(f32, f32)> = None;
    for edge in edges {
        for &line in lines {
            let d = line - edge;
            if d.abs() <= threshold && best.is_none_or(|(b, _)| d.abs() < b.abs()) {
                best = Some((d, line));
            }
        }
    }
    best
}

/// A canvas corner, for the quick placements.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Corner {
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

/// Transform for "put it in a corner": fitted into a box 30% of the canvas, a small margin
/// from the edges (a webcam over gameplay).
pub fn in_corner(canvas: Vec2, corner: Corner) -> ItemTransform {
    let size = canvas * 0.3;
    let margin = canvas.x.min(canvas.y) * 0.03;
    let left = matches!(corner, Corner::TopLeft | Corner::BottomLeft);
    let top = matches!(corner, Corner::TopLeft | Corner::TopRight);
    let x = if left {
        margin
    } else {
        canvas.x - size.x - margin
    };
    let y = if top {
        margin
    } else {
        canvas.y - size.y - margin
    };
    ItemTransform {
        position: Vec2::new(x, y),
        bounds_mode: BoundsMode::Fit,
        bounds: size,
        bounds_alignment: match corner {
            Corner::TopLeft => estream_core::Alignment::TopLeft,
            Corner::TopRight => estream_core::Alignment::TopRight,
            Corner::BottomLeft => estream_core::Alignment::BottomLeft,
            Corner::BottomRight => estream_core::Alignment::BottomRight,
        },
        ..Default::default()
    }
}

/// Transform for "fit to canvas" (keeps aspect, centered).
pub fn fit_to_canvas(canvas: Vec2) -> ItemTransform {
    ItemTransform {
        bounds_mode: BoundsMode::Fit,
        bounds: canvas,
        bounds_alignment: estream_core::Alignment::Center,
        ..Default::default()
    }
}

/// Transform for "stretch to canvas".
pub fn stretch_to_canvas(canvas: Vec2) -> ItemTransform {
    ItemTransform {
        bounds_mode: BoundsMode::Stretch,
        bounds: canvas,
        ..Default::default()
    }
}

/// Moves an item so its AABB is centered on the canvas.
pub fn centered(geom: &ItemGeom, transform: ItemTransform, canvas: Vec2) -> ItemTransform {
    let delta = canvas * 0.5 - geom.aabb.center();
    let local = invert_linear(geom.parent).mul_vec2(delta);
    ItemTransform {
        position: transform.position + local,
        ..transform
    }
}

/// Where canvas point `p` lands on an item's source, in source pixels, through the item's
/// transform (with bounds and alignment) and crop. The flag says whether it's on the visible
/// content; outside, the point keeps going (so a drag that leaves the page still tracks).
pub fn source_point(
    parent: Affine2,
    transform: &ItemTransform,
    crop: Crop,
    source_size: Size,
    p: Vec2,
) -> Option<(Vec2, bool)> {
    let layout = transform.layout(crop, source_size);
    let m = parent * layout.matrix;
    if m.matrix2.determinant().abs() < 1e-9 {
        return None;
    }
    let q = m.inverse().transform_point2(p);
    let inside = (0.0..=1.0).contains(&q.x) && (0.0..=1.0).contains(&q.y);
    let uv = layout.uv_min + q * (layout.uv_max - layout.uv_min);
    Some((uv * source_size.to_vec2(), inside))
}

#[cfg(test)]
mod tests {
    use super::*;
    use estream_core::{CanvasId, Rational};
    use estream_project::{Project, SourceDef};

    fn approx(a: Vec2, b: Vec2) {
        assert!((a - b).abs().max_element() < 1e-2, "{a:?} != {b:?}");
    }

    #[test]
    fn source_point_follows_scale_crop_and_rotation() {
        let page = Size::new(800.0, 600.0);
        // Half size, moved to (100, 50).
        let t = ItemTransform {
            position: Vec2::new(100.0, 50.0),
            scale: Vec2::splat(0.5),
            ..Default::default()
        };
        let (pt, inside) = source_point(
            Affine2::IDENTITY,
            &t,
            Crop::NONE,
            page,
            Vec2::new(300.0, 200.0),
        )
        .unwrap();
        approx(pt, Vec2::new(400.0, 300.0));
        assert!(inside);
        // Cropping 100 px off the left shifts what's under the pointer.
        let crop = Crop {
            left: 100,
            ..Crop::NONE
        };
        let (pt, _) =
            source_point(Affine2::IDENTITY, &t, crop, page, Vec2::new(100.0, 50.0)).unwrap();
        approx(pt, Vec2::new(100.0, 0.0));
        // Outside the item: still mapped, but flagged.
        let (_, inside) = source_point(
            Affine2::IDENTITY,
            &t,
            Crop::NONE,
            page,
            Vec2::new(10.0, 10.0),
        )
        .unwrap();
        assert!(!inside);
        // Rotated 90° clockwise around the position: canvas +y is page +x.
        let r = ItemTransform {
            position: Vec2::new(500.0, 0.0),
            rotation: 90.0,
            ..Default::default()
        };
        let (pt, inside) = source_point(
            Affine2::IDENTITY,
            &r,
            Crop::NONE,
            page,
            Vec2::new(490.0, 30.0),
        )
        .unwrap();
        approx(pt, Vec2::new(30.0, 10.0));
        assert!(inside);
    }

    #[test]
    fn source_point_inside_fit_bounds() {
        // A 800×600 page fitted into a 400×400 box, centred: 400×300 at y = 50.
        let t = ItemTransform {
            bounds_mode: BoundsMode::Fit,
            bounds: Vec2::splat(400.0),
            bounds_alignment: estream_core::Alignment::Center,
            ..Default::default()
        };
        let page = Size::new(800.0, 600.0);
        let (pt, inside) = source_point(
            Affine2::IDENTITY,
            &t,
            Crop::NONE,
            page,
            Vec2::new(200.0, 200.0),
        )
        .unwrap();
        approx(pt, Vec2::new(400.0, 300.0));
        assert!(inside);
        // The letterbox above the page isn't on it.
        let (_, inside) = source_point(
            Affine2::IDENTITY,
            &t,
            Crop::NONE,
            page,
            Vec2::new(200.0, 20.0),
        )
        .unwrap();
        assert!(!inside);
    }

    fn setup(items: Vec<SceneItem>) -> (Scene, impl Fn(SourceId) -> Option<Size>) {
        let mut scene = Scene::new("S", CanvasId::new());
        scene.items = items;
        (scene, |_| Some(Size::new(200.0, 100.0)))
    }

    fn item_at(pos: Vec2, src: SourceId) -> SceneItem {
        let mut item = SceneItem::source(src);
        item.transform.position = pos;
        item
    }

    #[test]
    fn corners_fit_a_third_of_the_canvas_inside_the_margin() {
        let canvas = Vec2::new(1920.0, 1080.0);
        let br = in_corner(canvas, Corner::BottomRight);
        approx(br.bounds, Vec2::new(576.0, 324.0));
        approx(
            br.position,
            Vec2::new(1920.0 - 576.0 - 32.4, 1080.0 - 324.0 - 32.4),
        );
        let tl = in_corner(canvas, Corner::TopLeft);
        approx(tl.position, Vec2::new(32.4, 32.4));
        assert_eq!(tl.bounds_mode, BoundsMode::Fit);
    }

    #[test]
    fn hit_test_prefers_topmost_and_selects_group_root() {
        let src = SourceId::new();
        let bottom = item_at(Vec2::ZERO, src);
        let child = item_at(Vec2::new(50.0, 0.0), src);
        let child_id = child.id;
        let group = SceneItem::group("G", vec![child]);
        let group_id = group.id;
        let bottom_id = bottom.id;
        let (scene, size) = setup(vec![bottom, group]);
        let geoms = scene_geometry(&scene, &size);

        assert_eq!(
            hit_test(&geoms, Vec2::new(10.0, 10.0), None),
            Some(bottom_id)
        );
        assert_eq!(
            hit_test(&geoms, Vec2::new(100.0, 10.0), None),
            Some(group_id)
        );
        // A child selected from the layers list stays selected when clicked.
        assert_eq!(
            hit_test(&geoms, Vec2::new(100.0, 10.0), Some(child_id)),
            Some(child_id)
        );
        assert_eq!(hit_test(&geoms, Vec2::new(500.0, 500.0), None), None);
        let g = geoms.iter().find(|g| g.id == group_id).unwrap();
        approx(g.aabb.min, Vec2::new(50.0, 0.0));
        approx(g.aabb.max, Vec2::new(250.0, 100.0));
    }

    #[test]
    fn rotated_hit_test_uses_true_shape() {
        let mut item = item_at(Vec2::new(100.0, 100.0), SourceId::new());
        item.transform.rotation = 45.0;
        let id = item.id;
        let (scene, size) = setup(vec![item]);
        let geoms = scene_geometry(&scene, &size);
        // Inside the AABB corner but outside the rotated box.
        assert_eq!(hit_test(&geoms, Vec2::new(40.0, 250.0), None), None);
        assert_eq!(hit_test(&geoms, Vec2::new(100.0, 140.0), None), Some(id));
    }

    fn drag_for(item: &SceneItem, kind: DragKind, start: Vec2) -> Drag {
        let (scene, size) = setup(vec![item.clone()]);
        let geoms = scene_geometry(&scene, &size);
        Drag::new(
            kind,
            &geoms[0],
            item.transform,
            item.crop,
            start,
            Vec2::new(1920.0, 1080.0),
            std::iter::empty(),
        )
    }

    #[test]
    fn move_snaps_to_canvas_center_with_guide() {
        let item = item_at(Vec2::new(100.0, 100.0), SourceId::new());
        let drag = drag_for(&item, DragKind::Move, Vec2::new(150.0, 150.0));
        // Center x would be 200 + 657 = 857 + 100 = 957 → snaps to 960.
        let r = drag.update(
            Vec2::new(150.0 + 757.0, 150.0),
            Modifiers::default(),
            1.0,
            true,
        );
        approx(r.transform.position, Vec2::new(860.0, 100.0));
        assert!(r.guides.contains(&Guide {
            vertical: true,
            pos: 960.0
        }));
        // Ctrl disables snapping.
        let r = drag.update(
            Vec2::new(150.0 + 757.0, 150.0),
            Modifiers {
                ctrl: true,
                ..Default::default()
            },
            1.0,
            true,
        );
        approx(r.transform.position, Vec2::new(857.0, 100.0));
    }

    #[test]
    fn corner_resize_keeps_opposite_corner() {
        let item = item_at(Vec2::new(100.0, 100.0), SourceId::new());
        // Drag top-left handle from (100,100) to (50,80): box grows to 250x120.
        let drag = drag_for(
            &item,
            DragKind::Resize { fx: 0, fy: 0 },
            Vec2::new(100.0, 100.0),
        );
        let r = drag.update(Vec2::new(50.0, 80.0), Modifiers::default(), 1.0, true);
        approx(r.transform.position, Vec2::new(50.0, 80.0));
        approx(r.transform.scale, Vec2::new(1.25, 1.2));
        // Shift keeps the aspect ratio.
        let r = drag.update(
            Vec2::new(50.0, 80.0),
            Modifiers {
                shift: true,
                ..Default::default()
            },
            1.0,
            true,
        );
        approx(r.transform.scale, Vec2::new(1.25, 1.25));
        approx(r.transform.position, Vec2::new(50.0, 75.0));
    }

    #[test]
    fn rotated_resize_works_in_item_frame() {
        let mut item = item_at(Vec2::new(100.0, 100.0), SourceId::new());
        item.transform.rotation = 90.0;
        // Right edge (local +x) points down in canvas space; drag it 50px further down.
        let start = item
            .transform
            .placement()
            .transform_point2(Vec2::new(200.0, 50.0));
        let drag = drag_for(&item, DragKind::Resize { fx: 2, fy: 1 }, start);
        let r = drag.update(
            start + Vec2::new(0.0, 50.0),
            Modifiers::default(),
            1.0,
            true,
        );
        approx(r.transform.scale, Vec2::new(1.25, 1.0));
        approx(r.transform.position, Vec2::new(100.0, 100.0));
    }

    #[test]
    fn alt_drag_crops_instead_of_scaling() {
        let mut item = item_at(Vec2::new(100.0, 100.0), SourceId::new());
        item.transform.scale = Vec2::splat(2.0);
        // Box is 400x200. Drag left edge right by 40 canvas px = 20 source px.
        let drag = drag_for(
            &item,
            DragKind::Resize { fx: 0, fy: 1 },
            Vec2::new(100.0, 200.0),
        );
        let r = drag.update(
            Vec2::new(140.0, 200.0),
            Modifiers {
                alt: true,
                ..Default::default()
            },
            1.0,
            true,
        );
        assert_eq!(r.crop.left, 20);
        approx(r.transform.scale, Vec2::splat(2.0));
        approx(r.transform.position, Vec2::new(140.0, 100.0));
    }

    #[test]
    fn rotate_around_center_and_snap_with_shift() {
        let item = item_at(Vec2::new(100.0, 100.0), SourceId::new());
        let center = Vec2::new(200.0, 150.0);
        let drag = drag_for(&item, DragKind::Rotate, center + Vec2::new(0.0, -100.0));
        // Pointer moves to the right of center: +90°.
        let r = drag.update(
            center + Vec2::new(100.0, 3.0),
            Modifiers::default(),
            1.0,
            true,
        );
        assert!(
            (r.transform.rotation - 91.7).abs() < 0.2,
            "{}",
            r.transform.rotation
        );
        let r = drag.update(
            center + Vec2::new(100.0, 3.0),
            Modifiers {
                shift: true,
                ..Default::default()
            },
            1.0,
            true,
        );
        assert_eq!(r.transform.rotation, 90.0);
        // Center stays put.
        let (scene, size) = {
            let mut i = item.clone();
            i.transform = r.transform;
            setup(vec![i])
        };
        let g = &scene_geometry(&scene, &size)[0];
        approx(g.world.transform_point2(Vec2::splat(0.5)), center);
    }

    #[test]
    fn moving_child_of_scaled_group_uses_parent_space() {
        let src = SourceId::new();
        let child = item_at(Vec2::ZERO, src);
        let mut group = SceneItem::group("G", vec![child.clone()]);
        group.transform.scale = Vec2::splat(0.5);
        let (scene, size) = setup(vec![group]);
        let geoms = scene_geometry(&scene, &size);
        let g = geoms.iter().find(|g| g.id == child.id).unwrap();
        let drag = Drag::new(
            DragKind::Move,
            g,
            child.transform,
            child.crop,
            Vec2::ZERO,
            Vec2::new(1920.0, 1080.0),
            std::iter::empty(),
        );
        let r = drag.update(Vec2::new(10.0, 10.0), Modifiers::default(), 1.0, false);
        approx(r.transform.position, Vec2::new(20.0, 20.0));
    }

    #[test]
    fn handles_found_in_screen_space() {
        let item = item_at(Vec2::new(100.0, 100.0), SourceId::new());
        let (scene, size) = setup(vec![item]);
        let g = &scene_geometry(&scene, &size)[0];
        assert_eq!(
            handle_at(g, Vec2::new(302.0, 201.0), 1.0),
            Some(Handle::Resize { fx: 2, fy: 2 })
        );
        assert_eq!(
            handle_at(g, Vec2::new(200.0, 100.0 - 28.0), 1.0),
            Some(Handle::Rotate)
        );
        // At 4x zoom-out (4 canvas px per screen px) the hit radius grows.
        assert_eq!(
            handle_at(g, Vec2::new(318.0, 218.0), 4.0),
            Some(Handle::Resize { fx: 2, fy: 2 })
        );
        assert_eq!(handle_at(g, Vec2::new(200.0, 150.0), 1.0), None);
    }

    #[test]
    fn center_action() {
        let mut p = Project::new_default(1920, 1080, Rational::fps(60));
        let src = SourceDef::new("S", "color", serde_json::Value::Null);
        let item = item_at(Vec2::new(10.0, 10.0), src.id);
        p.scenes[0].items.push(item.clone());
        let (scene, size) = setup(vec![item.clone()]);
        let g = &scene_geometry(&scene, &size)[0];
        let t = centered(g, item.transform, Vec2::new(1920.0, 1080.0));
        approx(t.position, Vec2::new(860.0, 490.0));
    }
}
