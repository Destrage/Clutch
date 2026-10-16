# Change: Add studio shell (modern desktop UI with live preview)

## Why
OBS's UI is functional but dated: dense dock clutter, modal property dialogs, no command palette,
no undo, and limited on-canvas editing. eStream's main differentiator for most users is a clean,
fast, modern interface built on top of the engine core.

## What Changes
- **studio-ui** capability (new):
  - Slint-based desktop app (`apps/estream`) with a dark/light theme design system.
  - Default layout: top bar (Go Live / Record / status), left Scenes rail, center live preview,
    right Inspector (properties of the selection), bottom Sources + Audio mixer strip.
  - Panels are dockable, resizable and collapsible. Layouts are saved and can be reset.
  - Live GPU preview with zero-copy display of the compositor texture.
  - On-canvas editing: select, move, resize, rotate, crop (Alt-drag), snapping to edges, centers and
    other items, smart guides, and nudging with arrow keys.
  - Inline inspector (no modal dialogs) with instant apply. Every change is undoable.
  - Command palette (Ctrl+K) for every action, plus a searchable "Add source" picker.
  - Keyboard-first: shortcuts for all common actions, full focus navigation, screen-reader labels.
  - First-run wizard: detect GPU and encoders, then pick a canvas preset (1080p60, 1440p60, vertical…).
  - Toast notifications and a non-blocking status bar (CPU, GPU, FPS, dropped frames).

## Impact
- Affected specs: `studio-ui` (new)
- Affected code: new `apps/estream` (Slint UI, `.slint` components, view-models)
- Depends on: `add-engine-core`
