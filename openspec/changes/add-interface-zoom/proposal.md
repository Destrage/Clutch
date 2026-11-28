# Change: Interface zoom and a larger text size

## Why
Testers with high-resolution monitors, TVs across the room, or low vision say the studio is
too small to read and use. The 0.12.0 Text size setting helps with text, but icons, buttons,
knobs, the deck and panel spacing stay the same size, and Larger (130%) isn't enough for some
people. Windows display scaling enlarges every app, which people don't want just for Clutch.

## What Changes
- **Interface zoom** (studio-ui) on Settings → Appearance: 100%, 125%, 150%, 175% and 200%.
  It scales the whole studio window (text, icons, controls, spacing, popups and dialogs) on
  top of the OS display scaling, applies right away, and is remembered.
- **Keyboard:** Ctrl+Shift+= zooms in, Ctrl+Shift+- zooms out, Ctrl+Shift+0 resets (Ctrl+0 and
  Ctrl+1 stay the preview's Fit and 100%). The command palette has the same three commands.
- **Fits the screen:** a zoom level that would make the window's logical size smaller than
  the studio's minimum layout on the current monitor is shown disabled with the reason. When the
  window moves to a smaller monitor, the zoom steps down and a toast says so.
- **Text size** gets **Largest (150%)**. Text size and zoom multiply, so 150% zoom with
  Large text draws text at about 172%.
- The preview, Program view and pop-out windows keep showing the canvas at its real
  pixels. Zoom changes the controls around the canvas, not the video.

## Impact
- Affected specs: studio-ui (Interface Zoom added; Text Size Setting modified).
- Affected code: `apps/estream/src/settings.rs` (zoom setting, new text size),
  `apps/estream/src/app.rs` (applying the scale factor, monitor changes, shortcuts),
  `apps/estream/src/actions.rs` (zoom actions), `apps/estream/ui/overlays.slint` (Appearance page).
