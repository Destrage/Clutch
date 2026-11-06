# Tasks: unify-eyedropper

## 1. Picking
- [x] 1.1 `screen_color`: start/stop picking on Windows with `WH_MOUSE_LL` and `WH_KEYBOARD_LL`
      hooks on the UI thread; hook procedures only record pick, cancel and the point; 60 s limit
- [x] 1.2 App: one `picker_eyedropper` action; tick shows the color under the pointer, applies the
      pick, restores the start color on cancel, removes the hooks; closing the picker stops picking
- [x] 1.3 Platforms without screen picking: the button starts Preview picking (as before)

## 2. UI
- [x] 2.1 `color.slint`: one eyedropper button (toggled while picking), press-and-drag still
      picks on release; the screen button is removed; hint "Click anywhere to pick a color. Esc
      cancels."

## 3. Check
- [x] 3.1 Test for telling a click on the button from a drag; clippy and the app tests. The hooks
      only see real input, so picking itself is checked by hand (3.2)
- [ ] 3.2 By hand on Windows: pick from the Preview, from another app, from another monitor;
      Esc and right click cancel; the other app doesn't get the click
