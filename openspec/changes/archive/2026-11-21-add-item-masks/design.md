# Design: add-item-masks

## Model (`estream-project`)
```rust
pub struct ItemMask {
    pub enabled: bool,
    pub invert: bool,
    pub shapes: Vec<MaskShape>,
}
pub struct MaskShape {
    pub geometry: MaskGeometry,
    pub mode: MaskMode,     // Keep | Cut
    pub feather: f32,       // canvas pixels
}
pub enum MaskGeometry {
    Rect { min: [f32; 2], max: [f32; 2], radius: f32 },
    Ellipse { center: [f32; 2], radii: [f32; 2] },
    Polygon { points: Vec<[f32; 2]> },   // polygon and freehand (simplified)
}
```
- Coordinates are 0..1 over the item's visible source (after crop), so the mask follows the item's
  transform and keeps its place when the source resolution changes.
- Lengths (corner radius, feather, effect sizes and offsets) are canvas pixels at the item's
  current size, as the user sees them; rescaling an item rebakes its mask.
- `SceneItem::mask: Option<ItemMask>` with `#[serde(default, skip_serializing_if = "Option::is_none")]`.
- Shapes combine in order from "nothing" (or "everything" when the first shape cuts): keep =
  max(coverage, shape), cut = coverage × (1 − shape). Invert flips the result.
- Freehand strokes are simplified (Ramer–Douglas–Peucker, 0.5 px at the drawing size) into polygons.

## Edge effects
```rust
pub struct ItemMask { /* … */ pub effects: MaskEffects }
pub struct MaskEffects {
    pub border: Option<Border>,   // color, width, placement: Inside | Center | Outside
    pub glow: Option<Glow>,       // color, size, strength, placement: Outside | Inside | Both
    pub shadow: Option<Shadow>,   // color, offset, softness, opacity
}
```
- An item with effects but no shapes uses its whole visible rectangle (after crop) as the shape,
  so "rounded corners + border + shadow" on a camera needs no drawing.
- Effects are computed from the mask's coverage when the mask, crop or on-canvas size changes:
  - outer glow and shadow: blur of the coverage (shadow offset), outside the shape only;
  - inner glow: blur of the inverted coverage, inside the shape only;
  - border: coverage dilated/eroded by the width (distance from the edge), minus the shape for
    an outside border.
- They are baked into two premultiplied RGBA textures covering the item expanded by the largest
  effect extent: an **under** layer (shadow, outer glow) drawn just before the item, and an
  **over** layer (inner glow, border) drawn just after it. Both are ordinary quads with the
  item's transform, so they need no new compositor features beyond a texture binding.
- Effect sizes are in canvas pixels at the item's current scale; rescaling the item rebakes them.

## Commands
A `mask: Option<Option<ItemMask>>` field on the existing `UpdateItem` patch (`null` over JSON removes
the mask), so masks get undo, coalescing and remote control like every other item edit. A mask is
validated (finite numbers, at most 64 shapes and 4096 points per polygon) and refused on groups.
A gesture (drawing a shape, dragging a handle) is one undo step.

## Rendering
- The engine rasterizes a mask with `tiny-skia` (already a dependency through Slint) into an R8
  texture when the mask, the item's crop or the item's on-canvas size changes, never per frame.
  Baking runs on a worker thread; the video thread only sends jobs and uploads results, and keeps
  drawing the previous textures (stretched over the item) until new ones arrive. A newly masked
  item is left out until its first mask is ready; headless renders wait for it.
  Texture size: the item's size on the canvas, at most 2048 px on the long side. Feather is a
  Gaussian blur of each shape's coverage before it is combined.
- Every item binds a mask texture at group 2 (a 1×1 white texture when unmasked, which multiplies
  by exactly 1, so no flag is needed) and the fragment shader multiplies alpha by the mask sampled at
  the quad's unit coordinate. One extra texture sample per masked pixel.
- The same path draws masked items into every canvas and into the studio preview.

## Editing (`apps/estream`)
- Mask mode is a preview overlay drawn by the app (like the transform handles), in canvas pixels,
  converted to item space through the inverse item transform.
- Tools: Rectangle, Ellipse, Polygon, Freehand, and Select (move shapes, drag handles and points).
  Shift keeps squares and circles; Alt draws from the center.
- While editing, the outside of the mask is dimmed in the preview only (not in outputs): the app
  bakes a small (256 px) copy of the mask with the engine's baker and draws it darkened over the
  item. Shape outlines are SVG paths and handles are drawn in the item's rotated box.
- Keys while editing: V, R, E, P, F pick the tools, X switches Keep and Cut, Enter closes a polygon,
  Delete removes the selected point or shape, Esc cancels a shape being drawn and then leaves.
  "Edit mask of selected item" is also in the command palette.
- The inspector Mask panel lists shapes with keep/cut toggles, feather sliders, reorder and delete;
  presets replace the shape list (one undo step).
