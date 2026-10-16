# Tasks: add-studio-shell

## 1. App scaffold
- [x] 1.1 Create `apps/estream` binary crate with Slint (wgpu renderer feature), `build.rs` compiling `.slint`
- [x] 1.2 Start engine on launch; shared wgpu device between engine and UI
- [x] 1.3 Single-instance guard, window state persistence (size, position, maximized)

## 2. Design system
- [x] 2.1 Theme tokens (dark/light/system, accent color)
- [ ] 2.2 Core components: Button, IconButton, Toggle, Slider, NumberField (scrub), Select, Panel, Badge, Tooltip, Toast, EmptyState done; ContextMenu and Meter pending
- [x] 2.3 Lucide icon set integration

## 3. Layout & panels
- [x] 3.1 Top bar (collection name, canvas info, undo/redo, palette launcher, theme, Record / Go Live disabled until outputs exist)
- [ ] 3.2 Scenes rail: add, inline rename (double-click / F2), duplicate, delete done; drag-to-reorder pending
- [ ] 3.3 Sources/layers list: visibility, lock, groups, move up/down done; drag-to-reorder and context menu pending
- [x] 3.4 Inspector (transform, size mode, crop, opacity, blend, source size, color)
- [ ] 3.5 Resizable split panes with saved layout + "Reset layout" done; collapsible panels pending

## 4. Preview
- [x] 4.1 Texture bridge: engine canvas texture → Slint `Image` (zero-copy; preview stored as `Rgba8Unorm` written via an sRGB view)
- [x] 4.2 Preview scaling (fit, 100%, Ctrl+wheel zoom around cursor, Space/middle-drag pan)
- [x] 4.3 Selection + transform handles (move/resize/rotate), Alt-drag crop, Shift for aspect lock / 15° rotation steps
- [x] 4.4 Snapping (canvas edges/center, other items) with smart guides; Ctrl bypasses; toggle in palette
- [x] 4.5 Keyboard nudge (1 px / Shift 10 px), Delete, Ctrl+D duplicate, Ctrl+C/V copy-paste items

## 5. Productivity
- [x] 5.1 Command registry (id, title, shortcut, category) shared by palette and hotkeys
- [x] 5.2 Command palette (Ctrl+K) with fuzzy search over actions, scenes and sources
- [x] 5.3 "Add source" picker with descriptions (unreleased types shown as "Soon")
- [x] 5.4 Undo/redo wiring (Ctrl+Z / Ctrl+Shift+Z / Ctrl+Y) + top bar buttons

## 6. Onboarding & feedback
- [x] 6.1 First-run wizard (GPU detection, vendor-based encoder hint, canvas preset); real encoder probing lands with add-recording
- [ ] 6.2 Toasts + status bar (FPS, render time, lagged frames, GPU name) done; CPU and GPU usage pending

## 7. Accessibility & verification
- [ ] 7.1 Accessible names/roles on interactive elements done; screen-reader pass and focus-order audit pending
- [ ] 7.2 UI responsiveness test: engine under synthetic load, input latency < 50 ms
- [ ] 7.3 Preview latency measurement (engine frame timestamp → present) ≤ 2 frames
