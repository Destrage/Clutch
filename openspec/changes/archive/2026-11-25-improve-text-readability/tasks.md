## 1. Contrast
- [x] 1.1 New `text`, `text-muted`, `text-faint` values for both themes
- [x] 1.2 Unit test that computes contrast of every text token against every surface token (both themes) and fails below 4.5:1

## 2. Text size
- [x] 2.1 `Theme.text-scale`; derive `font-xs` … `font-xl` from it; `font-xs` 12px at Default
- [x] 2.2 Replace the hard-coded `font-size` values with tokens
- [x] 2.3 `text_size` setting (Small, Default, Large, Larger), lenient parsing, default Default; applied on start and on change
- [x] 2.4 Text size selector on Settings → Appearance under Theme

## 3. Layout at Larger
- [x] 3.1 Audit fixed-height rows that hold text (deck, sidebar, inspector, mixer, settings, wizard, Go Live box) and derive their heights from the font tokens
- [x] 3.2 In-app check of every main view and dialog at Small and Larger, both themes, with screenshots

## 4. Verify
- [x] 4.1 `cargo clippy` and tests for `estream`
