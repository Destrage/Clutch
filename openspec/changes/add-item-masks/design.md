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
    pub feather: f32,       // pixels of the source after crop
}
pub enum MaskGeometry {
    Rect { min: [f32; 2], max: [f32; 2], radius: f32 },
    Ellipse { center: [f32; 2], radii: [f32; 2] },
    Polygon { points: Vec<[f32; 2]> },   // polygon and freehand (simplified)
}
```
- Coordinates are 0..1 over the item's visible source (after crop), so the mask follows the item's
  transform and keeps its place when the source resolution changes.
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
`SetItemMask { scene, item, mask: Option<ItemMask> }`, invertible. Dragging a handle coalesces
into one undo step, as for transforms.

## Rendering
- The engine rasterizes a mask with `tiny-skia` (already a dependency through Slint) into an R8
  texture when the mask, the item's crop or the item's on-canvas size changes, never per frame.
  Texture size: the item's size on the canvas, at most 2048 px on the long side. Feather is a
  Gaussian blur of each shape's coverage before it is combined.
- The compositor's item uniform gets a `masked` flag. Masked items bind a mask texture at group 2
  (a 1×1 white texture otherwise) and the fragment shader multiplies alpha by the mask sampled at
  the quad's unit coordinate. One extra texture sample per masked pixel.
- The same path draws masked items into every canvas and into the studio preview.

## Editing (`apps/estream`)
- Mask mode is a preview overlay drawn by the app (like the transform handles), in canvas pixels,
  converted to item space through the inverse item transform.
- Tools: Rectangle, Ellipse, Polygon, Freehand, and Select (move shapes, drag handles and points).
  Shift keeps squares and circles; Alt draws from the center.
- While editing, the outside of the mask is dimmed in the preview only (not in outputs).
- The inspector Mask panel lists shapes with keep/cut toggles, feather sliders, reorder and delete;
  presets replace the shape list (one undo step).
