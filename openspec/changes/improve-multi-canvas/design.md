# Design: Clear canvas outputs, and copying canvases and scenes

## Scaling (`interact::rescale`)
`rescale(t, from, to)` maps a top-level item's transform from a canvas of size `from` to one of
size `to`:
- `s = min(to.x / from.x, to.y / from.y)`, `offset = (to - from * s) / 2`.
- `position = position * s + offset`; with a bounds mode `bounds *= s`, otherwise
  `scale *= s`.
- An item with a bounds mode whose box covers the old canvas (position 0, bounds = `from`,
  within half a pixel) gets position 0 and bounds = `to`, so a full-screen game or camera keeps
  filling the canvas whatever its shape.
Only top-level items are rescaled. Group children live in the group's space and follow it.
Crop is in source pixels and stays.

## Copies
- **Duplicate canvas** is one batch: `AddCanvas` (the copy right after the original: name,
  background and fps; size from the choice), then `AddScene` for every scene of the canvas in
  order, each with fresh item ids (`fresh_ids`) and rescaled items, then `SetActiveScene` to
  the copy of the original's active scene. The copy is selected. Name: "<name> 2560 × 1440"
  for another size, "<name> copy" for the same size, made unique.
- **Copy scene to canvas** is one `AddScene` on the target canvas, named after the scene
  (unique within that canvas). The studio switches to the target canvas so the copy is seen.
- Sources are shared, not duplicated: a camera or game capture is one device. The audio graph
  already mixes a source shown on two canvases once.

## Output tags
`refresh_canvases` builds each tab's tag from settings:
- "Stream": an enabled destination resolves to the canvas (stored id, else first canvas).
- "Record": recording resolves to it, or records all canvases.
- "Replay": the replay buffer is running on it (it follows the recording canvas).
- "Camera": the virtual camera canvas resolves to it.
`CanvasTab.live` is true while a running destination streams that canvas; the tag turns red.
Tags are empty with one canvas.

## Tab menu
A chevron `IconButton` on each tab opens a `PopupWindow`, styled like the setup menu. Rows:
"Stream and record this canvas" (shown with a check, and disabled, when every output already
uses it), "Duplicate", one "Duplicate as <preset>" per canvas preset (size and frame rate), "Rename…",
"Remove" (danger; disabled with one canvas). The tab's old × button moves into this menu.
Rows call `canvas-menu(id, action, preset)`. The tabs are only replaced when they change, so an
open menu stays open while the stream status refreshes them.

"Stream and record this canvas" sets `destinations[*].stream.canvas`, `recording.canvas`
(clearing `all_canvases`) and saves. If something is running, a toast says it applies when
that output starts again.

## Go live
- The Go live window gets a "Canvas" select above the sections when there is more than one
  canvas. Its value is the canvas the enabled destinations stream, or none, with a
  hint, when they differ. Picking one sets every destination.
- With more than one canvas, the "Going live on …" toast adds the canvas that went out:
  "Canvas: “Full HD” (1920 × 1080)". It also shows when the Go live window is skipped.

## Testing
- Unit: `rescale` for the same aspect (exact scale), a different aspect (centered, full items
  fill), scale-mode items, and a round trip.
- In-app: duplicate Full HD as 1440p, check that items line up; copy a scene to a vertical
  canvas; use the tab menu to stream the 1440p canvas; tags change; go live and check the
  ingest resolution.
