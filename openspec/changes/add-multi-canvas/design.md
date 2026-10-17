# Design: Multiple canvases

## Engine
Rendering already handles many canvases: each has its own frame clock, output ring, readback and
studio preview. The only single-canvas assumption is the audio graph. It now mixes global sources
plus the sources visible in the active scene of every canvas, and in the scene being left during
a transition. A source shown on two canvases is mixed once.

## Outputs (`estream-media`)
- `RecordingSettings.canvas: Option<CanvasId>` (`None` = first canvas) and `all_canvases: bool`.
  Recording all canvases starts one session per canvas. Every file after the first gets the
  canvas name as a suffix (`… (Vertical).mkv`). Stopping returns every path.
- The replay buffer follows `canvas` (one canvas).
- `StreamSettings.canvas: Option<CanvasId>`. `MultiStream` groups by `(canvas, encoder
  settings)`.
- A stored canvas id that no longer exists (e.g. the settings belong to a different collection)
  falls back to the first canvas. Settings are global; canvas ids belong to a collection.

## Studio
- `current_canvas` in the app replaces every "first canvas". It falls back to the first canvas
  when the selected one is removed or another collection is loaded.
- **Canvas tabs** above the preview: one tab per canvas (name and size), plus "+ Add canvas".
  - Picking a preset runs `AddCanvas` + `AddScene` + `SetActiveScene` as one undoable batch and
    selects the new canvas.
  - Double-click renames a tab (`UpdateCanvas`). The × removes a tab (`RemoveCanvas`, undoable;
    the last canvas can't be removed).
- **Settings:**
  - Recording gets a "Canvas" select (each canvas plus "All canvases").
  - Each stream destination gets a "Canvas" select.

## Testing
- **Graph:** the audio union across canvases, with a shared source mixed once.
- **Media:** recording all canvases writes one file per canvas at each canvas's own size.
  A destination on a vertical canvas sends vertical video. Destinations on different canvases
  use separate encoders.
- **App:** canvas selection fallback.
- **In-app:** add a vertical canvas, build a scene on it, switch tabs.
