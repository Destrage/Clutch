# Design: Engine Core

## Context
eStream has to compose many video sources in real time at a fixed frame rate. It must stay
responsive to UI and remote edits and never stall because of I/O. The engine is the only owner of
mutable project state.

## Goals / Non-Goals
- Goals: deterministic frame pacing, lock-free UI ↔ engine communication, an undoable command
  model, a GPU-only composition path, and headless testability.
- Non-Goals: encoding/outputs (see `add-recording`), audio mixing (see `add-audio-engine`), and
  multi-GPU support.

## Decisions

### Threads
| Thread | Responsibility | Rules |
|---|---|---|
| Engine (control) | Owns `Project`; applies commands; emits events and snapshots | No blocking I/O |
| Video | Frame clock tick → source `tick()` → compositor render → hand textures to outputs | No alloc, no locks across frames |
| Audio | (later change) 10 ms blocks, mixer | Real-time priority |
| I/O pool | `tokio` runtime for network/file/remote API | Never touches GPU directly |

The video thread reads an immutable `Arc<RenderGraph>`, which the engine thread rebuilds and
swaps (`arc-swap`) whenever the project changes. This keeps rendering lock-free.

### Command bus
- `enum Command` (serde-serializable). Each variant implements `apply(&mut Project) -> Result<Inverse>`.
- Applying a command returns its inverse command, which is pushed onto the undo stack.
  Commands with the same coalescing key coalesce within 500 ms, so dragging an item counts as one undo step.
- `Transaction` groups commands atomically (all-or-nothing, single undo entry).
- After each applied batch the engine publishes `Event`s plus a new `Arc<ProjectSnapshot>`.
- Channels: `crossbeam-channel` (commands, bounded) and `tokio::sync::broadcast` (events to async consumers).

### Clock
- `EngineClock`: monotonic nanoseconds since engine start (`QueryPerformanceCounter` /
  `mach_absolute_time` / `CLOCK_MONOTONIC`). All media timestamps use this timebase.
- `FrameClock`: rational frame rate (e.g. 60000/1001). The frame for index `n` is due at
  `t0 + n * den/num`. Pacing uses a high-resolution waitable timer plus a short spin. If the
  engine misses a frame, the deadline doesn't drift; the frame is counted as "lagged" and skipped.

### Compositor
- wgpu. Each canvas renders into an `Rgba16Float` texture (linear light), so blending and HDR later
  are correct. A final pass converts to the output format (sRGB 8-bit for preview, NV12/P010 for encoders).
- Scene items are drawn as textured quads. Transform = position, rotation, scale, crop, bounds
  (fit/fill/stretch/none), and alignment. Blend modes are normal, additive, multiply and screen.
- A source that has no frame yet draws nothing. A source with a stale frame redraws its last frame.
- Texture pool keyed by (size, format) to avoid per-frame allocation.
- Headless mode: `wgpu` with no surface. Used for golden-image tests and CI.

### Persistence
- Scene collection = one JSON file `{ "schema": N, "canvases": [], "scenes": [], "sources": [] }`.
- Every stored object has a stable UUID v7 ID. References are by ID, never by name, so renaming is safe.
- Migrations are pure functions `vN → vN+1`, run in sequence on load.
- Autosave: debounced 2 s after the last change, writing an atomic temp file and then renaming it.
  A crash-recovery file is kept next to the collection.

## Alternatives Considered
- **Bevy ECS for the scene graph**: powerful but heavy; it imposes its scheduler and app model on us.
  A plain data model with an explicit render graph is simpler and easier to reason about for real time.
- **Locks around shared state**: rejected. It causes UI hitches and priority inversion
  on the render thread.
- **8-bit sRGB compositing**: rejected. It gives incorrect blending and blocks HDR.

## Risks / Trade-offs
- Rebuilding the render graph on every edit could be costly for huge scenes. Mitigation: incremental
  rebuild for transform-only changes (patch the item uniforms in place).
- A wgpu abstraction might block zero-copy interop with platform textures. Mitigation: use wgpu-hal
  interop (`create_texture_from_hal`) for D3D12/Metal/Vulkan shared handles, prototyped in this change.

## Open Questions
- Do we need a 10-bit/HDR canvas in phase 1? Proposed: keep the internal format HDR-ready, but expose only SDR for now.
