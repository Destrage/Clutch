# Design: Keep the compositor independent of UI presentation

## Evidence (beta.3, Windows 10, GTX 1650 Ti)
- Final session: Dota 2 game capture starts, then the log goes silent. The status bar shows
  `0.0 / 60 fps` with `render 3.01 ms` frozen. `update_stats` runs on the UI thread, so the
  UI was alive while the canvas `rendered` counter stopped.
- Sessions that ran a fullscreen game for minutes end with `scene collection saved` and no
  `engine stopped`. `Engine::stop` joins the video thread, whose last statement is
  `device.poll(PollType::wait_indefinitely())`.
- `Surface::configure` → `GpuWaitTimeout`: wgpu-core 30 `Device::configure_surface` runs
  `maintain(wait_indefinitely)` and returns `GpuWaitTimeout` when the queue isn't empty
  afterwards ("another thread is submitting at the same time"). It returns before the new
  presentation is stored, so the swapchain keeps its old size.
- wgpu-core 30 `Queue::present` takes `device.command_indices.write()` around
  `hal::Queue::present` (DXGI `Present`). `Queue::submit` takes the same lock.

## Video thread watchdog
`VideoProgress` (in `estream-engine::video`) holds atomics only, so the hot path stays
allocation- and lock-free:

```rust
pub(crate) struct VideoProgress {
    stage: AtomicU8,        // Stage as u8
    since: AtomicU64,       // EngineClock ns when the stage was entered
    source: [AtomicU64; 2], // SourceId (u128) being ticked, when stage == TickSource
    done: AtomicBool,       // set when video::run returns (or unwinds)
}
```

Stages: `Sync`, `Publish`, `Idle`, `Wait`, `TickSource`, `Filters`, `Render`, `Submit`,
`Poll`, `Deliver`, `Shutdown`. `VideoCore` owns an `Arc<VideoProgress>` and updates it at
each transition, and per source in `tick_sources`.

The watchdog is a detached thread started next to the video thread. Every 500 ms it compares
`clock.now()` with `since`. Past 2 s it logs once:
`warn!(stage, source = <name from the render graph>, stalled_ms, "video thread stalled")`.
When the stage changes again, it logs `info!(stalled_ms, "video thread recovered")`. It exits
when `done` is set, which a drop guard in `video::run` does even on panic. It isn't joined,
so a hung video thread can't also hang the watchdog, and it still reports a hang during
shutdown.

`Wait` and `Idle` legitimately sleep for up to one frame, far below the threshold.

## Separate engine device (Windows)
Two `GpuContext`s on the same adapter (matched by LUID):
- **UI device:** handed to Slint (`WGPUConfiguration::Manual`). It renders and presents the
  UI, and imports preview images.
- **Engine device:** the compositor, sources, capture interop (`D3D11Interop` opens shared
  textures here), filters, encoders and readback.

Each device has its own D3D12 direct queue and its own wgpu-core locks. A blocking UI
`Present` or a `configure_surface` drain no longer involves compositor submissions. The OS
GPU scheduler interleaves the two queues.

**One D3D12 device, two queues.** wgpu-hal opens every wgpu device of an adapter on the
adapter's single `ID3D12Device` (D3D12 devices are per-adapter singletons) and gives each its
own direct queue. So no NT handles are needed: a resource or fence made once is valid on both
queues. `estream-gpu::PreviewBridge` (only on the engine's `GpuContext`) holds the device, both
raw queues and two fences.

**Preview bridge.** Each preview slot of a canvas output (and of the studio Preview) is a
committed resource with `D3D12_RESOURCE_FLAG_ALLOW_SIMULTANEOUS_ACCESS`, wrapped with
`texture_from_raw` → `create_texture_from_hal` on both devices (engine: render target, UI:
copy source). Simultaneous access lets each queue use it from the common state, so the two
wgpu state trackers never have to agree; each side only ever uses it one way, so wgpu issues
at most one barrier from the initial state and relies on implicit promotion afterwards.
Textures from hal count as initialized, so wgpu never clears them.

**Ordering.** Raw `Signal` / `Wait` calls next to our own submits (wgpu-hal's
`add_signal_fence` / `add_wait_fence` attach to whichever submit comes next, possibly another
thread's):
- After the video thread's submit, the engine signals `frame = n` on its queue and publishes
  `n` per slot with the frame (`ready`; 0 when the slot's preview wasn't drawn).
- On the UI tick, `UiPreview::show` queues a GPU `Wait(frame, n)` on the UI queue, copies the
  slot into one of two UI-owned textures (alternating, so the image Slint gets changes), then
  signals `copied = m` and stores `m` in the slot (`released`).
- Before drawing a slot's preview, the engine checks `copied` without waiting. If the UI's
  last copy of that slot hasn't finished, it leaves the preview alone for that frame (the
  canvas and encoder output are unaffected) and publishes `ready = 0`; the UI keeps its last
  picture. Neither CPU thread ever waits for the other's GPU work.

The UI copies instead of sampling the shared texture directly so the engine's slot is free
again as soon as the copy is done, rather than whenever Slint stops drawing it.

**Other imports.**
- Scene thumbnails (`snap_thumb`) copy from the UI's picture, so they stay on the UI device.
- Clip and montage engines (`clips.rs`, `montage.rs`) aren't live outputs. They keep using
  the UI device (no bridge, so their previews are shown directly) and are revisited if they
  show contention.

**Fallback.** If the second device, the fences or a shared preview can't be created, log it
and run on one shared device as before (previews are shown directly). Linux and macOS use the shared device until their own interop exists
(Vulkan external memory, IOSurface).

## GPU scheduling priority
In `estream-gpu` on Windows, after the adapter is chosen:
`D3DKMTSetProcessSchedulingPriorityClass(GetCurrentProcess(), HIGH if HAGS else REALTIME)`.
Skip it when the vendor ID is Intel (0x8086). A failure (non-admin) is logged at debug
level. `IDXGIDevice::SetGPUThreadPriority`
applies to D3D11 devices, so it's set on the capture interop device only.

## Alternatives considered
- **Native child HWND for the preview, drawn by the video thread**.
  This removes contention fully, but Slint draws on top of the preview: the floating "In
  this scene" panel, selection handles and guides would be covered by the child window and
  would have to move into the engine. Too invasive for now.
- **Pause compositor submits around window resizes.** Fixes only the configure race; the
  Present lock remains.
- **`SetMaximumFrameLatency(16)`**: often set on a D3D11 DXGI device to avoid
  Present stalls. For D3D12 the latency belongs to each swap chain, which Slint owns, so it
  isn't adopted. The device split makes it unnecessary for the compositor.
