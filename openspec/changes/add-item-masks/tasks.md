# Tasks: add-item-masks

## 1. Model
- [ ] 1.1 `ItemMask`, `MaskShape`, `MaskGeometry`, `MaskMode` on `SceneItem` (serde default, skipped when absent)
- [ ] 1.2 `SetItemMask` command with undo and coalescing; persistence round-trip test
- [ ] 1.3 Mask coverage combination (keep/cut in order, invert) with unit tests on a small grid

## 2. Rendering
- [ ] 2.1 Rasterize masks with `tiny-skia` into R8 textures (rect with radius, ellipse, polygon, feather), rebuilt only on change
- [ ] 2.2 Compositor: `masked` flag, mask bind group, alpha multiply; unmasked items unchanged
- [ ] 2.3 Golden-image tests: circle keep, rounded rect with a cut circle, feathered edge, inverted mask, rotated item

- [ ] 2.4 Edge effects: bake under (shadow, outer glow) and over (inner glow, border) layers from the mask coverage; draw them around the item
- [ ] 2.5 Golden-image tests: outside border on a circle, outer glow past the item bounds, offset shadow, effects on an item without shapes

## 3. Editing
- [ ] 3.1 Preview mask mode: draw rectangle, ellipse, polygon and freehand; select, move, resize and edit points; dim outside
- [ ] 3.2 Inspector Mask panel: shape list, keep/cut, feather, reorder, delete, invert, on/off, Edit mask
- [ ] 3.3 Presets: Circle, Rounded corners, Cut a corner, Diagonal split; effect styles Soft shadow, Neon glow, Thin border
- [ ] 3.5 Effects section in the Mask panel: border, glow and shadow with color pickers and sliders
- [ ] 3.4 Keyboard access and screen-reader labels for the panel and tools

## 4. Verification
- [ ] 4.1 `cargo test` and `cargo clippy --all-targets -- -D warnings` for the touched crates
- [ ] 4.2 Masked camera and game capture while streaming and recording at 1080p60: masks look the same in preview, stream and recording; no extra dropped frames
