# Change: Interface zoom and larger text sizes

## Why
`improve-text-readability` added a Text size setting (up to 130%) but scales text only: icons,
buttons, panels and spacing stay small. Testers with high-resolution screens, or who need
larger UI, still find the app hard to see, and the only fix today is raising Windows display
scaling for every app.

## What Changes
- **Interface zoom** on Settings → Appearance, above Text size: **90%, 100%, 110%, 125%, 150%,
  175%, 200%**. It scales the whole interface (text, icons, controls, spacing). It applies at
  once without a restart, is saved with the settings, and stacks with the OS display scaling.
  The preview and program pictures stay sharp because they are rendered at the window's real
  pixel size.
- **Shortcuts:** Ctrl + `=` / Ctrl + `-` step the zoom, Ctrl + `0` resets it (Cmd on macOS).
  The same actions are in the command palette. A short toast shows the new level.
- **Fits the screen:** a zoom level whose layout would not fit the current monitor (the window's
  minimum size, 980 × 620 at 100%, times the zoom, larger than the work area) is greyed out with
  "Too large for this screen". When the window moves to a smaller monitor, the zoom steps down
  for that monitor and returns when it moves back.
- **Text size gets one more step:** **Largest (150%)**. Text size and zoom multiply.
- **First run:** the first-run wizard has a small **Aa** button in its footer that opens zoom
  and text size, so someone who can't read the UI can fix it before anything else.

## Out of scope
- Per-panel zoom, and zoom for the on-stream chat overlay (it has its own size).

## Impact
- Affected specs: `studio-ui` (ADDED: Interface Zoom; MODIFIED: Text Size Setting).
- Affected code: `apps/estream/src/app.rs` (apply zoom by multiplying the window's scale
  factor through Slint's `WindowEvent::ScaleFactorChanged`, re-applied when the OS scale
  changes), `apps/estream/src/settings.rs` (`ui_zoom`), `apps/estream/ui/overlays.slint`
  (Appearance rows, wizard Aa button), `apps/estream/ui/theme.slint` (150% text step).
- Shared work: branch `add-interface-zoom`.
