# Design: add-stats-health

## Crate `estream-health`
It has no media or engine dependencies, so it stays easy to test. The app gathers raw counters
into a `Sample` and the monitor does the rest.

```rust
pub struct Sample {
    pub at: Instant,
    pub canvases: Vec<CanvasCounters>,   // name, target_fps, rendered, lagged, encoder_dropped, render_ms
    pub outputs: Vec<OutputCounters>,    // key, name, encoder, frames, dropped, bytes
    pub destinations: Vec<DestCounters>, // key, name, state, bytes, dropped, queue_ms, reconnects
    pub system: SystemSample,            // cpu_process, cpu_total, memory_bytes, gpu, disk_free
    pub recording: bool,
}
```

- **Counters to rates:** `Monitor::push(sample)` turns cumulative counters into per-second rates
  by diffing them against the previous sample with the same key. A reset counter (restarted
  output) starts a fresh baseline.
- **History:** a `VecDeque` of 300 `Point`s (one per second). Series include fps, render ms,
  skipped %, CPU, GPU, memory, upload kbps, encoder dropped %, network dropped % and queue ms.
- **Rules** run on every push. Each rule computes a condition and a severity. An alert is
  **raised** after the condition has held for `raise_after` seconds and **cleared** after it has
  been false for `clear_after` seconds.

| Rule | Condition (10 s window unless noted) | Severity | Advice |
|---|---|---|---|
| render-lag | skipped frames > 2 % of target frames | warn (> 10 % problem) | Lower the canvas frame rate or resolution, hide heavy sources, close GPU-heavy apps |
| encoder-overload | encoder dropped > 1 % | warn (> 5 % problem) | Use a hardware encoder or a faster preset, or lower the resolution |
| network | network dropped > 1 %, or queue > 2 s | problem | Lower the stream bitrate or check the connection |
| reconnecting | a destination is reconnecting | problem | eStream retries on its own; check the network |
| cpu | total CPU > 90 % for 15 s | warn | Close other apps, or use a hardware encoder |
| gpu | GPU 3D > 95 % for 15 s | warn | Lower game graphics settings or cap the game's frame rate |
| disk | recording with < 5 GB free or < 10 min left | problem (< 1 GB) | Free space or change the recording folder |

- The **overall health** is the worst active severity.
- An **alert** carries `id` (the rule plus the output or destination key), `severity`, `title`,
  `detail` (numbers) and `advice`.

## System sampler (`estream_health::system`, Windows)
- **CPU:** `GetSystemTimes` gives the total load, and `GetProcessTimes` gives eStream's own,
  normalised by core count. Both are measured between samples.
- **Memory:** `K32GetProcessMemoryInfo` private bytes.
- **GPU:** the PDH counter `\GPU Engine(*engtype_3D)\Utilization Percentage`, summed per
  adapter, using the busiest adapter. It's read through `PdhCollectQueryData` on the sampling
  thread. When PDH is unavailable, the GPU shows as unknown.
- **Disk:** `GetDiskFreeSpaceExW` on the recording folder.

Sampling runs on a small background thread every second (PDH needs two collections, a second
apart). The app reads the latest system sample without blocking.

## App
- **Every second:** gather the counters, then `Monitor::push`.
  - Update the status bar health dot (0 ok, 1 warning, 2 problem) with the main alert's title.
  - Show a toast for each newly raised alert, once per alert id per session.
- **Stats dialog:**
  - **Tiles** (value plus sparkline): FPS, render time, skipped, CPU, GPU, memory, disk free,
    upload.
  - **Rows:** encoders (encoder, fps, dropped, Mb/s) and destinations (state, Mb/s, dropped,
    queue, reconnects).
  - **Alerts** with advice.
  - Sparklines are Slint `Path` elements from the history, normalised per series.
- **Remote API:** `Studio::system_stats()` (CPU %, memory MB, free disk MB) fills `GetStats`.

## Testing
- **Monitor unit tests** with synthetic samples:
  - rates from counters, and baselines after resets;
  - history capped at 300 points;
  - each rule raises after its delay, clears after calm, and picks the right severity;
  - the overall health and the main alert ordering.
- **System sampler test:** CPU stays within 0–100, memory is above 0, free disk space is above 0
  for the temp folder, and GPU is either a number or unknown.
- **In-app check:**
  - open the Stats dialog while recording;
  - simulate an overloaded encoder with a very slow software preset at 4K, and see the
    encoder-overload alert and its toast.
