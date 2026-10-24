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

**Preview bridge.** Preview and program textures that the UI shows are created by
`estream-gpu::SharedTexture` on the engine device with `D3D12_HEAP_FLAG_SHARED`, exported as
an NT handle and opened on the UI device (`OpenSharedHandle` → `texture_from_raw` →
`create_texture_from_hal`). The UI-side `wgpu::Texture` is what `slint::Image::try_from`
receives.

**Ordering.** A D3D12 fence created shared on the engine device and opened on the UI device:
- The engine stages `add_signal_fence(fence, n)` on its hal queue before the submit that
  writes preview slot *k*. It publishes `(k, n)` with the frame.
- The UI stages `add_wait_fence(fence, n)` on its hal queue before Slint's next frame, so its
  GPU reads wait for the engine's writes without blocking either CPU thread.
- Slots are already rotated by the engine (`pslot`). The engine doesn't rewrite a slot until
  the UI has presented a newer one. The UI signals a second shared fence after its frame, and
  the engine checks it without waiting, skipping a preview slot update rather than stalling.

**Other imports.**
- Scene thumbnails (`snap_thumb`) become bridge textures; the copy is recorded on the engine
  device.
- Clip and montage engines (`clips.rs`, `montage.rs`) aren't live outputs. They keep using
  the UI device for now and are revisited if they show contention.

**Fallback.** If shared-resource creation or fence sharing fails, log it and run on one shared
device as today. Linux and macOS use the shared device until their own interop exists
(Vulkan external memory, IOSurface).

## GPU scheduling priority
In `estream-gpu` on Windows, after the adapter is chosen:
`D3DKMTSetProcessSchedulingPriorityClass(GetCurrentProcess(), HIGH if HAGS else REALTIME)`.
Skip it when the vendor ID is Intel (0x8086). A failure (non-admin) is logged at debug
level. This mirrors the obs-studio `set_priority` behaviour. `IDXGIDevice::SetGPUThreadPriority`
applies to D3D11 devices, so it's set on the capture interop device only.

## Alternatives considered
- **Native child HWND for the preview, drawn by the video thread** (the obs-studio layout).
  This removes contention fully, but Slint draws on top of the preview: the floating "In
  this scene" panel, selection handles and guides would be covered by the child window and
  would have to move into the engine. Too invasive for now.
- **Pause compositor submits around window resizes.** Fixes only the configure race; the
  Present lock remains.
- **`SetMaximumFrameLatency(16)`**: obs-studio sets it on its D3D11 DXGI device to avoid
  Present stalls. For D3D12 the latency belongs to each swap chain, which Slint owns, so it
  isn't adopted. The device split makes it unnecessary for the compositor.
