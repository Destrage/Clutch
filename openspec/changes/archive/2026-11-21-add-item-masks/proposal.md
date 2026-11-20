# Change: Add shape masks and edge effects to scene items

## Why
A source can only be cropped to a rectangle, keyed by color, or masked with an image made in
another app. Users want to shape a source directly on the canvas: a round or rounded camera, a
camera cut to the outline of a desk or a doorway, a game with the minimap or a HUD corner cut away
so another source shows through, or a diagonal split between two sources. Everything outside the
shape should be transparent, so whatever is below (or another source placed there) is seen.

## What Changes
- **Masks on scene items** (any source: game, window and display capture, camera, media, image,
  browser). A mask is a list of shapes in the item's own space, so it moves, scales and rotates
  with the item:
  - Shapes: rectangle (with corner radius), ellipse, polygon (click points), freehand (drag).
  - Each shape **keeps** or **cuts**. Shapes apply in order, so a camera can keep a rounded
    rectangle and cut a circle out of it.
  - Per shape: feather (soft edge, in pixels). Per mask: invert, and on/off.
- **Edge effects** that follow the visible shape (or the whole item when it has no shapes):
  - **Border**: color, width, inside / centered / outside the edge.
  - **Glow**: color, size, strength, outside / inside / both.
  - **Shadow**: color, offset, softness, opacity.
  Effects can extend past the item (a glow around a round camera) and appear in every output.
- **Presets** for the common cases: Circle, Rounded corners (one radius slider), Cut a corner,
  Diagonal split; and effect styles such as Soft shadow, Neon glow and Thin border.
- **Editing on the preview**: with an item selected, "Edit mask" shows the shapes with handles.
  Drag to draw, drag handles to resize, drag points to reshape, double-click an edge to add a
  point, Delete to remove. Everything outside the mask is dimmed while editing. Esc or Done leaves.
- **Mask panel** in the item inspector: the shape list (reorder, keep/cut, feather, delete),
  invert, the presets, and "Edit mask".
- Every edit is a command (undo/redo, remote control) and is saved with the scene collection.
- Rendering stays on the GPU: the shapes are drawn into a small alpha texture only when the mask
  changes, and the compositor multiplies the item's alpha by it. Items without a mask draw as today.

Not in this change: masks on groups, animated or tracked masks, masks from AI background
removal (`add-ai-features`), and effects that follow a source's own changing transparency (a
chroma-keyed person); effects follow the mask shape.

## Impact
- Affected specs: `scene-composition` (item masks), `studio-ui` (mask editing)
- Affected code: `estream-project` (mask model, commands, persistence), `estream-engine` (mask
  textures), `estream-gpu` (compositor mask sample), `apps/estream` (preview tool, inspector panel)
- Older Clutch versions ignore the new field and show those items unmasked.
