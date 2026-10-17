# Change: Multiple canvases

## Why
Creators publish the same show horizontally (Twitch, YouTube) and vertically (Shorts, TikTok,
Reels). OBS needs a plugin and a second scene collection for that. The eStream engine already
renders any number of canvases, each at its own size and frame rate. This change exposes that in
the studio and lets every output choose its canvas.

## What Changes
- **multi-canvas** capability (new):
  - Canvas tabs above the preview. Add a canvas from presets (vertical 1080×1920, square,
    horizontal), rename it, or remove it (undoable).
  - The selected canvas drives the scene list, the preview, the inspector, studio mode and
    transitions.
  - Sources are shared, so a camera or overlay can appear in scenes on several canvases without
    being opened twice.
  - **Audio:** the mix follows the active scenes of all canvases (each source mixed once).
- **recording** (modified): recordings and the replay buffer choose a canvas, or record every
  canvas at once (one file per canvas, named after it).
- **streaming** (modified): each destination chooses its canvas. Destinations share an encoder
  only if they also share the canvas.

## Impact
- Affected specs: `multi-canvas` (new), `recording`, `streaming` (modified)
- Affected code:
  - `estream-engine` (audio graph across canvases);
  - `estream-media` (canvas choice for sessions, recordings, streams);
  - `apps/estream` (canvas tabs, current canvas, settings).
