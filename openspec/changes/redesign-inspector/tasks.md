# Tasks: redesign-inspector

## 1. Widgets
- [x] 1.1 `ValueSlider`: name, value, fill from the origin, drag without jumping, Shift for
      fine, click to type, arrow keys, accessible as a slider
- [x] 1.2 `FoldPanel`: header with a chevron that folds the body; sub-panel style; optional
      switch and actions in the header
- [x] 1.3 `Segmented`: a two-to-four way choice

## 2. Inspector
- [x] 2.1 Header (kind, name, eye and lock) and the tab strip, hidden with one tab; the shown
      tab falls back to Sound for sounds, else the first one the selection has
- [x] 2.2 Source tab: settings (sliders for small ranges), page, playback
- [x] 2.3 Layout tab: placement picture, position and size, rotation slider, crop (folded)
- [x] 2.4 Look tab: opacity, blend, video filters as panels with sliders and switches
- [x] 2.5 Sound tab: volume and balance, who hears it, voice buttons, audio filters as panels,
      Advanced (folded); the Live deck uses the same panels

## 3. State
- [x] 3.1 `inspector-tab` and `inspector-folds` on the window; saved in `Settings.inspector`
      and applied on start

## 4. Check
- [x] 4.1 Build, clippy, fold-name round-trip test; in-app check on a test setup (text with
      video filters, chat overlay, test tone with audio filters): tabs, slider drag and one-step
      undo, click to type, folding, folds kept after a restart
- [x] 4.2 In-app check by the user with real devices: camera, microphone, browser source, a
      group, and the Live deck's sound panel
