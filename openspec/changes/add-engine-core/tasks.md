# Tasks: add-engine-core

## 1. Workspace
- [x] 1.1 Create Cargo workspace (`Cargo.toml` with `[workspace]`, edition 2024, shared `[workspace.dependencies]`)
- [x] 1.2 Create crates `estream-core`, `estream-gpu`, `estream-engine`, `estream-project`
- [x] 1.3 Add `rustfmt.toml`, `clippy.toml`, `.gitignore`, `rust-toolchain.toml`
- [x] 1.4 CI workflow: fmt, clippy `-D warnings`, test, `openspec validate --strict` (written; not yet run on GitHub)

## 2. Core types (`estream-core`)
- [x] 2.1 Typed IDs (UUID v7 newtypes: `SourceId`, `SceneId`, `ItemId`, `CanvasId`)
- [x] 2.2 `EngineClock` (monotonic ns) and `Rational` frame rate type
- [x] 2.3 Geometry types (`Vec2`, `Rect`, `Transform`, `Crop`, `BoundsMode`, `Alignment`) + layout math
- [x] 2.4 Error types (`thiserror`)

## 3. Engine runtime (`estream-engine`)
- [x] 3.1 `Command` enum + `apply`/inverse mechanism; `Transaction` (`Command::Batch`)
- [x] 3.2 Engine thread with bounded command channel, event broadcast and snapshot publishing (`arc-swap`)
- [x] 3.3 Undo/redo stack with coalescing keys
- [x] 3.4 `FrameClock` with hi-res timer pacing and lag accounting
- [x] 3.5 Video thread loop: tick sources → render canvases → publish frame textures
- [ ] 3.6 `tracing` setup + per-frame spans; optional Tracy feature (logging done; per-frame spans and Tracy pending)

## 4. Scene model & compositor (`estream-project`, `estream-engine`, `estream-gpu`)
- [x] 4.1 Project model: canvases, scenes, scene items, groups, sources (model in `estream-project`; runtime registry in engine)
- [x] 4.2 `VideoSource` trait (`update`, `tick`, `frame() -> Option<SourceFrame>`, `status`)
- [x] 4.3 Built-in test sources: solid color, test pattern (SMPTE bars + moving marker)
- [x] 4.4 wgpu device/queue init (high-performance adapter, software fallback), texture pool
- [x] 4.5 Quad pipeline: transform, crop, bounds, alignment, opacity, blend modes
- [x] 4.6 Linear `Rgba16Float` canvas + output conversion passes (sRGB8, NV12)
- [ ] 4.7 Render graph build + incremental transform patching (full rebuild per change works; incremental patching pending)
- [x] 4.8 Headless renderer + golden-image test harness

## 5. Persistence (`estream-project`)
- [x] 5.1 Serde schema v1 for scene collections
- [x] 5.2 Migration framework (`vN → vN+1`) with tests
- [x] 5.3 Atomic save, debounced autosave, crash-recovery file
- [x] 5.4 Load → validate references → build project; report broken references without failing

## 6. Verification
- [x] 6.1 Golden-image tests for transform/crop/bounds/blend scenarios
- [ ] 6.2 Frame-pacing benchmark: 60 fps for 10 min, jitter p99 < 1 ms, 0 lagged frames idle (short 5 s version passes; 10-min `#[ignore]` test not yet run)
- [x] 6.3 Undo/redo property test (random command sequences, undo all → original state)
- [x] 6.4 Save/load round-trip test
- [ ] 6.5 Steady-state allocation test for the video thread (needs a counting allocator; wgpu itself may allocate per submit)
