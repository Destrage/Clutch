# Tasks: improve-canvas-fill-and-color

## 1. Removing sounds from the mixer
- [x] 1.1 Sound knobs: right-click menu (Settings, Mute, Remove); full mixer rows: Remove
      button; sound settings panel: "Remove source" / "Remove from scene" button
- [x] 1.2 App: `mixer_remove(id)`: `RemoveSource` for a source heard in all scenes; for one in
      the current scene, the `remove_selected` logic (item, or source when last use); toast
      with Ctrl+Z hint
- [x] 1.3 Test: removing a global source and undoing restores its volume and filters

## 2. Presets and canvas settings
- [x] 2.1 Add the new presets to `PRESETS` (vertical 1080 × 1920 60, 1440 × 2560 60,
      2160 × 3840 30 and 60, landscape 4K 60), landscape first, then vertical (preset indices
      are only used live in the UI, never saved)
- [x] 2.2 (moved to 3.5: the background gradient needs `Paint`)
- [x] 2.3 Canvas settings dialog from the canvas tab menu: name, size (preset or custom, even,
      128 to 4096), frame rate (24, 25, 29.97, 30, 48, 50, 59.94, 60), solid background
- [x] 2.4 "Scale items to the new size" (default on): one `Batch` of `UpdateCanvas` plus
      `UpdateItem` per top-level item; test that one undo restores canvas and items
- [x] 2.5 Lock size and frame rate while a stream, recording, replay buffer or virtual camera
      uses the canvas, with the hint
- [x] 2.6 Encoder-open error hint for canvases taller than 2304: "Try HEVC or AV1, or a
      1440 × 2560 canvas"
- [x] 2.7 Check: recorded a 2160 × 3840 at 60 fps canvas with NVENC H.264 on an RTX 4070 Ti
      SUPER (ffprobe: h264 2160 × 3840, 60/1, 370 frames in 6.2 s). Later (2026-11-07):
      streamed it to a local RTMP receiver with NVENC H.264 (59.98 fps for 55 s) and HEVC
      (60 fps for 53 s, nothing dropped), and recorded it with HEVC (60 fps)

## 3. Paint and gradients
- [x] 3.1 `estream-core`: `Paint`, `GradientStop`; validation (2 to 8 stops, sorted, clamped);
      serde tests
- [x] 3.2 `estream-gpu`: gradient pass (linear and radial, linear-light blend, baked
      interleaved-gradient-noise dither) rendering into a texture; pixel tests against the
      CPU reference (`Paint::color_at`) and a banding test
- [x] 3.3 `PropKind::Gradient` in the property system, with validation, and its inspector
      editor (see 4.4)
- [x] 3.4 Color source: `style` and `gradient` settings; render once on update; old settings
      render the solid color unchanged (tests, plus an end-to-end render)
- [x] 3.5 `Canvas::background_paint` in the model (serde default `None`) and
      `CanvasPatch::background_paint` (applied and inverted by `UpdateCanvas`, tested);
      `background` written as the solid color or first stop; renderer draws a cached gradient
      texture first in program, transition and Preview passes (end-to-end test); Canvas
      settings gets Solid color / Linear gradient / Radial gradient

## 4. Color picker
- [x] 4.1 Picker (`color.slint`, one per window via the `ColorPicking` global): hue ring image
      (Rust, once; orientation tested), saturation/brightness square, opacity slider on a
      checkerboard, hex and R/G/B/A fields, preset swatches, Done; Esc or a click outside closes
- [x] 4.2 Recent colors (last 12, newest first, no repeats) in settings (tested)
- [x] 4.3 Every `color` property, gradient colors and the canvas background open the picker
      through `ColorSwatch`; live edits; the engine holds the undo run while the picker is
      open (`Engine::hold_history`) and seals it on close (test: pauses don't split the step).
      Item fill uses the same swatch in 5.5
- [x] 4.4 Gradient editor: preview bar with a handle per color (click the bar to add a color
      with the color at that point, drag a handle between its neighbours), Remove button,
      selected color's swatch and hex, Angle slider (linear), Center X/Y and Size sliders
      (radial); shared by the inspector and Canvas settings
- [x] 4.5 Eyedropper from the Preview: crosshair and loupe; one-pixel readback of the shown
      picture (`readback::read_pixel`, tested); nothing runs when not picking; Esc cancels
- [x] 4.6 Screen color picking on Windows: press the screen button and drag, release over the
      color (`screen_color`: `GetCursorPos` + `GetPixel`, polled every 30 ms while dragging)
- [x] 4.7 The screen button is hidden where the platform has no implementation yet (macOS
      `NSColorSampler` and the Linux `PickColor` portal come in their platform branches)
- [x] 4.8 Checked in the app: picker layout (card widened so the A field fits), ring and
      square, recent colors, Preview eyedropper (picked the test pattern's #bf0000 exactly), one
      Ctrl+Z undoes a whole picker session
- [x] 4.9 Screen eyedropper by hand: verified by the user (2026-11-06)

## 5. Fill around fitted items
- [x] 5.1 `SceneItem::fill: ItemFill` (serde default `None`, not written when `None`);
      `ItemPatch::fill` applied and inverted by `UpdateItem`; Copy to canvas and Duplicate
      canvas clone it with the item; tests
- [x] 5.2 Render graph: `FlatItem` carries the item id and its fill (Fit bounds only); the fill
      is drawn just beneath the item over its bounds box, with the item's opacity and blend
- [x] 5.3 Frosted (`estream-engine/src/fill.rs`): cover UV on the CPU
      (`ItemTransform::cover_uv`, tested); the covering picture is drawn into a small texture
      (a quarter of the box, 96 to 640 px on the long side), then a filter chain blurs it and
      applies dim (exact brightness multiply via color correction) and saturation; GPU state is
      kept per item and reallocated only when the fill or box changes
- [x] 5.4 Color and Gradient fill using the paint textures from 3.2
- [x] 5.5 Layout tab: "Fill empty space" panel, shown when Size mode is Fit: None, Frosted (Blur,
      Dim, Saturation sliders), Color (swatch opens the picker, hex), Gradient (the gradient
      editor); folds like the other panels
- [x] 5.6 Every item starts with None; items saved by older versions load with None (test)
- [x] 5.7 End-to-end pixel test: 16:9 bars fitted in a 9:16 canvas with None (black), Color
      (exact color) and Frosted (blurred bars above and below, source unchanged);
      `ESTREAM_FILL_PNG=<folder>` writes the pictures
- [x] 5.8 Measured (`frosted_fill_cost`, ignored test, release): a 1080 × 1920 at 60 fps
      canvas with a 1920 × 1080 source takes 0.18 ms a frame without fill, 0.30 ms with
      Frosted (+0.12 ms) and 0.31 ms at Blur 100 % on the development machine

## 6. Finish
- [x] 6.1 `cargo clippy --workspace --all-targets -- -D warnings` and tests for the touched
      crates
- [x] 6.2 Checked in the app (driven by window messages, screenshots): remove and undo Desktop
      Audio from the knob menu; Canvas settings to 60 fps, gradient background, 4K vertical with
      items scaled; Frosted, Color and Gradient fills on a fitted 16:9 picture; radial Color
      source gradient with an added color; settings survive a restart. Fixed on the way: wider
      canvas menu (long preset names were cut), W/H labels in Canvas settings, picker width,
      stronger maximum blur (radius 48)
