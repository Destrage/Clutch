# Tasks: redesign-inspector

## 1. Widgets
- [ ] 1.1 `ValueSlider`: name, value, fill from the origin, drag without jumping, Shift for
      fine, click to type, arrow keys, accessible as a slider
- [ ] 1.2 `FoldPanel`: header with a chevron that folds the body; sub-panel style; optional
      switch and actions in the header
- [ ] 1.3 `Segmented`: a two-to-four way choice

## 2. Inspector
- [ ] 2.1 Header (kind, name, eye and lock) and the tab strip, hidden with one tab; the shown
      tab falls back to the first one the selection has
- [ ] 2.2 Source tab: settings (sliders for small ranges), page, playback
- [ ] 2.3 Layout tab: placement picture, position and size, rotation slider, crop (folded)
- [ ] 2.4 Look tab: opacity, blend, video filters as panels with sliders and switches
- [ ] 2.5 Sound tab: volume and balance, who hears it, voice buttons, audio filters as panels,
      Advanced (folded); the Live deck uses the same panels

## 3. State
- [ ] 3.1 `inspector-tab` and `inspector-folds` on the window; saved in `Settings.inspector`
      and applied on start

## 4. Check
- [ ] 4.1 Build, clippy; in-app check of each tab for a camera, a microphone, text, a browser
      source and a group; undo of a slider drag
