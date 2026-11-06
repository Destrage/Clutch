# Design: improve-canvas-fill-and-color

## Paint and gradients
`estream-core` gets one paint type, used by the Color source, item fill and canvas background:

```rust
pub enum Paint {
    Solid(Rgba8),
    Linear { angle_deg: f32, stops: Vec<GradientStop> },
    Radial { center: [f32; 2], radius: f32, stops: Vec<GradientStop> },
}
pub struct GradientStop { pub color: Rgba8, pub at: f32 } // at in 0..=1
```

- 2 to 8 stops, sorted by `at`. Validation clamps and sorts; fewer than 2 stops reads as solid.
- Interpolation is in linear light, which avoids the grey band between complementary colors that
  sRGB interpolation gives.
- Gradients are rendered once, when settings change, never per frame. A small WGSL pass draws
  the gradient into an `Rgba8UnormSrgb` texture at the target's size (long side capped at 4096)
  with a baked ordered dither, so the 8-bit encode doesn't band. Solid paint keeps today's 1×1
  texture.
- Property system: a new `PropKind::Gradient` whose value is the serialized `Paint`. The Color
  source keeps its `color` key and gains `style` (`solid` | `linear` | `radial`) and `gradient`.
  With `style` missing or `solid`, it renders `color` exactly as today.
- Text keeps its two-color top-to-bottom gradient for now; moving it to `Paint` is a later step.

## Fill around fitted items
`SceneItem` gains `fill: ItemFill` (serde default `None`):

```rust
pub enum ItemFill {
    None,
    Frosted { blur: f32, dim: f32, saturation: f32 }, // 0..=1 each
    Paint(Paint),
}
```

- The fill covers the item's bounds box and is drawn directly beneath the item, with the item's
  opacity and blend mode. It is shown in the Layout tab only when the bounds mode is Fit (the
  only mode that leaves empty space in the box). For other modes the setting is kept but has no
  effect.
- Source items only. Groups don't get a fill in this change.
- **Frosted** renders the item's cropped source frame scaled to *cover* the bounds box:
  1. Downscale into a small pool texture whose long side is 1/8 of the bounds box (clamped to
     64..=480 px). The cover UV rect is computed on the CPU each frame (matrix math only).
  2. Two-pass gaussian blur at that size, reusing the blur filter's passes.
  3. Composite: bilinear upscale into the bounds box with dim and saturation applied in the
     item shader (two uniforms, no new pipeline variant if the existing uniform padding fits).
  The small textures are allocated when the fill is set or the bounds change, never per frame.
  At 1080 × 1920 the blur runs on about 135 × 240 pixels, a tiny fraction of the frame.
- **Paint** fills the bounds box with a solid or gradient texture rendered as above.
- Defaults: every item, new or existing, starts with `None`, so the empty space shows the
  canvas background (black unless changed) and updating Clutch never changes how a saved scene
  looks. Choosing Frosted starts from `Frosted { blur: 0.6, dim: 0.25, saturation: 0.9 }`.
- Copy to canvas and Duplicate canvas copy `fill` with the item.

## Canvas background and settings
- `Canvas` gains `background_paint: Option<Paint>`. `background` (Rgba8) stays and is written as
  the solid color or the gradient's first stop, so an older version still opens the project and
  shows a sensible color. The renderer uses `background_paint` when present: solid paint is the
  clear color, a gradient is a cached canvas-sized texture drawn first.
- `CanvasPatch` gets `background_paint`. Size, frame rate and background changes already go
  through `UpdateCanvas`, so they are undoable and reach the remote API.
- Canvas settings dialog: name; size from the presets or custom (even numbers, 128 to 4096 per
  side); frame rate 24, 25, 30, 48, 50, 60, 29.97 or 59.94; background (Color / Gradient).
- "Scale items to the new size" (on by default) adds `UpdateItem` commands to the same batch,
  scaling each item's position and bounds by the width and height ratios, so one undo restores
  everything.
- Size and frame rate are disabled while a stream, recording or replay buffer uses the canvas,
  with the hint "Stop the stream, recording and replay buffer on this canvas to change its size
  or frame rate." Restarting encoders mid-stream is out of scope.

## Presets
Grouped landscape first, then vertical:

| Label | Size | FPS |
|---|---|---|
| 1080p · 60 fps | 1920 × 1080 | 60 |
| 1080p · 30 fps | 1920 × 1080 | 30 |
| 1440p · 60 fps | 2560 × 1440 | 60 |
| 720p · 60 fps | 1280 × 720 | 60 |
| 4K · 30 fps | 3840 × 2160 | 30 |
| 4K · 60 fps | 3840 × 2160 | 60 |
| Vertical 1080 × 1920 · 30 fps | 1080 × 1920 | 30 |
| Vertical 1080 × 1920 · 60 fps | 1080 × 1920 | 60 |
| Vertical 2K 1440 × 2560 · 60 fps | 1440 × 2560 | 60 |
| Vertical 4K 2160 × 3840 · 30 fps | 2160 × 3840 | 30 |
| Vertical 4K 2160 × 3840 · 60 fps | 2160 × 3840 | 60 |

`canvas_menu` and Duplicate canvas pass a preset *index*, so the existing entries keep their
order and the new ones are appended to `PRESETS`; the menus sort for display. The 2K and 4K
vertical details say "recording and YouTube", since many platforms take at most 1080p.

Risk: some H.264 hardware encoders reject 2160 × 3840. The existing encoder-open error is shown;
its hint gains "Try HEVC or AV1, or a 1440 × 2560 canvas" when the canvas is taller than 2304.

## Color picker
- A Slint popup opened from any `color` property swatch. Hue ring around a saturation/brightness
  square. The square is two native linear gradients (white to hue, transparent to black). The
  ring is a Rust-generated image, made once per size, since Slint has no conic gradient.
- Opacity slider over a checkerboard; hex field (`#rgb`, `#rrggbb`, `#rrggbbaa`); R, G, B and
  opacity number fields; the eight preset swatches; the last 12 picked colors, kept in settings.
- Live edits: dragging sends `SetSourceSettings` (or the matching command) continuously. Those
  commands already coalesce per target (`Command::coalesce_key`), and the picker calls
  `History::seal` when it closes, so one session is one undo step.
- The gradient editor reuses the picker: a bar with draggable stops (click to add, drag off or
  Delete to remove), an angle dial for linear, and a center point for radial.

## Eyedropper
- **From the Preview** (all platforms): while picking, the Preview shows a crosshair and a
  magnified loupe. A click asks the engine for one pixel of the Preview's canvas texture. The
  video thread copies 1 × 1 texel into a reused staging buffer after the frame, maps it
  asynchronously, and replies over the existing event channel (Rgba16Float to sRGB Rgba8). No
  per-frame cost when not picking.
- **From the screen**: a platform trait `pick_screen_color() -> Option<Rgba8>` (async, None if
  cancelled).
  - Windows (this change): grab every monitor once (GDI `BitBlt` of the virtual screen), show it
    in borderless topmost windows with the loupe; click picks, Esc cancels. Because the image is
    frozen, clicking doesn't reach the app underneath.
  - macOS: `NSColorSampler`. Linux: the `org.freedesktop.portal.Screenshot.PickColor` portal.
    Both are left to their platform branches. Until they land, the screen option is hidden there.

## Removing sounds from the mixer
- Sound knobs (Stage and Live deck): a right-click menu with Settings, Mute and Remove. Full
  mixer rows: a Remove button next to Mute. Sound settings panel: "Remove source" (or "Remove
  from scene") at the bottom. Delete also removes a sound picked in the mixer.
- A source heard in all scenes: `RemoveSource` (it has no scene items). A source in the current
  scene: the same logic as `remove_selected` (remove the item; remove the source too when it was
  the last use). Toast: "Removed Desktop Audio — Ctrl+Z to undo".
- Nothing re-adds Desktop Audio by itself: setups only run on first run or when picked.
