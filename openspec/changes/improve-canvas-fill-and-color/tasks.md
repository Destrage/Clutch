# Tasks: improve-canvas-fill-and-color

## 1. Removing sounds from the mixer
- [ ] 1.1 Mixer strip: right-click menu and menu button with Remove; sound settings panel:
      "Remove source" button
- [ ] 1.2 App: `mixer_remove(id)`: `RemoveSource` for a source heard in all scenes; for one in
      the current scene, the `remove_selected` logic (item, or source when last use); toast
      with Ctrl+Z hint
- [ ] 1.3 Test: removing a global source and undoing restores its volume and filters

## 2. Presets and canvas settings
- [ ] 2.1 Append the new presets to `PRESETS` (vertical 1080 × 1920 60, 1440 × 2560 60,
      2160 × 3840 30 and 60, landscape 4K 60); menus list landscape first, then vertical,
      keeping preset indices stable
- [ ] 2.2 `CanvasPatch::background_paint`; `UpdateCanvas` applies and inverts it; tests
- [ ] 2.3 Canvas settings dialog from the canvas tab menu: name, size (preset or custom, even,
      128 to 4096), frame rate (24, 25, 30, 48, 50, 60, 29.97, 59.94), background
- [ ] 2.4 "Scale items to the new size" (default on): one `Batch` of `UpdateCanvas` plus
      `UpdateItem` per item; test that one undo restores canvas and items
- [ ] 2.5 Lock size and frame rate while a stream, recording or replay buffer uses the canvas,
      with the hint
- [ ] 2.6 Encoder-open error hint for canvases taller than 2304: "Try HEVC or AV1, or a
      1440 × 2560 canvas"
- [ ] 2.7 Check: stream and record a 2160 × 3840 at 60 fps canvas with NVENC H.264 and HEVC

## 3. Paint and gradients
- [ ] 3.1 `estream-core`: `Paint`, `GradientStop`; validation (2 to 8 stops, sorted, clamped);
      serde tests
- [ ] 3.2 `estream-gpu`: gradient pass (linear and radial, linear-light blend, baked ordered
      dither) rendering into a texture; golden-image test
- [ ] 3.3 `PropKind::Gradient` in the property system, with validation
- [ ] 3.4 Color source: `style` and `gradient` settings; render once on update; old settings
      render the solid color unchanged (test)
- [ ] 3.5 `Canvas::background_paint` in the model (serde default `None`); `background` written as
      the solid color or first stop; renderer draws a cached gradient texture first

## 4. Color picker
- [ ] 4.1 Picker popup widget: hue ring image (Rust, once per size), saturation/brightness
      square, opacity slider on checkerboard, hex and RGB fields, preset swatches
- [ ] 4.2 Recent colors (last 12) in settings
- [ ] 4.3 Every `color` property, the canvas background and item fill open the picker; live
      edits coalesce; `History::seal` on close (test: one undo step per session)
- [ ] 4.4 Gradient editor: stop bar (click adds a stop with the color at that point, drag off or
      Delete removes), angle dial, radial center
- [ ] 4.5 Eyedropper from the Preview: crosshair and loupe; engine 1 × 1 readback into a reused
      staging buffer, reply on the event channel; no cost when not picking
- [ ] 4.6 Screen color picking trait (`pick_screen_color`); Windows: frozen screenshot in
      borderless topmost windows per monitor, loupe, click picks, Esc cancels
- [ ] 4.7 Hide "From screen" where the platform has no implementation yet (macOS
      `NSColorSampler` and the Linux portal `PickColor` come in their platform branches)

## 5. Fill around fitted items
- [ ] 5.1 `SceneItem::fill: ItemFill` (serde default `None`); `UpdateItem` patch field; Copy to
      canvas and Duplicate canvas copy it; tests
- [ ] 5.2 Render graph: fill drawn beneath the item inside its bounds box, with the item's
      opacity and blend mode, for Fit bounds only
- [ ] 5.3 Frosted: cover UV on the CPU; downscale into a pool texture (long side 1/8 of the box,
      64 to 480 px); two blur passes reusing the blur filter; composite with dim and saturation;
      textures reallocated only when the fill or bounds change
- [ ] 5.4 Color and Gradient fill using the paint textures from 3.2
- [ ] 5.5 Layout tab: "Fill empty space" panel (Frosted with Blur, Dim and Saturation sliders;
      Color; Gradient; None), shown when the bounds mode is Fit
- [ ] 5.6 Every item starts with None; existing projects unchanged (test loading an old
      collection)
- [ ] 5.7 Golden-image test: 16:9 source fitted in a 9:16 canvas with Frosted, Color and None
- [ ] 5.8 Benchmark: Frosted on a 1080 × 1920 at 60 fps canvas adds well under 0.5 ms GPU time

## 6. Finish
- [ ] 6.1 `cargo clippy --all-targets -- -D warnings` and tests for the touched crates
- [ ] 6.2 Manual check in the app: remove and undo Desktop Audio; vertical 60 fps canvas with a
      game and Frosted fill; Color source gradient; eyedropper from the Preview and the screen
