# Change: Canvas fill, gradients and a real color picker

## Why
A user setting up a vertical stream ran into five gaps:

- **Desktop Audio can't be removed.** The setups add Desktop Audio and Microphone as sources
  heard in all scenes. They belong to no scene, so the scene's source list doesn't show them,
  and the mixer and the sound settings can only mute them. A user who doesn't want Desktop
  Audio has no way to get rid of it.
- **Colors are typed, not picked.** Every color setting is a hex box and eight fixed swatches.
  There is no wheel, no way to set transparency except by typing `#rrggbbaa`, and no way to
  match a color that is on screen or in the game.
- **The Color source is solid only.** Text can fade between two colors, but a Color source,
  the usual background, can't be a gradient.
- **Vertical canvases show black bars.** A 16:9 game fitted into a 9:16 canvas covers about a
  third of it. The rest is the canvas background, which is black and has no setting in the UI.
  The common fix is to put a blurred, enlarged copy of the game behind it, which today takes a
  second capture source plus a blur filter, and costs a second capture.
- **Vertical is 1080 × 1920 at 30 fps only.** The canvas presets have one vertical entry and
  4K only at 30 fps landscape. A canvas's size and frame rate can't be changed after it is
  made, although the engine's `UpdateCanvas` command already supports it.

## What Changes
- **Remove sounds from the mixer.** Each mixer strip and the sound settings panel get a Remove
  action. For a source heard in all scenes it removes the source. For a source placed in the
  current scene it removes it from that scene, the same as deleting it in the source list.
  Both are undoable. The mixer's existing "Add audio source" brings Desktop Audio back.
- **Color picker.** Clicking any color swatch in the inspector opens a picker with a hue ring,
  a saturation and brightness square, an opacity slider, hex and RGB fields, the preset
  swatches, the last 12 picked colors, and an eyedropper. The eyedropper picks from the Preview
  or from anywhere on screen. Changes show live while dragging; one picker session is one undo
  step. The same picker is used by every color setting (sources, filters, canvas background,
  text, item fill).
- **Gradients.** A new paint type: solid, linear (angle) or radial (center), with 2 to 8 color
  stops. The Color source gets a Style choice (Solid, Linear gradient, Radial gradient) and a
  gradient editor. Old projects keep their solid color.
- **Fill around fitted items.** A scene item that fits its source inside its bounds gets a
  "Fill empty space" setting in the Layout tab: Frosted (a blurred, enlarged and slightly
  darkened copy of the same source), Color, Gradient, or None. It fills the item's bounds box
  behind the source and needs no second capture. New Game, Window and Display capture and
  camera items, which are fitted to the canvas when added, start with Frosted. Existing items
  keep None.
- **Canvas background.** Each canvas's background becomes editable: a solid color or a gradient.
- **Canvas settings.** The canvas tab menu gets "Canvas settings…": name, size (presets or a
  custom width × height), frame rate and background. Size and frame rate are locked while a
  stream, recording or replay buffer uses the canvas. Changing the size can scale the
  canvas's items to match (on by default).
- **More presets.** Vertical 1080 × 1920 at 60 fps, 1440 × 2560 (2K) at 60 fps and
  2160 × 3840 (4K) at 30 and 60 fps, plus landscape 4K at 60 fps.

## Impact
- Affected specs: `studio-ui` (color picker, removing sounds from the mixer), `video-sources`
  (Color source gradients), `multi-canvas` (presets and canvas settings, canvas background,
  fill around fitted items).
- Affected code:
  - `estream-core`: `Paint` and `GradientStop` types.
  - `estream-project`: `SceneItem::fill`, `Canvas::background_paint` (serde defaults).
  - `estream-engine`: gradient property kind, Color source gradient rendering, canvas patch for
    the background, frosted-fill passes in the render graph, Preview pixel readback for the
    eyedropper.
  - `estream-gpu`: gradient shader, frosted downscale + blur (reusing the blur filter's
    passes) and a dim/saturation composite.
  - `apps/estream`: color picker and gradient editor widgets, Layout tab fill panel, canvas
    settings dialog, mixer Remove, presets, Windows screen color picking.
  - `estream-capture` (platform): screen color picking on Windows; macOS and Linux follow in
    their platform branches.
- No migration. New fields default to today's behavior, and older versions ignore them.
