# Change: Setups you can see and switch

## Why
The top bar shows the scene collection as a grey "Untitled" pill with a folder icon and the
canvas size. It isn't clickable and doesn't say what it is, and there is no way to have more than
one collection. The top bar also still shows the old logo.

## What Changes
- **studio-ui:**
  - The collection is called a **setup** in the UI (scenes, sources and audio together).
  - The top-bar pill becomes a menu: switch between setups, plus **New setup…** (runs the
    first-run setup, can be cancelled), **Rename…**, **Duplicate** and **Delete…** (asks first;
    not offered for the last setup).
  - Switching is blocked while streaming or recording.
  - The canvas shows as a short tag ("1080p60"), with the full size in the tooltip.
  - The first setup is called "My setup"; an existing default "Untitled" setup is renamed once.
  - The top bar, the window icon and the first-run dialog show the Clutch logo.

## Impact
- Affected specs: studio-ui.
- Affected code: `apps/estream` (app.rs, settings.rs, actions.rs, ui/panels.slint,
  ui/overlays.slint, ui/app.slint, ui/icons).
