# Tasks: add-interface-zoom

## 1. Zoom
- [x] 1.1 Prove runtime scale-factor override on Windows; runtime path chosen
- [ ] 1.1b Same check on Linux (Wayland + X11) and macOS
- [x] 1.2 Zoom setting (100–200%), applied at startup before the first frame and on change
- [x] 1.3 Re-apply after OS scale or monitor changes; step down when the level no longer fits
- [x] 1.4 Preview 100% and its zoom label in screen pixels

## 2. Text size
- [x] 2.1 Largest (150%); sidebar, layers panel and panel headers grow or elide (checked at Largest × 150%)
- [ ] 2.2 Check the deck and inspector at Largest × 200% on a 4K monitor

## 3. UI
- [x] 3.1 Appearance page: zoom choices; levels too large for the screen say so and are refused
- [x] 3.2 Ctrl+Shift+= / Ctrl+Shift+- / Ctrl+Shift+0 and command palette entries
- [x] 3.3 Compact top bar in narrow windows (layout-loop-free window width from the app)
- [x] 3.4 Dialogs taller than the window scroll, with a scroll indicator

## 4. Check
- [x] 4.1 Tests: settings round trip, fit rule per monitor size, steps, labels, text size fallback
- [x] 4.2 Build; in-app check on Windows at 100% and 150% on a 1080p monitor (wizard, studio, settings, shortcuts)
- [ ] 4.3 In-app check on a 4K monitor and moving the window between monitors
