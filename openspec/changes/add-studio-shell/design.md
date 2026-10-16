# Design: Studio Shell

## Context
The UI must look modern, stay responsive while the engine is under load, and display a live
preview with minimal latency.

## Decisions

### UI toolkit: Slint
- Declarative `.slint` markup with a live-preview editor (fast design iteration) and a GPU renderer.
- Preview integration: the engine renders the canvas to a wgpu texture. Slint (with its wgpu renderer)
  imports that texture as an `Image` without copying it to the CPU. The engine and UI share one
  `wgpu::Device`.
- Fallback: if texture sharing is unavailable on a backend, copy GPU→GPU into a Slint-owned texture
  (still no CPU readback).

**Alternatives considered**
- *Tauri/Electron + React*: the most familiar look and feel, but showing the preview needs frame
  copies to the webview or an overlay child window (z-order, DPI and resize issues), and we'd
  maintain two languages.
- *egui*: very fast to build with, but its immediate-mode look is hard to make feel polished for
  consumers.
- *iced*: a good option (MIT license, wgpu shader widget). Slint is preferred for its designer
  tooling and richer built-in widgets.

### View-model layer
- `apps/estream/src/vm/*` maps `ProjectSnapshot` → Slint models (`VecModel`). It diffs snapshots to
  update models incrementally.
- UI actions → `Command` → engine. The UI never mutates its own models directly; it waits for the
  snapshot (optimistic updates only for drag gestures, which settle when the next snapshot arrives).

### Design system
- Tokens: spacing (4 px grid), radius (6/10/14), type scale (Inter / Segoe UI Variable fallback),
  semantic colors (bg, surface, surface-2, border, text, text-muted, accent, live-red, warn, ok).
- Dark theme by default, plus light theme and a "follow system" option. Accent color is configurable.
- Component library: Button, IconButton, Toggle, Slider, NumberField (drag to scrub), Select,
  Panel, Tabs, ListRow, Toast, Meter, Tooltip, ContextMenu, CommandPalette.
- Icons: Lucide (ISC license) SVGs.

### Layout
```
┌──────────────────────────────────────────────────────────────────────┐
│ ● eStream   Scene Collection ▾        CPU 3% · 60 fps   [Record] [Go Live] │
├────────┬──────────────────────────────────────────┬──────────────────┤
│ Scenes │                                          │ Inspector        │
│  Main  │              Live Preview                │  Transform       │
│  BRB   │       (on-canvas handles, guides)        │  Crop            │
│  Chat  │                                          │  Filters         │
│  +     │                                          │  Source props    │
├────────┴───────────────────────┬──────────────────┴──────────────────┤
│ Sources (layers)               │ Audio Mixer  ▮▮▮▮▮▯▯  ▮▮▮▯▯▯▯          │
└────────────────────────────────┴─────────────────────────────────────┘
```

## Risks / Trade-offs
- Slint's wgpu interop API may be marked unstable. Mitigation: pin the Slint version and wrap the
  interop in a single module (`preview/texture_bridge.rs`).
- Docking is not built into Slint. Mitigation: implement a simple split-pane docking model
  ourselves (no floating windows in phase 1).
