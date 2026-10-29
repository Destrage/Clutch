# Change: Lighter capture → compose → encode pipeline

## Why
A benchmark on 2026-10-25 (RTX 4070 Ti SUPER, 20 cores, Apex Legends in the Firing Range)
recorded one scene with Clutch 0.5.0-beta.1: one game capture, desktop audio and mic,
1920×1080 at 60 fps, NVENC H.264 CBR 12 Mbps, mkv, no replay buffer. Every frame was encoded
with nothing dropped and NVENC load was ~20%, but Clutch's own overhead was far above what this
scene needs:

| Metric (5 min average) | Clutch 0.5.0-beta.1 | Target |
|---|---|---|
| CPU, whole process tree (% of 20 cores) | 3.23 | ≤ 0.75 |
| Private memory | 1206 MB | < 500 MB |
| GPU 3D engine, process | 10.7% | ≤ 8.5% |
| Render time per frame, avg / p95 | 0.94 / 1.67 ms | ≤ 0.4 / < 0.6 ms |

Reading the pipeline shows where the cost comes from.

- **Every encoded frame goes through the CPU.** The canvas is converted to NV12, copied into a
  mapped buffer, de-strided into a `Vec`, cloned once per extra output, copied again into the
  `AVFrame`, then uploaded back to the GPU by the encoder driver. At 1080p60 that is
  ~190 MB/s each way, plus three or more CPU copies of every frame for every output. FFmpeg's
  NVENC also creates a CUDA context to take system-memory frames, which costs memory.
- **Game and window capture spin the CPU.** After each captured frame the capture callback
  copies it, then busy-waits (`yield_now` loop on a D3D11 event query) until the GPU is done.
  It does this at the game's present rate, not the canvas rate, so a 240 fps game costs four
  waits per canvas frame.
- **Each output encodes separately.** Recording, the clip-mode buffer and each stream with its
  own settings open their own encoder session, even when the settings are the same.
- **Work nobody sees is still done.** The sRGB preview is rendered every frame while the window
  is minimized, a fullscreen single game capture is still composited into a 16-bit float
  canvas, and the UI loop wakes every 4 ms whether or not anything changed.

## What Changes
- **Measure first:** a repeatable recording benchmark (kept by the maintainers in the private
  `clutch-bench` repository), and the video thread reports per-stage timings, so each step below is measured against the baseline.
- **Capture without CPU waits:** capture copies are ordered with a shared GPU fence that the
  engine's queue waits on, instead of the capture thread spinning. Frames that arrive faster
  than the fastest canvas needs are released without being copied, and the capture session's
  minimum update interval is set where Windows supports it.
- **GPU-to-encoder frames (zero copy):** hardware encoders (NVENC, AMF, QuickSync) receive the
  canvas as a GPU NV12 texture through FFmpeg's D3D11 hardware frames. No readback, no CPU
  copy, no CUDA context. The CPU readback stays for software encoders and as an automatic
  fallback when the GPU path can't be set up.
- **Encode once, fan out:** outputs on the same canvas whose encoder settings match share one
  encoder session (today only stream destinations do). Recording and the clip-mode buffer
  share one by default when their settings match.
- **Skip work that isn't seen:** the preview is rendered only while a view shows it, and at
  most at the display's rate. A canvas showing a single opaque full-canvas source with no
  filters or transition converts that source directly instead of compositing it. The sRGB
  preview and NV12 conversion read the canvas in one pass.
- **Quieter UI loop:** the UI wakes for new frames and events instead of every 4 ms, runs
  slow housekeeping (chat, stream details) on its own timers, and idles while minimized.
- **Memory:** encoder probing runs in a short-lived child process with the result cached per
  driver version, so probed encoder runtimes don't stay loaded. A memory breakdown is taken
  before and after, and anything else large gets its own task.

Picture quality is unchanged: at the same settings the encoder gets the same pixels. Encoder
tuning for more quality at the same cost (adaptive quantization, multipass) is left to
`add-performance-profiles`, where it can be measured per profile.

## Targets
Measured with the recording benchmark on the reference PC above, same scene and settings:
- process CPU at most 0.75% of 20 cores;
- render time at most 0.4 ms on average and under 0.6 ms at p95;
- private memory under 500 MB with clip mode off;
- GPU 3D engine use at most 8.5%;
- clip mode + recording + three destinations with different bitrates: 4 encoder sessions
  instead of 5, zero dropped frames.

## Impact
- Affected specs: engine-runtime, video-sources, recording, studio-ui.
- Affected code: `crates/estream-engine` (video loop, feed, stage timings, pass-through),
  `crates/estream-gpu` (fused conversion, shared NV12 targets), `crates/estream-capture`
  (fence-ordered capture, capture rate cap), `crates/estream-media` (D3D11 hardware frames,
  shared encoder sessions, probe process), `apps/estream/src` (preview demand, UI loop).
- Builds on `fix-gpu-contention` where both touch the engine's GPU device: the shared fence
  and textures here are created on the engine device that change introduces, or on the single
  shared device when it falls back.
- Windows first. Linux and macOS keep the CPU readback path until their own changes.
