# Tasks: add-studio-shell

## 1. App scaffold
- [ ] 1.1 Create `apps/estream` binary crate with Slint (wgpu renderer feature), `build.rs` compiling `.slint`
- [ ] 1.2 Start engine on launch; shared wgpu device between engine and UI
- [ ] 1.3 Single-instance guard, window state persistence (size, position, maximized, monitor)

## 2. Design system
- [ ] 2.1 Theme tokens (dark/light/system, accent color)
- [ ] 2.2 Core components (Button, IconButton, Toggle, Slider, NumberField, Select, Tabs, Panel, ListRow, Tooltip, ContextMenu, Toast, Meter)
- [ ] 2.3 Lucide icon set integration

## 3. Layout & panels
- [ ] 3.1 Top bar (collection switcher, stats, Record / Go Live buttons, disabled until outputs exist)
- [ ] 3.2 Scenes rail (add, rename inline, duplicate, delete, reorder by drag)
- [ ] 3.3 Sources/layers list (visibility, lock, reorder, groups, context menu)
- [ ] 3.4 Inspector (transform, crop, bounds, blend, opacity, source properties)
- [ ] 3.5 Split-pane docking with saved layouts + "Reset layout"

## 4. Preview
- [ ] 4.1 Texture bridge: engine canvas texture → Slint `Image` (zero-copy), GPU-copy fallback
- [ ] 4.2 Preview scaling (fit, 100%, zoom/pan with wheel + space-drag)
- [ ] 4.3 Selection + transform handles (move/resize/rotate), Alt-drag crop, Shift for aspect lock
- [ ] 4.4 Snapping (canvas edges/center, other items) and smart guides; toggle in settings
- [ ] 4.5 Keyboard nudge (1 px / Shift 10 px), Delete, Ctrl+D duplicate, Ctrl+C/V copy-paste items

## 5. Productivity
- [ ] 5.1 Command registry (id, title, shortcut, predicate) shared by menus, palette, hotkeys
- [ ] 5.2 Command palette (Ctrl+K) with fuzzy search
- [ ] 5.3 "Add source" picker with search and categories
- [ ] 5.4 Undo/redo wiring (Ctrl+Z / Ctrl+Shift+Z) + Edit menu history

## 6. Onboarding & feedback
- [ ] 6.1 First-run wizard (GPU/encoder detection, canvas preset)
- [ ] 6.2 Toast notifications + status bar stats (CPU, GPU, render time, FPS, lagged frames)

## 7. Accessibility & verification
- [ ] 7.1 Accessible names/roles on all interactive elements; focus order audit
- [ ] 7.2 UI responsiveness test: engine under synthetic load, input latency < 50 ms
- [ ] 7.3 Preview latency measurement (engine frame timestamp → present) ≤ 2 frames
