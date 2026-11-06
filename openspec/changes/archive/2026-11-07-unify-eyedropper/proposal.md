# Change: One eyedropper that picks from anywhere

## Why
The color picker has two eyedroppers: a pipette that picks from the Preview with a click, and a
screen button that picks from anywhere, but only by pressing it and dragging. A user who tried
both asked for one: a single eyedropper that picks from any app. Two buttons that do almost the
same thing, with two different gestures, are confusing.

## What Changes
- **One eyedropper button.** Click it, then click anywhere: the Preview, another part of Clutch,
  another app, another monitor. The color under the pointer is taken and the picker stays open.
- **Live while picking.** Until the click, the picker's color follows the pointer, and the
  Preview keeps its magnifier when the pointer is over it.
- **The click goes nowhere else.** The click that picks doesn't reach the app under the pointer.
- **Cancel** with Esc or a right click; the color goes back to what it was before picking.
- **Drag still works.** Pressing the button and releasing over a color picks it too.
- The separate screen button is removed.
- On macOS and Linux, until their platform branches add screen picking, the same button picks
  from the Preview only, as today.

## Impact
- Affected spec: `studio-ui` (Color picker).
- Affected code: `apps/estream/ui/color.slint` (one button, picking hint),
  `apps/estream/src/screen_color.rs` (Windows: a low-level mouse and keyboard hook while picking),
  `apps/estream/src/app.rs` (picking state, cancel restores the color).
