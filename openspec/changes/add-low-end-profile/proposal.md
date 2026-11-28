# Change: A performance profile for low-end PCs

## Why
`add-performance-profiles` offers Gaming, Balanced and Quality, but even Gaming keeps the
canvas resolution and frame rate for every output and assumes a hardware encoder. On a laptop
with an old integrated GPU, 4 cores or 8 GB of memory, a 1080p60 encode next to a game still
costs too much. Clutch also has no way to encode at a lower size than the canvas, so the only
fix today is rebuilding the canvas.

## What Changes
- **Output size and frame rate per output (new):** recording, replay buffer and each stream
  destination can encode at **Same as canvas**, **720p** or **480p**, and at the canvas fps or
  **half of it** (60 → 30). Scaling happens on the GPU in the existing color-conversion pass,
  and half fps drops every other composited frame before the encoder, so neither costs CPU.
  The canvas, scenes and preview stay as they are.
- **Low-end PC** profile, lighter than Gaming:
  - 720p at half fps (30 from a 60 fps canvas) for streams and recordings;
  - the hardware encoder's fastest preset, or x264 `superfast` without one;
  - stream bitrate from the platform's 720p30 recommendation, capped by the measured upload;
  - a 30 s clip-mode replay buffer;
  - the preview drawn at 15 fps while live or recording (outputs are unaffected).
  The profile's scope is the same as the others (encoder settings, output size/fps, buffer,
  preview rate). It never changes canvases, scenes or sources.
- **Suggestions it doesn't apply on its own:** on choosing Low-end PC, the confirmation also
  lists heavy things in the current setup, each with a checkbox to switch it (off by default):
  strong noise suppression → standard, background removal off, browser sources capped at
  30 fps.
- **Optimize for this PC** recommends Low-end PC when there is no hardware encoder and 4 cores
  or fewer, or when memory is under 8 GB, or when the only GPU is integrated and older than the
  current generation's media engine (no HEVC encode). Other weak cases keep Gaming.
- **Use a lighter profile** on overload alerts can now step from Gaming to Low-end PC.
- **Testing without a weak PC (developer):** `CLUTCH_SIMULATE_HW` (for example
  `cores=2,memory=4,no-hw-encoders,igpu`) makes the hardware probe report those values and
  hides hardware encoders, so recommendations, the software path and the UI can be checked on
  any machine. `tools/low-end-sim.ps1` starts Clutch inside a Windows job object that limits
  CPU rate, cores and memory, to measure real behaviour under constrained resources.

## Out of scope
- Lowering the canvas resolution itself (users can still do that in canvas settings).
- Automatic switching to Low-end PC without the user confirming.

## Impact
- Affected specs: `performance` (ADDED: Output Scaling, Low-End PC Profile, Simulated Hardware).
- Affected code: `crates/estream-gpu` (scaled conversion target), `crates/estream-media`
  (output size/fps in recording and stream settings), `apps/estream/src/performance.rs`
  (profile, recommendation rules, simulation), `apps/estream/ui/overlays.slint` (profile card,
  output size/fps rows), `tools/low-end-sim.ps1`.
- Builds on `add-performance-profiles` (its in-app check, task 4.1, is still open).
- Shared work: branch `add-low-end-profile`.
