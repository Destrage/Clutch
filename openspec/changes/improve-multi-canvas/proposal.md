# Change: Clear canvas outputs, and copying canvases and scenes

## Why
A user added a 2560 × 1440 canvas, built it out, went live and streamed the 1920 × 1080 one.
Each destination picks its canvas in Settings, and a destination that never picked one streams
the first canvas. The canvas tabs only choose what is edited, but nothing on screen says which
canvas goes out, so the selected tab looks like the one that will stream.

Making a second canvas also means rebuilding every scene by hand: a new canvas starts with one
empty scene, and there is no way to duplicate a canvas or copy a scene to another canvas. The
user wanted the Full HD setup again at 1440p or 4K with the same items, scaled to fit.

## What Changes
- **Each canvas tab says what it feeds:** a tag after its size, e.g. "Stream · Record",
  listing the outputs that use it (stream destinations, recording, replay buffer, virtual
  camera). It turns red while that canvas is live. Tags only show when there is more than one
  canvas.
- **A menu on each canvas tab** (a chevron on the tab), with:
  - **Stream and record this canvas:** points every destination, recording and the replay
    buffer at this canvas. Outputs that are running keep their canvas until they restart, and
    Clutch says so.
  - **Duplicate:** at the same size, or at any canvas preset (1080p, 1440p, 4K, 720p,
    vertical). The copy gets every scene of the canvas with the same sources and items,
    scaled to the new size.
  - **Rename** and **Remove**.
- **Copy a scene to another canvas:** the scene card's hover actions gain "Copy to canvas"
  when the setup has more than one canvas. It lists the other canvases; the copy is scaled the
  same way.
- **Scaling rule:** items keep their place relative to the canvas. With the same aspect ratio
  everything scales evenly. With a different one, the layout scales to fit and is centered;
  items that filled the old canvas fill the new one.
- **Go live names the canvas:** with more than one canvas, the Go live window has a "Canvas"
  choice for all destinations, and the "Going live" message names the canvas that went out,
  also when the window is skipped.

## Impact
- Affected specs: multi-canvas.
- Affected code: `apps/estream/src/interact.rs` (scaling), `apps/estream/src/app.rs` (tab
  tags, menu actions, scene copy, Go live canvas), `apps/estream/ui/panels.slint`
  (`CanvasTabs`), `apps/estream/ui/stage.slint` (`SceneCard`), `apps/estream/ui/golive.slint`,
  `apps/estream/ui/types.slint`, `apps/estream/ui/app.slint`.
- Every change to the project is a command batch, so Duplicate and Copy undo in one step. No
  settings change meaning; nothing is migrated.
