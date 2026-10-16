# Design: Core Video Sources

## Decisions

### Property schema
Each source type declares a `PropertySchema` (fields: bool, int/float with range, enum, color,
font, path, device list, text). The inspector renders it generically. Settings are stored as JSON
values in the scene collection, validated against the schema. Plugins will reuse this mechanism.

### Capture → GPU path (Windows)
1. Windows.Graphics.Capture delivers `ID3D11Texture2D` frames (BGRA8) on a D3D11 device created on
   the same adapter as wgpu.
2. Each frame is copied into a pooled shared texture (`D3D11_RESOURCE_MISC_SHARED_NTHANDLE |
   SHARED_KEYEDMUTEX`), which is opened on the D3D12 side once and wrapped as a `wgpu::Texture`
   via `create_texture_from_hal`.
3. The compositor samples it directly. There is no CPU readback.
4. If cross-API sharing fails (old drivers), fall back to CPU staging with a warning in the log.

### Camera
Media Foundation `IMFSourceReader` with hardware MJPEG/NV12/YUY2 → GPU upload. YUV→RGB conversion
runs in a WGSL shader with the correct matrix (BT.601/709) and range (limited/full).

### Game capture
OBS uses DLL injection hooks (graphics-hook) for exclusive-fullscreen games. That approach has
anti-cheat risk and is complex. Phase 1 uses WGC, which handles borderless and windowed games well.
A hook-based capture may come later as a separate change, with a signed helper.

### Text
Use `cosmic-text` for shaping/layout (complex scripts, emoji, fallback fonts) and render glyphs to a
GPU atlas. Outline and shadow are done with SDF glyphs.

## Implementation notes (learned while building)
- **Sync without keyed mutexes**: the capture callback copies into a free ring slot, waits on a
  D3D11 event query, then publishes the slot. The video thread marks the slot it reads, and the
  writer never touches that slot or the newest one.
- **Text** is rasterized on the CPU (worker thread, only when the text or style changes) instead
  of SDF on the GPU: simpler, exact outlines/shadows, and supports color emoji. Revisit if
  animated text becomes a feature.
- **No blocking work on the video or control threads**: capture start-up (window enumeration,
  WinRT activation) runs on a helper thread; image decoding, text rendering and camera reads have
  their own worker threads; the video thread only polls their results with `try_lock`/`try_recv`.
- **Deadlock lesson**: `GetWindowTextW` on a window of our own process sends `WM_GETTEXT` to the
  UI thread. Validation used to enumerate windows on the engine's control thread while the UI
  thread waited for it, which deadlocked. Our own windows are now skipped before any
  message-sending call, and source types supply a cheap validation schema that never enumerates.
- wgpu: two `wgpu::Instance`s on one GPU in the same process can mix up resource IDs when
  importing HAL textures in parallel; the app uses a single device and tests share one.

## Risks
- WGC shows a yellow capture border on Windows 10. Mitigation: request `IsBorderRequired = false`
  where supported (Windows 11).
- Media Foundation device quirks with capture cards. Mitigation: expose a "buffering" toggle and
  manual format selection.
