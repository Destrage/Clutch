# Change: Brighter text and a text size setting

## Why
Testers find text hard to read. Secondary text (`text-muted`) and hint text (`text-faint`) are
used in about 240 places. Hint text measures 2.9–4.0:1 contrast on the dark theme's surfaces
and 2.6–3.1:1 on the light theme's, below the 4.5:1 that small text needs. The smallest size is
11px, and a few labels are hard-coded at 9–10px. There is no way to make text larger short of
changing Windows' display scaling for every app.

## What Changes
- **Brighter text tokens** in `theme.slint`. Every text color SHALL reach at least 4.5:1 on
  every surface of its theme (measured values below), and primary text gets a little brighter:

  | Token | Dark now → new | Light now → new |
  |---|---|---|
  | `text` | `#e8edf2` → `#f3f6f9` (14.4 → 15.7:1) | `#141a21` → `#0d1218` |
  | `text-muted` | `#93a1b0` → `#b4c0cc` (5.0–7.0 → 7.1–10.0:1) | `#4f5d6b` → `#3d4955` (5.6–6.8 → 7.7–9.2:1) |
  | `text-faint` | `#66768a` → `#8a9aac` (2.9–4.0 → 4.6–6.4:1) | `#8794a2` → `#5f6b79` (2.6–3.1 → ≥ 4.5:1) |

  The final hex values may move slightly during the in-app check. The 4.5:1 floor is the requirement.
- **Text size** setting on Settings → Appearance, next to Theme: **Small (90%)**, **Default**,
  **Large (115%)**, **Larger (130%)**. It applies at once without a restart and is saved with
  the other settings.
  - `Theme` gets a `text-scale` property; `font-xs` … `font-xl` are derived from it.
  - The nine hard-coded `font-size: Npx` values become tokens, so they scale too.
  - Rows and controls that hold text and have fixed heights grow with the text instead of
    clipping it (min-heights derived from the font tokens).
- The smallest token (`font-xs`) goes from 11px to 12px at Default.

## Out of scope
- Scaling icons, the preview or the whole window (Windows display scaling already does that).
- Chat message size in the chat overlay on stream (it has its own setting).

## Impact
- Affected specs: `studio-ui` (ADDED: Readable Text, Text Size Setting)
- Affected code: `apps/estream/ui/theme.slint`, the `.slint` files with literal sizes or
  fixed-height text rows, `apps/estream/src/settings.rs` (new `text_size` with lenient parsing),
  `apps/estream/src/app.rs` (apply on start and on change)
- Shared work: branch `improve-text-readability`
