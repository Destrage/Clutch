# Design: Lighter capture → compose → encode pipeline

## Context
Today's frame path for a recording of one game capture on Windows:

```
game present ─▶ WGC frame (D3D11) ─copy─▶ shared BGRA slot ─CPU spin until done─▶ published
video thread: composite ─▶ Rgba16Float canvas ─▶ sRGB preview (always)
                                          └──▶ NV12 (R8 + RG8) ─copy─▶ MAP_READ buffer
deliver: map ─▶ de-stride into Vec ─▶ clone per extra consumer ─▶ channel
encoder thread: Vec ─copy─▶ AVFrame (system memory) ─▶ h264_nvenc (CUDA upload)
```

Versions in use: wgpu 30.0.1 (DX12 backend), Slint 1.18.1 on the same wgpu device (until
`fix-gpu-contention` splits it), ffmpeg-next 9.0 over FFmpeg 7.x shared libraries.

The approach is the usual one for hardware encoders: hand the encoder a D3D11 texture that NV12
is rendered into, and let outputs with matching settings share one encoder. The implementation
below is our own, built from FFmpeg's hwcontext API documentation and the D3D11/D3D12
documentation.

## Goals / Non-Goals
- Goals: no CPU copies or CPU waits per frame on the hardware-encoder path; one encode per
  distinct encoder setting; no rendering that no one sees; lower steady-state memory.
- Non-goals: changing picture quality or encoder tuning; a game hook (injected DLL) capture
  method; Linux/macOS GPU encode paths (later changes).

## Decisions

### 1. Capture: GPU fence instead of a CPU spin
- A shared `ID3D11Fence` (D3D11.4, `D3D11_FENCE_FLAG_SHARED`) is created on the capture D3D11
  device and opened on the engine's D3D12 device. After copying a frame into a slot, the
  capture thread calls `ID3D11DeviceContext4::Signal(fence, n)` and `Flush`, and publishes the
  slot with value `n`. It does not wait.
- Before the video thread submits a frame that samples a capture slot, it makes the engine's
  D3D12 queue wait on the highest fence value among the slots it reads
  (`ID3D12CommandQueue::Wait`, through `wgpu::Queue::as_hal`). The wait is on the GPU; the CPU
  continues.
- Slot reuse: when the video thread moves to a newer slot it signals a second (D3D12-only)
  fence on its queue; everything submitted before that signal is the last work that can read
  the old slot. The capture side checks that fence's completed value (non-blocking) and only
  writes slots that are free; if none is, the frame is dropped rather than waited for. The ring
  grows from 3 to 4 slots, and the WGC frame pool from 2 to 3 buffers, since frames are now
  released as soon as their copy is queued.
- Fallback when `ID3D11Device5`/`ID3D11DeviceContext4` is unavailable (drivers older than
  WDDM 2.2, below the Windows 10 2004 minimum in practice): the capture thread waits for its
  copy as before, but sleeps between polls instead of spinning. A non-blocking query handoff
  was considered and left out; it adds a second ordering path for hardware Clutch doesn't
  target.
- Alternative considered: keep the spin but cap the capture rate. Rejected as the only fix:
  even at 60 copies per second, each spin holds a thread while the GPU is busy with the game.

### 2. Capture rate cap
- `GraphicsCaptureSession::MinUpdateInterval` (Windows 11 24H2 and later) is set to the
  shortest frame interval of the canvases that show the source.
- The interval comes from the source itself: the reader measures the time between its ticks
  (the video thread's frame interval, i.e. the fastest canvas) and smooths it.
- The callback releases a frame without copying when less than half that interval has passed
  since the last copied frame, and `MinUpdateInterval` is set to the same half interval. Half
  rather than the whole interval keeps a fresh frame for every render instead of beating
  against the game's rate: a 240 fps game costs about 120 copies per second at 60 fps, not 240.

### 3. GPU frames to hardware encoders
- Per canvas with a hardware-encoder consumer, the engine keeps a ring of 4 NV12 textures
  created on a D3D11 device on the engine's adapter (the existing capture interop device) with
  `D3D11_RESOURCE_MISC_SHARED_NTHANDLE | D3D11_RESOURCE_MISC_SHARED`, opened on the engine's
  D3D12 device.
- Spike result (task 3.1): wgpu 30 allows `NV12` textures only as `TEXTURE_BINDING`
  (`wgpu-types` 30.0.1, `texture/format.rs`: "We only support sampling nv12 and p010 textures
  until we implement transfer plane data"). wgpu can neither render into nor copy into an NV12
  texture, so the plane copy happens outside wgpu.
- The engine keeps rendering NV12 with its own WGSL conversion (same color math as today) into
  R8 and RG8 plane textures. Those are created on wgpu's D3D12 device as committed resources
  on a shared heap with `ALLOW_SIMULTANEOUS_ACCESS`, opened in D3D11 with
  `OpenSharedResource1`, and wrapped as wgpu render targets. Simultaneous access matters:
  wgpu leaves a texture it rendered to in the render-target state, which D3D11 can't see;
  simultaneous-access resources decay to `COMMON` after each submission, so no state handoff
  between the APIs is needed.
- The engine side is platform-neutral: `estream_engine::GpuExport` creates plane slots,
  queues the release wait and signals the ready fence; `estream-media` implements it for
  D3D11 (`gpu_frames.rs`), and the app passes it in `EngineConfig::gpu_export`. A ring slot
  is free once every `GpuFrame` holding its handle is dropped; a consumer that holds frames
  only causes drops, never a stall.
- On the encoder's D3D11 context (the same interop device, which FFmpeg's D3D11VA device
  context wraps), after a GPU-side fence wait, a pass-through pixel shader copies the two planes
  into the NV12 texture through per-plane render target views
  (`ID3D11Device3::CreateRenderTargetView1`, `PlaneSlice` 0 and 1; D3D11.3, Windows 10).
  D3D11 has no per-plane copy for NV12 (planes aren't separate subresources there), hence
  the shader. It does no color math, so the WGSL conversion stays the single source of truth;
  the copy is ~3 MB of GPU bandwidth per 1080p frame.
- Alternatives considered: a raw D3D12 `CopyTextureRegion` into NV12 plane subresources on
  wgpu's queue (needs hand-managed command lists and resource states around wgpu's own); and
  the D3D11 video processor converting RGBA → NV12 (fixed-function, but its matrix, range and
  chroma filter would differ from the WGSL path).
- FFmpeg gets an `AV_HWDEVICE_TYPE_D3D11VA` device context wrapping that D3D11 device, and
  each encoder an `AVHWFramesContext` (`AV_PIX_FMT_D3D11`, sw format NV12, render-target bind
  flag, FFmpeg's own pool). The plane copy renders into the pooled texture FFmpeg hands out,
  under FFmpeg's device lock (the immediate context is shared by every encoder thread).
- Ordering: wgpu's queue signals a shared `ready` fence after the frame's submit; the encoder
  waits on it on the D3D11 context before the copy and signals a shared `released` fence
  after; wgpu's queue waits on `released` before rendering into those planes again. All three
  are GPU-side waits. Frames travel over the existing bounded channel as a planes handle plus
  the `ready` value instead of a `Vec`.
- Encoders: `h264_nvenc`, `hevc_nvenc`, `av1_nvenc` and the AMF encoders accept D3D11 frames
  directly. QuickSync needs QSV frames: a derived QSV frames context
  (`av_hwframe_ctx_create_derived`) is tried, and QSV falls back to the CPU path if that fails.
- Fallback: if the export can't be created, planes can't be made for a canvas, or the GPU
  encoder fails to open, the output logs why and switches to the CPU readback before writing
  anything. A GPU encoder that later receives CPU frames ends its session with an error
  rather than freezing. A failure on the first `send_frame` is not retried on the CPU path:
  the muxer header is already written by then, and in testing the failures surface at open.
  Software encoders always use the readback.
- This also drops NVENC's CUDA context, since NVENC opens its D3D11 interface for D3D11 frames.

### 4. Shared encoder sessions
- An `EncoderHub` per engine owns video encoder sessions keyed by (canvas, resolved encoder
  name, rate control, bitrate, quality, preset, keyframe interval, codec). Outputs ask the hub
  for a session and get a packet subscription; the session ends when its last subscriber
  leaves. Multistream's per-group sharing moves onto the hub.
- Each subscriber gets its own bounded packet queue; a slow subscriber (a stalled network
  destination) drops its own packets and asks for a keyframe, never blocks the encoder or
  others.
- Forced keyframes (stream reconnect) apply to the shared session; an extra IDR in a
  recording or clip buffer is harmless. Subscribers that start mid-GOP wait for the next
  keyframe; a new subscriber requests one so it starts within one frame.
- Audio encoders are shared the same way, keyed by codec, bitrate and track.
- Different stream bitrates still need separate sessions. The hub reports its session count
  to the stats panel so the cost is visible.

### 5. Preview on demand and single-source pass-through
- The UI publishes a `preview_wanted` flag and the display refresh rate per canvas (an atomic
  in the engine's shared state). The video thread renders the sRGB preview only when wanted,
  at most at that rate. Minimized, hidden or another page showing: no preview work.
- Pass-through: when a canvas's program scene is exactly one visible item whose source is
  opaque, has no filters, covers the whole canvas at 1:1 size (or a uniform fit), with no
  transition running, the frame is converted to NV12 (and the preview, when wanted) straight
  from the source texture, skipping the composite into the 16-bit canvas. Anything else falls
  back to the full composite on the next frame, with no visible difference. A golden-image
  test compares both paths.
- Fused conversion: the NV12 and sRGB preview outputs are written by one compute pass that
  reads the canvas once.
- The 16-bit float canvas stays the default (it keeps blending and filters accurate). Task 5.5
  measures an 8-bit sRGB canvas; it's only switched if the gain is significant and golden-image
  tests still pass.

### 6. UI loop
- The 4 ms repeating timer is replaced by: a wake when the engine publishes a new canvas frame
  (rate-limited to the display refresh), event-channel wakes for engine/output events, and
  separate slow timers (250 ms–1 s) for chat, stream details and health. While minimized, only
  the event wakes and slow timers run.

### 7. Memory
- Encoder probing runs `clutch.exe --probe-encoders` as a child process that opens each
  candidate and prints the result; the parent caches it in settings keyed by GPU driver
  version, so later starts don't probe at all. The parent never loads AMF/QSV/Kvazaar
  runtimes it won't use.
- Before/after breakdowns are taken with VMMap and D3D12 residency queries; new large items
  get their own tasks.

## Risks / Trade-offs
- D3D11 hwframes and custom pools are less exercised than system-memory frames in FFmpeg
  builds → keep the CPU path as an automatic per-session fallback, and test with NVENC, AMF and
  QSV hardware where available.
- Fences need Windows 10 1703+ drivers with D3D11.4 → event-query fallback, also without spin.
- A shared encoder couples outputs (one keyframe request affects all) → acceptable; recordings
  and clip buffers tolerate extra keyframes.
- Pass-through must look identical to the composite → golden-image tests on both paths;
  conditions checked every frame.

## Migration
None for users: settings and files are unchanged. A hidden setting
(`"gpu_encode": false` in settings.json) forces the CPU path for troubleshooting.

## Open Questions
- Can wgpu 30 render to NV12 plane views on DX12 (removes the plane copy)? Answered by task 3.1.
- Should the clip buffer move to disk-backed storage to cut its memory? In an earlier
  2026-10-25 run with clip mode on (120 s, quality 20) memory grew ~450 MB while the buffer
  filled. Measured in task 8.2; a separate change if yes.
