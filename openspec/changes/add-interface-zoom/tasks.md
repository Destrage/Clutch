# Tasks: add-interface-zoom

## 1. Zoom
- [ ] 1.1 Prove runtime scale-factor override on Windows, Linux (Wayland + X11) and macOS; pick runtime or restart path
- [ ] 1.2 Zoom setting (100–200%), applied at startup before the first frame and on change
- [ ] 1.3 Re-apply after OS scale or monitor changes; step down when the level no longer fits
- [ ] 1.4 Preview Fit/100% computed in physical pixels

## 2. Text size
- [ ] 2.1 Largest (150%); check deck, sidebar, inspector and settings at Largest × 200%

## 3. UI
- [ ] 3.1 Appearance page: zoom choices with disabled levels and their reason
- [ ] 3.2 Ctrl+Shift+= / Ctrl+Shift+- / Ctrl+Shift+0 and command palette entries

## 4. Check
- [ ] 4.1 Tests: settings round trip, fit rule per monitor size, text size fallback
- [ ] 4.2 Build; in-app check at each zoom on a 1080p and a 4K monitor, and moving between them
