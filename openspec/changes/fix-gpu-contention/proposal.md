# Change: Keep the compositor independent of UI presentation

## Why
Testing 0.4.0-beta.3 on Windows 10 with a GTX 1650 Ti laptop (Valorant, Dota 2) showed two
failures, both caused by Slint and the engine sharing one wgpu device and queue:

- **The video thread stops.** The status bar showed 0.0 fps while the UI kept updating, so
  the preview went black and the stream got no frames. Three sessions never logged "engine
  stopped" on exit: shutdown hung joining the video thread. The logs don't show where the
  thread was stuck.
- **The window draws at the wrong size.** When the window is resized, which a fullscreen game
  changing the display mode also does, Slint reconfigures its surface. wgpu-core 30 waits for
  the queue to drain and then fails with `GpuWaitTimeout` if another thread submitted in the
  meantime. The compositor submits every frame, so the configure often fails and the old
  swapchain size stays. The DX12 backend never reports `Outdated`, so Slint never retries.
  Logged as `Failed to wait for GPU to come idle before reconfiguring the Surface`, on
  Windows 10 and 11.

There is also a second contention point. wgpu-core holds the device's `command_indices` write
lock across `IDXGISwapChain::Present`. While the UI presents (Slint uses `AutoVsync`), every
compositor `queue.submit` waits behind it.

The obs-studio source shows the usual way to avoid this: one thread owns all rendering for its
device, preview swap chains are resized on that thread, and the render loop only presents
when the frame-latency waitable reports a free slot. The UI toolkit draws elsewhere. The same
codebase also raises the process's GPU scheduling priority, which matters when an uncapped
game saturates the GPU.

## What Changes
- **Video thread watchdog:** the video loop records which stage it is in (and which source it
  is ticking). A watchdog thread logs a warning naming the stage and source when no progress
  is made for 2 seconds, and logs again when the thread recovers. This also covers the
  shutdown wait.
- **Separate engine GPU device on Windows:** the engine (compositor, capture interop,
  encoders, previews) gets its own wgpu device, apart from the device Slint renders and
  presents with. Preview textures are created as D3D12 shared resources on the engine device
  and opened on the UI device. A shared D3D12 fence orders engine writes before UI reads. UI
  presents and surface reconfiguration then can't block or race compositor submits. Linux and
  macOS keep the single shared device for now.
- **GPU scheduling priority on Windows:** at startup, raise the process GPU scheduling class
  with `D3DKMTSetProcessSchedulingPriorityClass`: HIGH when hardware-accelerated GPU
  scheduling is on, REALTIME otherwise. Skip it on Intel adapters, and log at debug level if
  it fails (it usually needs administrator rights).

## Impact
- Affected specs: `engine-runtime`, `studio-ui`.
- Affected code: `estream-engine` (video loop progress, watchdog), `estream-gpu` (device
  creation, shared preview textures, cross-device fence, scheduling priority),
  `estream-capture` (interop stays on the engine device), `apps/estream` (two devices at
  startup, preview, scene thumbnail and montage image imports).
- Depends on `fix-game-capture-stability` (process priority, device-removed logging).
