# Change: Add engine core (workspace, command bus, scene graph, GPU compositor, persistence)

## Why
Every eStream feature (sources, outputs, UI, remote API, plugins) depends on a solid engine:
a real-time frame clock, a scene model, a GPU compositor, and a single mutation path that enables
undo/redo and remote control. Getting this foundation right first avoids the architectural debt
that OBS carries (global mutable state, mutations that can't be undone, single canvas).

## What Changes
- Create the Cargo workspace and crate layout described in `project.md`.
- **engine-runtime**: engine thread model, monotonic engine clock, fixed-rate video frame clock,
  command bus (commands in, events/snapshots out), and structured logging and tracing.
- **scene-composition**: project model (canvases → scenes → scene items → sources), transforms,
  crop, bounds, blend modes, visibility, z-order, groups, and a wgpu compositor that renders each
  canvas to a GPU texture every frame, with a headless mode for tests.
- **project-persistence**: JSON scene collections with schema versioning and migrations, autosave
  and crash recovery, and an undo/redo history that covers every command.

## Impact
- Affected specs: `engine-runtime` (new), `scene-composition` (new), `project-persistence` (new)
- Affected code: new `crates/estream-core`, `crates/estream-gpu`, `crates/estream-engine`,
  `crates/estream-project`
- Unblocks: `add-studio-shell`, `add-video-sources`, `add-audio-engine`, `add-recording`
