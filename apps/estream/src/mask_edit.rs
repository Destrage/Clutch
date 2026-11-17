//! Mask editing on the preview: drawing, selecting and reshaping an item's mask shapes. Pure
//! logic in the item's unit space (0..1 over its visible picture); the app turns the masks it
//! returns into item commands and draws the outlines and handles it describes.

use estream_core::ItemId;
use estream_project::mask::MAX_POINTS;
use estream_project::{ItemMask, MaskGeometry, MaskMode, MaskShape};
use glam::{Affine2, Vec2};

/// Handle reach in screen pixels.
const HANDLE_PX: f32 = 7.0;
/// Drags shorter than this (screen pixels) draw nothing.
const MIN_DRAW_PX: f32 = 3.0;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Tool {
    #[default]
    Select,
    Rect,
    Ellipse,
    Polygon,
    Freehand,
}

impl Tool {
    pub const ALL: [Tool; 5] = [
        Tool::Select,
        Tool::Rect,
        Tool::Ellipse,
        Tool::Polygon,
        Tool::Freehand,
    ];

    pub fn from_name(name: &str) -> Option<Self> {
        Some(match name {
            "select" => Tool::Select,
            "rect" => Tool::Rect,
            "ellipse" => Tool::Ellipse,
            "polygon" => Tool::Polygon,
            "freehand" => Tool::Freehand,
            _ => return None,
        })
    }

    /// The tool a key picks (V, R, E, P, F).
    pub fn from_key(key: &str) -> Option<Self> {
        Some(match key.to_ascii_lowercase().as_str() {
            "v" => Tool::Select,
            "r" => Tool::Rect,
            "e" => Tool::Ellipse,
            "p" => Tool::Polygon,
            "f" => Tool::Freehand,
            _ => return None,
        })
    }

    pub fn index(self) -> i32 {
        Self::ALL.iter().position(|t| *t == self).unwrap_or(0) as i32
    }
}

/// Where the item's visible picture is on the canvas: the unit square mapped to canvas pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Frame {
    pub world: Affine2,
}

impl Frame {
    pub fn to_unit(self, p: Vec2) -> Option<Vec2> {
        (self.world.matrix2.determinant().abs() > 1e-9)
            .then(|| self.world.inverse().transform_point2(p))
    }

    pub fn to_canvas(self, u: Vec2) -> Vec2 {
        self.world.transform_point2(u)
    }

    /// The picture's size on the canvas.
    pub fn size(self) -> Vec2 {
        Vec2::new(
            self.world.matrix2.x_axis.length(),
            self.world.matrix2.y_axis.length(),
        )
        .max(Vec2::splat(1e-3))
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Mods {
    pub shift: bool,
    pub alt: bool,
}

#[derive(Clone, Debug)]
enum Gesture {
    Draw { start: Vec2 },
    Move { start: Vec2, orig: MaskShape },
    Resize { handle: usize, orig: MaskShape },
    Point { index: usize },
    Stroke,
}

/// The eight box handles: corners and edge middles, clockwise from the top left.
const BOX_HANDLES: [Vec2; 8] = [
    Vec2::new(0.0, 0.0),
    Vec2::new(0.5, 0.0),
    Vec2::new(1.0, 0.0),
    Vec2::new(1.0, 0.5),
    Vec2::new(1.0, 1.0),
    Vec2::new(0.5, 1.0),
    Vec2::new(0.0, 1.0),
    Vec2::new(0.0, 0.5),
];

#[derive(Clone, Debug)]
pub struct Editor {
    pub item: ItemId,
    pub tool: Tool,
    /// New shapes cut instead of keep.
    pub cut: bool,
    pub selected: Option<usize>,
    /// The selected point of a selected polygon.
    pub point: Option<usize>,
    /// Points of a polygon or stroke being drawn.
    pub drawing: Vec<Vec2>,
    /// A rectangle or ellipse being dragged out.
    pub draft: Option<MaskShape>,
    /// The pointer, for the rubber band of a polygon being drawn.
    pub pointer: Option<Vec2>,
    gesture: Option<Gesture>,
}

impl Editor {
    pub fn new(item: ItemId) -> Self {
        Self {
            item,
            tool: Tool::Select,
            cut: false,
            selected: None,
            point: None,
            drawing: Vec::new(),
            draft: None,
            pointer: None,
            gesture: None,
        }
    }

    pub fn set_tool(&mut self, tool: Tool) {
        self.tool = tool;
        self.cancel();
    }

    /// Drops a shape being drawn. Returns whether there was one.
    pub fn cancel(&mut self) -> bool {
        let busy = !self.drawing.is_empty() || self.draft.is_some() || self.gesture.is_some();
        self.drawing.clear();
        self.draft = None;
        self.gesture = None;
        busy
    }

    pub fn busy(&self) -> bool {
        self.gesture.is_some()
    }

    /// Unit-space handle positions of the selected shape.
    pub fn handles(&self, mask: &ItemMask) -> Vec<Vec2> {
        let Some(shape) = self.selected.and_then(|i| mask.shapes.get(i)) else {
            return Vec::new();
        };
        match &shape.geometry {
            MaskGeometry::Polygon { points } => points.clone(),
            g => {
                let (lo, hi) = g.bounds();
                BOX_HANDLES.iter().map(|f| lo + (hi - lo) * *f).collect()
            }
        }
    }

    pub fn down(
        &mut self,
        mask: &ItemMask,
        p: Vec2,
        frame: &Frame,
        px: f32,
        mods: Mods,
    ) -> Option<ItemMask> {
        let u = frame.to_unit(p)?;
        self.pointer = Some(u);
        match self.tool {
            Tool::Select => {
                if let Some(i) = self.selected.filter(|i| *i < mask.shapes.len()) {
                    let handle = self
                        .handles(mask)
                        .iter()
                        .position(|h| frame.to_canvas(*h).distance(p) <= HANDLE_PX * px);
                    if let Some(h) = handle {
                        let shape = &mask.shapes[i];
                        self.gesture = Some(match shape.geometry {
                            MaskGeometry::Polygon { .. } => {
                                self.point = Some(h);
                                Gesture::Point { index: h }
                            }
                            _ => Gesture::Resize {
                                handle: h,
                                orig: shape.clone(),
                            },
                        });
                        return None;
                    }
                }
                self.point = None;
                self.selected = (0..mask.shapes.len())
                    .rev()
                    .find(|i| contains(&mask.shapes[*i].geometry, u));
                if let Some(i) = self.selected {
                    self.gesture = Some(Gesture::Move {
                        start: u,
                        orig: mask.shapes[i].clone(),
                    });
                }
                None
            }
            Tool::Rect | Tool::Ellipse => {
                self.gesture = Some(Gesture::Draw { start: u });
                self.draft = None;
                None
            }
            Tool::Polygon => {
                let closes = self.drawing.len() >= 3
                    && frame.to_canvas(self.drawing[0]).distance(p) <= HANDLE_PX * px;
                if closes {
                    return self.close(mask);
                }
                let _ = mods;
                if self.drawing.len() < MAX_POINTS {
                    self.drawing.push(u);
                }
                None
            }
            Tool::Freehand => {
                self.drawing = vec![u];
                self.gesture = Some(Gesture::Stroke);
                None
            }
        }
    }

    pub fn moved(
        &mut self,
        mask: &ItemMask,
        p: Vec2,
        frame: &Frame,
        px: f32,
        mods: Mods,
    ) -> Option<ItemMask> {
        let u = frame.to_unit(p)?;
        self.pointer = Some(u);
        let i = self.selected;
        match self.gesture.clone()? {
            Gesture::Draw { start } => {
                let (lo, hi) = drag_box(start, u, frame.size(), mods);
                self.draft = Some(self.new_shape(box_geometry(self.tool, lo, hi)));
                None
            }
            Gesture::Move { start, orig } => {
                let mut shape = orig;
                translate(&mut shape.geometry, u - start);
                replace(mask, i?, shape)
            }
            Gesture::Resize { handle, orig } => {
                let mut shape = orig;
                resize(&mut shape.geometry, handle, u);
                replace(mask, i?, shape)
            }
            Gesture::Point { index } => {
                let mut shape = mask.shapes.get(i?)?.clone();
                if let MaskGeometry::Polygon { points } = &mut shape.geometry {
                    *points.get_mut(index)? = u;
                }
                replace(mask, i?, shape)
            }
            Gesture::Stroke => {
                let last = *self.drawing.last()?;
                if frame.to_canvas(last).distance(p) >= px && self.drawing.len() < 4 * MAX_POINTS {
                    self.drawing.push(u);
                }
                None
            }
        }
    }

    pub fn up(
        &mut self,
        mask: &ItemMask,
        p: Vec2,
        frame: &Frame,
        px: f32,
        mods: Mods,
    ) -> Option<ItemMask> {
        let u = frame.to_unit(p);
        let gesture = self.gesture.take()?;
        self.draft = None;
        match gesture {
            Gesture::Draw { start } => {
                let u = u?;
                let (lo, hi) = drag_box(start, u, frame.size(), mods);
                let span = frame.to_canvas(hi) - frame.to_canvas(lo);
                if span.abs().max_element() < MIN_DRAW_PX * px {
                    return None;
                }
                let shape = self.new_shape(box_geometry(self.tool, lo, hi));
                self.add(mask, shape)
            }
            Gesture::Stroke => {
                if let Some(u) = u {
                    self.drawing.push(u);
                }
                let canvas: Vec<Vec2> = self.drawing.iter().map(|u| frame.to_canvas(*u)).collect();
                self.drawing.clear();
                let mut points: Vec<Vec2> = simplify(&canvas, px.max(0.5))
                    .into_iter()
                    .filter_map(|p| frame.to_unit(p))
                    .collect();
                points.truncate(MAX_POINTS);
                if points.len() < 3 {
                    return None;
                }
                let shape = self.new_shape(MaskGeometry::Polygon { points });
                self.add(mask, shape)
            }
            _ => None,
        }
    }

    /// A double click: closes a polygon being drawn, or adds a point on the selected polygon's
    /// nearest edge.
    pub fn double(&mut self, mask: &ItemMask, p: Vec2, frame: &Frame, px: f32) -> Option<ItemMask> {
        if self.tool == Tool::Polygon {
            // The double click's own presses added the same point twice.
            self.drawing.dedup_by(|a, b| {
                frame.to_canvas(*a).distance(frame.to_canvas(*b)) <= HANDLE_PX * px
            });
            return self.close(mask);
        }
        let i = self.selected?;
        let MaskGeometry::Polygon { points } = &mask.shapes.get(i)?.geometry else {
            return None;
        };
        if points.len() >= MAX_POINTS {
            return None;
        }
        let u = frame.to_unit(p)?;
        let n = points.len();
        let (edge, dist) = (0..n)
            .map(|k| {
                let (a, b) = (
                    frame.to_canvas(points[k]),
                    frame.to_canvas(points[(k + 1) % n]),
                );
                (k, segment_distance(p, a, b))
            })
            .min_by(|a, b| a.1.total_cmp(&b.1))?;
        if dist > HANDLE_PX * px {
            return None;
        }
        let mut points = points.clone();
        points.insert(edge + 1, u);
        self.point = Some(edge + 1);
        let mut shape = mask.shapes[i].clone();
        shape.geometry = MaskGeometry::Polygon { points };
        replace(mask, i, shape)
    }

    /// Delete: the selected point (a polygon keeps three), else the selected shape.
    pub fn delete(&mut self, mask: &ItemMask) -> Option<ItemMask> {
        let i = self.selected.filter(|i| *i < mask.shapes.len())?;
        let mut next = mask.clone();
        if let (Some(k), MaskGeometry::Polygon { points }) =
            (self.point, &mut next.shapes[i].geometry)
            && points.len() > 3
            && k < points.len()
        {
            points.remove(k);
            self.point = None;
            return Some(next);
        }
        next.shapes.remove(i);
        self.selected = None;
        self.point = None;
        Some(next)
    }

    /// Enter: closes a polygon being drawn.
    pub fn enter(&mut self, mask: &ItemMask) -> Option<ItemMask> {
        (self.tool == Tool::Polygon).then(|| self.close(mask))?
    }

    fn close(&mut self, mask: &ItemMask) -> Option<ItemMask> {
        let points = std::mem::take(&mut self.drawing);
        if points.len() < 3 {
            return None;
        }
        let shape = self.new_shape(MaskGeometry::Polygon { points });
        self.add(mask, shape)
    }

    fn new_shape(&self, geometry: MaskGeometry) -> MaskShape {
        MaskShape {
            geometry,
            mode: if self.cut {
                MaskMode::Cut
            } else {
                MaskMode::Keep
            },
            feather: 0.0,
        }
    }

    /// Adds a finished shape, selects it and goes back to the Select tool.
    fn add(&mut self, mask: &ItemMask, shape: MaskShape) -> Option<ItemMask> {
        let mut next = mask.clone();
        if next.shapes.len() >= estream_project::mask::MAX_SHAPES {
            return None;
        }
        next.shapes.push(shape);
        self.selected = Some(next.shapes.len() - 1);
        self.point = None;
        self.tool = Tool::Select;
        Some(next)
    }
}

fn replace(mask: &ItemMask, i: usize, shape: MaskShape) -> Option<ItemMask> {
    if mask.shapes.get(i)? == &shape {
        return None;
    }
    let mut next = mask.clone();
    next.shapes[i] = shape;
    Some(next)
}

/// The box dragged from `start` to `u`: Shift makes it square on the canvas, Alt grows it from
/// the start as its center.
fn drag_box(start: Vec2, u: Vec2, size: Vec2, mods: Mods) -> (Vec2, Vec2) {
    let mut d = u - start;
    if mods.shift {
        let px = d * size;
        let side = px.abs().max_element();
        d = Vec2::new(side.copysign(px.x), side.copysign(px.y)) / size;
    }
    let (a, b) = if mods.alt {
        (start - d, start + d)
    } else {
        (start, start + d)
    };
    (a.min(b), a.max(b))
}

fn box_geometry(tool: Tool, lo: Vec2, hi: Vec2) -> MaskGeometry {
    match tool {
        Tool::Ellipse => MaskGeometry::Ellipse {
            center: (lo + hi) * 0.5,
            radii: (hi - lo) * 0.5,
        },
        _ => MaskGeometry::Rect {
            min: lo,
            max: hi,
            radius: 0.0,
        },
    }
}

fn translate(g: &mut MaskGeometry, d: Vec2) {
    match g {
        MaskGeometry::Rect { min, max, .. } => {
            *min += d;
            *max += d;
        }
        MaskGeometry::Ellipse { center, .. } => *center += d,
        MaskGeometry::Polygon { points } => points.iter_mut().for_each(|p| *p += d),
    }
}

/// Moves the sides of `g`'s box that `handle` (see [`BOX_HANDLES`]) holds to `u`.
fn resize(g: &mut MaskGeometry, handle: usize, u: Vec2) {
    let Some(f) = BOX_HANDLES.get(handle) else {
        return;
    };
    let (mut lo, mut hi) = g.bounds();
    if f.x == 0.0 {
        lo.x = u.x;
    } else if f.x == 1.0 {
        hi.x = u.x;
    }
    if f.y == 0.0 {
        lo.y = u.y;
    } else if f.y == 1.0 {
        hi.y = u.y;
    }
    let (lo, hi) = (lo.min(hi), lo.max(hi));
    match g {
        MaskGeometry::Rect { min, max, .. } => {
            *min = lo;
            *max = hi;
        }
        MaskGeometry::Ellipse { center, radii } => {
            *center = (lo + hi) * 0.5;
            *radii = (hi - lo) * 0.5;
        }
        MaskGeometry::Polygon { .. } => {}
    }
}

/// Whether unit point `u` is inside the shape (rounded corners count as square).
pub fn contains(g: &MaskGeometry, u: Vec2) -> bool {
    match g {
        MaskGeometry::Rect { .. } => {
            let (lo, hi) = g.bounds();
            u.cmpge(lo).all() && u.cmple(hi).all()
        }
        MaskGeometry::Ellipse { center, radii } => {
            let r = radii.abs().max(Vec2::splat(1e-6));
            ((u - *center) / r).length_squared() <= 1.0
        }
        MaskGeometry::Polygon { points } => {
            let mut inside = false;
            let n = points.len();
            for k in 0..n {
                let (a, b) = (points[k], points[(k + n - 1) % n]);
                if (a.y > u.y) != (b.y > u.y) && u.x < (b.x - a.x) * (u.y - a.y) / (b.y - a.y) + a.x
                {
                    inside = !inside;
                }
            }
            inside
        }
    }
}

fn segment_distance(p: Vec2, a: Vec2, b: Vec2) -> f32 {
    let ab = b - a;
    let t = if ab.length_squared() > 0.0 {
        ((p - a).dot(ab) / ab.length_squared()).clamp(0.0, 1.0)
    } else {
        0.0
    };
    p.distance(a + ab * t)
}

/// Ramer–Douglas–Peucker: drops points closer than `epsilon` to the simplified line.
pub fn simplify(points: &[Vec2], epsilon: f32) -> Vec<Vec2> {
    if points.len() < 3 {
        return points.to_vec();
    }
    let mut keep = vec![false; points.len()];
    keep[0] = true;
    keep[points.len() - 1] = true;
    let mut stack = vec![(0, points.len() - 1)];
    while let Some((a, b)) = stack.pop() {
        let far = (a + 1..b)
            .map(|k| (k, segment_distance(points[k], points[a], points[b])))
            .max_by(|x, y| x.1.total_cmp(&y.1));
        if let Some((k, d)) = far
            && d > epsilon
        {
            keep[k] = true;
            stack.push((a, k));
            stack.push((k, b));
        }
    }
    points
        .iter()
        .zip(keep)
        .filter_map(|(p, k)| k.then_some(*p))
        .collect()
}

/// SVG path commands for a shape's outline, in a 1000 × 1000 box over the item (`size` is the
/// item's size on the canvas, for corner radii).
pub fn outline(g: &MaskGeometry, size: Vec2) -> String {
    let v = |u: Vec2| u * 1000.0;
    match g {
        MaskGeometry::Rect { radius, .. } => {
            let (lo, hi) = g.bounds();
            let (lo, hi) = (v(lo), v(hi));
            let r = (Vec2::splat(*radius) / size * 1000.0)
                .min((hi - lo) * 0.5)
                .max(Vec2::ZERO);
            if r.min_element() <= 0.0 {
                return format!(
                    "M {} {} L {} {} L {} {} L {} {} Z",
                    lo.x, lo.y, hi.x, lo.y, hi.x, hi.y, lo.x, hi.y
                );
            }
            let arc = |x: f32, y: f32| format!("A {} {} 0 0 1 {x} {y}", r.x, r.y);
            format!(
                "M {} {} L {} {} {} L {} {} {} L {} {} {} L {} {} {} Z",
                lo.x + r.x,
                lo.y,
                hi.x - r.x,
                lo.y,
                arc(hi.x, lo.y + r.y),
                hi.x,
                hi.y - r.y,
                arc(hi.x - r.x, hi.y),
                lo.x + r.x,
                hi.y,
                arc(lo.x, hi.y - r.y),
                lo.x,
                lo.y + r.y,
                arc(lo.x + r.x, lo.y),
            )
        }
        MaskGeometry::Ellipse { center, radii } => {
            let (c, r) = (v(*center), v(radii.abs()).max(Vec2::splat(0.01)));
            format!(
                "M {} {} A {} {} 0 1 1 {} {} A {} {} 0 1 1 {} {} Z",
                c.x - r.x,
                c.y,
                r.x,
                r.y,
                c.x + r.x,
                c.y,
                r.x,
                r.y,
                c.x - r.x,
                c.y
            )
        }
        MaskGeometry::Polygon { points } => polyline(points, true),
    }
}

/// SVG commands through unit points (closed or open), in the 1000 × 1000 box.
pub fn polyline(points: &[Vec2], closed: bool) -> String {
    let mut out = String::new();
    for (i, p) in points.iter().enumerate() {
        let p = *p * 1000.0;
        out.push_str(&format!(
            "{} {} {} ",
            if i == 0 { "M" } else { "L" },
            p.x,
            p.y
        ));
    }
    if closed && points.len() > 2 {
        out.push('Z');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 200 × 100 item at (100, 50) on the canvas, unrotated; one screen px per canvas px.
    fn frame() -> Frame {
        Frame {
            world: Affine2::from_translation(Vec2::new(100.0, 50.0))
                * Affine2::from_scale(Vec2::new(200.0, 100.0)),
        }
    }

    /// Canvas point of item unit point.
    fn at(x: f32, y: f32) -> Vec2 {
        frame().to_canvas(Vec2::new(x, y))
    }

    fn drag(e: &mut Editor, mask: &ItemMask, from: Vec2, to: Vec2, mods: Mods) -> Option<ItemMask> {
        let f = frame();
        let a = e.down(mask, from, &f, 1.0, mods);
        let mid = e.moved(a.as_ref().unwrap_or(mask), (from + to) * 0.5, &f, 1.0, mods);
        let b = e.moved(mid.as_ref().unwrap_or(mask), to, &f, 1.0, mods);
        let latest = b.or(mid).or(a);
        let c = e.up(latest.as_ref().unwrap_or(mask), to, &f, 1.0, mods);
        c.or(latest)
    }

    fn approx(a: Vec2, b: Vec2) {
        assert!((a - b).abs().max_element() < 1e-4, "{a} != {b}");
    }

    #[test]
    fn draws_rectangles_and_ellipses() {
        let mut e = Editor::new(ItemId::new());
        e.set_tool(Tool::Rect);
        let mask = drag(
            &mut e,
            &ItemMask::default(),
            at(0.1, 0.2),
            at(0.5, 0.6),
            Mods::default(),
        )
        .unwrap();
        let MaskGeometry::Rect { min, max, .. } = mask.shapes[0].geometry else {
            panic!()
        };
        approx(min, Vec2::new(0.1, 0.2));
        approx(max, Vec2::new(0.5, 0.6));
        assert_eq!(mask.shapes[0].mode, MaskMode::Keep);
        assert_eq!(
            (e.tool, e.selected),
            (Tool::Select, Some(0)),
            "back to Select, selected"
        );

        // Shift: a circle on the canvas (equal pixels each way) even on a 2:1 item. Alt: from the
        // center. Cut mode.
        e.set_tool(Tool::Ellipse);
        e.cut = true;
        let mods = Mods {
            shift: true,
            alt: true,
        };
        let mask = drag(&mut e, &mask, at(0.5, 0.5), at(0.6, 0.9), mods).unwrap();
        let shape = &mask.shapes[1];
        assert_eq!(shape.mode, MaskMode::Cut);
        let MaskGeometry::Ellipse { center, radii } = shape.geometry else {
            panic!()
        };
        approx(center, Vec2::new(0.5, 0.5));
        approx(radii * Vec2::new(200.0, 100.0), Vec2::splat(40.0));

        // A click without a drag draws nothing.
        e.set_tool(Tool::Rect);
        assert!(drag(&mut e, &mask, at(0.2, 0.2), at(0.2, 0.2), Mods::default()).is_none());
    }

    #[test]
    fn selects_moves_and_resizes() {
        let mut e = Editor::new(ItemId::new());
        let mask = ItemMask::rounded(0.0);
        let mut small = ItemMask::default();
        small.shapes.push(MaskShape::keep(MaskGeometry::Rect {
            min: Vec2::new(0.2, 0.2),
            max: Vec2::new(0.4, 0.4),
            radius: 5.0,
        }));
        let mut both = mask.clone();
        both.shapes.extend(small.shapes);

        // A click picks the topmost shape under it; dragging moves it.
        let moved = drag(&mut e, &both, at(0.3, 0.3), at(0.4, 0.5), Mods::default()).unwrap();
        assert_eq!(e.selected, Some(1));
        let MaskGeometry::Rect { min, max, radius } = moved.shapes[1].geometry else {
            panic!()
        };
        approx(min, Vec2::new(0.3, 0.4));
        approx(max, Vec2::new(0.5, 0.6));
        assert_eq!(radius, 5.0, "the radius stays");
        assert_eq!(moved.shapes[0], both.shapes[0]);

        // The bottom right handle resizes.
        let resized = drag(&mut e, &moved, at(0.5, 0.6), at(0.7, 0.9), Mods::default()).unwrap();
        let MaskGeometry::Rect { min, max, .. } = resized.shapes[1].geometry else {
            panic!()
        };
        approx(min, Vec2::new(0.3, 0.4));
        approx(max, Vec2::new(0.7, 0.9));

        // A click on nothing clears the selection.
        let mut e = Editor::new(ItemId::new());
        e.down(&small_only(), at(0.9, 0.9), &frame(), 1.0, Mods::default());
        assert_eq!(e.selected, None);
    }

    fn small_only() -> ItemMask {
        let mut m = ItemMask::default();
        m.shapes.push(MaskShape::keep(MaskGeometry::FULL));
        m.shapes[0].geometry = MaskGeometry::Rect {
            min: Vec2::splat(0.1),
            max: Vec2::splat(0.2),
            radius: 0.0,
        };
        m
    }

    #[test]
    fn polygons_by_clicks_and_editing_points() {
        let f = frame();
        let mut e = Editor::new(ItemId::new());
        e.set_tool(Tool::Polygon);
        e.cut = true;
        let empty = ItemMask::default();
        for p in [at(0.1, 0.1), at(0.5, 0.1), at(0.5, 0.5), at(0.1, 0.5)] {
            assert!(e.down(&empty, p, &f, 1.0, Mods::default()).is_none());
            e.up(&empty, p, &f, 1.0, Mods::default());
        }
        assert_eq!(e.drawing.len(), 4);
        // A click on the first point closes it.
        let mask = e
            .down(&empty, at(0.1, 0.1) + 2.0, &f, 1.0, Mods::default())
            .unwrap();
        let MaskGeometry::Polygon { points } = &mask.shapes[0].geometry else {
            panic!()
        };
        assert_eq!(points.len(), 4);
        assert_eq!(mask.shapes[0].mode, MaskMode::Cut);
        assert!(e.drawing.is_empty() && e.selected == Some(0));

        // A double click on an edge adds a point there; Delete removes the selected point.
        let more = e.double(&mask, at(0.3, 0.1), &f, 1.0).unwrap();
        let MaskGeometry::Polygon { points } = &more.shapes[0].geometry else {
            panic!()
        };
        assert_eq!(points.len(), 5);
        approx(points[1], Vec2::new(0.3, 0.1));
        assert_eq!(e.point, Some(1));
        // Dragging a point moves it.
        let dragged = drag(&mut e, &more, at(0.3, 0.1), at(0.3, 0.0), Mods::default()).unwrap();
        let MaskGeometry::Polygon { points } = &dragged.shapes[0].geometry else {
            panic!()
        };
        approx(points[1], Vec2::new(0.3, 0.0));
        let fewer = e.delete(&dragged).unwrap();
        assert_eq!(fewer.shapes.len(), 1, "a point went, not the shape");
        // With no point selected, Delete removes the shape.
        assert!(e.delete(&fewer).unwrap().shapes.is_empty());

        // Enter closes, Escape drops a polygon being drawn.
        e.set_tool(Tool::Polygon);
        for p in [at(0.1, 0.1), at(0.5, 0.1), at(0.5, 0.5)] {
            e.down(&empty, p, &f, 1.0, Mods::default());
        }
        assert_eq!(e.enter(&empty).unwrap().shapes.len(), 1);
        e.set_tool(Tool::Polygon);
        e.down(&empty, at(0.1, 0.1), &f, 1.0, Mods::default());
        assert!(e.cancel() && e.drawing.is_empty());
        assert!(!e.cancel(), "nothing left to cancel");
    }

    #[test]
    fn freehand_strokes_are_simplified() {
        let f = frame();
        let mut e = Editor::new(ItemId::new());
        e.set_tool(Tool::Freehand);
        let empty = ItemMask::default();
        e.down(&empty, at(0.0, 0.0), &f, 1.0, Mods::default());
        // A triangle traced with many points per side.
        let corners = [at(0.0, 0.0), at(1.0, 0.0), at(0.5, 1.0), at(0.0, 0.0)];
        for w in corners.windows(2) {
            for k in 1..=40 {
                let p = w[0].lerp(w[1], k as f32 / 40.0);
                e.moved(&empty, p, &f, 1.0, Mods::default());
            }
        }
        let mask = e
            .up(&empty, at(0.0, 0.0), &f, 1.0, Mods::default())
            .unwrap();
        let MaskGeometry::Polygon { points } = &mask.shapes[0].geometry else {
            panic!()
        };
        assert!(points.len() <= 5, "{} points", points.len());
        assert!(contains(&mask.shapes[0].geometry, Vec2::new(0.5, 0.4)));
        assert!(!contains(&mask.shapes[0].geometry, Vec2::new(0.1, 0.9)));
    }

    #[test]
    fn handles_follow_rotation() {
        // Rotated 90°: unit x runs down the canvas.
        let f = Frame {
            world: Affine2::from_translation(Vec2::new(300.0, 0.0))
                * Affine2::from_angle(std::f32::consts::FRAC_PI_2)
                * Affine2::from_scale(Vec2::new(200.0, 100.0)),
        };
        let mut e = Editor::new(ItemId::new());
        e.set_tool(Tool::Rect);
        let start = f.to_canvas(Vec2::new(0.25, 0.25));
        let end = f.to_canvas(Vec2::new(0.75, 0.5));
        e.down(&ItemMask::default(), start, &f, 1.0, Mods::default());
        let mask = e
            .up(&ItemMask::default(), end, &f, 1.0, Mods::default())
            .unwrap();
        let MaskGeometry::Rect { min, max, .. } = mask.shapes[0].geometry else {
            panic!()
        };
        approx(min, Vec2::new(0.25, 0.25));
        approx(max, Vec2::new(0.75, 0.5));
        approx(e.handles(&mask)[4], Vec2::new(0.75, 0.5));
    }

    #[test]
    fn outlines_are_path_commands() {
        let size = Vec2::new(200.0, 100.0);
        assert_eq!(
            outline(&MaskGeometry::FULL, size),
            "M 0 0 L 1000 0 L 1000 1000 L 0 1000 Z"
        );
        let rounded = outline(
            &MaskGeometry::Rect {
                min: Vec2::ZERO,
                max: Vec2::ONE,
                radius: 20.0,
            },
            size,
        );
        assert!(
            rounded.starts_with("M 100 0 L 900 0 A 100 200 0 0 1 1000 200"),
            "{rounded}"
        );
        assert!(
            outline(
                &MaskGeometry::Ellipse {
                    center: Vec2::splat(0.5),
                    radii: Vec2::splat(0.5)
                },
                size
            )
            .contains(" A 500 500 ")
        );
        assert_eq!(polyline(&[Vec2::ZERO, Vec2::X], false), "M 0 0 L 1000 0 ");
    }
}
