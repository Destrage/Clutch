# Tasks: add-interface-zoom

## 1. Zoom
- [ ] 1.1 Spike: multiply the scale factor at runtime with the winit backend; keep it across OS scale changes
- [ ] 1.2 `ui_zoom` setting (lenient parsing), applied on start and on change
- [ ] 1.3 Levels that don't fit the monitor are disabled; step down on a smaller monitor, restore on return
- [ ] 1.4 Ctrl/Cmd + = / - / 0 and command palette actions, with a toast

## 2. Text
- [ ] 2.1 Largest (150%) text size; check fixed-height rows at 150% text × 200% zoom

## 3. UI
- [ ] 3.1 Settings → Appearance: Interface zoom row above Text size
- [ ] 3.2 First-run wizard Aa button with zoom and text size

## 4. Check
- [ ] 4.1 In-app check at each level on 1080p and 4K monitors, with 100% and 150% OS scaling
