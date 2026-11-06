# Design: unify-eyedropper

## Picking clicks outside Clutch (Windows)
A window only gets mouse input from outside itself while a button pressed on it is held, which is
why the screen eyedropper needed a drag. To take the *next click anywhere*, Clutch installs a
low-level mouse hook (`SetWindowsHookExW(WH_MOUSE_LL)`) and a low-level keyboard hook
(`WH_KEYBOARD_LL`) on the UI thread, which already runs a message loop:

- Installed when picking starts, removed when it ends (pick, cancel, picker closed, or 60 s
  without a click). Nothing is hooked otherwise.
- Mouse hook: left button down → remember the point and swallow it; left button up → swallow
  it and take the color there. Right button down → cancel (swallowed). Everything else passes
  on unchanged, so the pointer moves and wheels as usual.
- Keyboard hook: Esc → cancel (swallowed); other keys pass.
- The hook procedures only store the event in atomics and return; the app's tick reads them,
  reads the pixel (`GetPixel` on the screen DC, physical coordinates), applies or cancels, and
  removes the hooks.
- While picking, the tick also shows the color under the pointer in the picker every 30 ms
  (the existing `screen_color::under_pointer`).

Clicks on Clutch itself go through the same path, so picking from the Preview on Windows reads
the screen like any other app: the color as shown.

The press-and-drag gesture keeps working: pressing the button starts picking, and the release
(seen by the hook) picks.

## Other platforms
Without screen picking (`screen_color::AVAILABLE` false), the button turns on Preview picking as
today: the Preview takes the next click, with its magnifier, and Esc cancels.

## Cancel
Picking remembers the color it started from; cancel sets it back (one more change in the same
held undo run, so the picker session is still one undo step).
