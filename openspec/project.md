# Project Context

## Purpose

**eStream** is a live-streaming and recording studio built from scratch. It aims to match the
OBS Studio feature set and improve on it in performance, reliability, UX and extensibility.

### What "better than OBS" means (product pillars)

1. **Modern, clean UX.** A single coherent design system, a command palette (Ctrl+K), undo/redo for
   every edit, on-canvas snapping and guides, drag-and-drop everywhere, and sensible defaults
   with no "Advanced" settings maze.
2. **Performance by design.** GPU-first compositing (wgpu), zero-copy capture → compose → encode
   paths where the platform allows, no per-frame heap allocation on hot paths, and a lock-free
   engine/UI split.
3. **Reliability.** Memory-safe core (Rust). Crash-prone components (browser source, third-party
   plugins) run out of process or in a sandbox, so they can't take down a live stream. Outputs
   reconnect automatically and degrade gracefully.
4. **Multi-everything as a first-class feature.** Multiple canvases (e.g. 16:9 + 9:16 vertical at
   the same time), multistreaming to many destinations, per-source ISO recording, and multi-track audio.
5. **Modern protocols & codecs.** RTMP(S), Enhanced RTMP (HEVC/AV1), SRT, WHIP (WebRTC) and
   AV1/HEVC/H.264 with hardware encoders.
6. **Automation & control.** An obs-websocket v5 compatible remote API (Stream Deck, Touch Portal,
   bots keep working), built-in macros/triggers, and a sandboxed WASM plugin system.
7. **Built-in AI-assisted features (local, offline).** Noise suppression, background removal,
   auto-captions, all optional and running locally.

### Non-goals (for now)

- Multi-track NLE features (multiple video tracks, keyframed effects, color grading) beyond the
  clip trimmer (`add-clips`) and the single-track montage editor (`add-montage-editor`).
- A cloud service or account system. eStream is a local-first desktop app.
- Binary compatibility with OBS C plugins. (We import OBS scene collections instead.)

## Tech Stack

| Concern | Choice | Rationale |
|---|---|---|
| Language | **Rust** (stable, edition 2024) | Memory safety + C-level performance; great FFI for platform APIs |
| GPU / compositor | **wgpu** (DX12 on Windows, Metal on macOS, Vulkan on Linux) | Portable modern GPU API, WGSL shaders, compute support |
| UI toolkit | **Slint** | Native GPU-rendered UI, declarative `.slint` markup, live-preview tooling, can render wgpu textures zero-copy for the preview |
| Encoding / muxing | **FFmpeg (libav\*)** via `rsmpeg`/`ffmpeg-next`, dynamically linked (LGPL build) | Hardware encoders (NVENC, AMF, QSV, VideoToolbox, VAAPI), all containers, RTMP/SRT |
| WebRTC / WHIP | **str0m** (sans-IO WebRTC) | Pure Rust, no libwebrtc dependency |
| Audio I/O | **cpal** + platform APIs (WASAPI process loopback, CoreAudio, PipeWire) | Cross-platform device I/O; per-app capture via native APIs |
| Audio DSP | In-house mixer (f32, 48 kHz); `nnnoiseless` (RNNoise); CLAP hosting via `clack` | Low-latency mixer we control |
| Screen capture | Windows.Graphics.Capture / DXGI Desktop Duplication (`windows` crate); ScreenCaptureKit; PipeWire portal | Native, GPU-texture based |
| Camera | Media Foundation / AVFoundation / V4L2 (via `nokhwa` or direct) | |
| Browser source | CEF, **out of process** (offscreen rendering, shared texture) | Isolation; feature parity with OBS browser source |
| Plugins / scripting | **wasmtime** + WIT component model | Sandboxed, language-agnostic, crash-isolated |
| Remote API | `tokio` + `tokio-tungstenite`, obs-websocket v5 protocol | Ecosystem compatibility |
| Async runtime | `tokio` (I/O only; never on render/audio threads) | |
| Serialization | `serde` + JSON (scene collections), TOML (settings) | Human-readable, diff-friendly |
| Local AI | `ort` (ONNX Runtime, DirectML/CoreML/CUDA EPs); whisper via `whisper-rs` | Offline inference |
| Logging / tracing | `tracing` + `tracing-subscriber`, Tracy integration in dev | Frame-level profiling |

Platform priority: **Windows 10 2004+ / Windows 11 first**, then macOS 13+, then Linux (Wayland + X11).
All platform code sits behind traits, so the core stays portable from day one.

## Architecture Overview

```
┌───────────────────────────── apps/estream (Slint UI) ─────────────────────────────┐
│  Panels, preview widget, command palette   ── Commands ──▶   ◀── State/Events ──   │
└───────────────────────────────────────────────┬────────────────────────────────────┘
                                                │ (crossbeam channels, snapshots)
┌───────────────────────────── estream-engine ──▼────────────────────────────────────┐
│ Command bus → Project model (scenes, sources, canvases) → Undo stack               │
│ Video thread: frame clock → source tick → wgpu compositor (per canvas) → outputs   │
│ Audio thread: device capture → per-source filters → mixer (6 tracks) → outputs     │
└──────────┬──────────────────────┬──────────────────────┬───────────────────────────┘
     estream-capture        estream-media            estream-api / plugins
   (platform sources)   (encoders, muxers, outputs)  (obs-websocket, WASM host)
```

Key rule: **every state mutation is a `Command`**. The UI, hotkeys, remote API, macros and plugins
all use the same command bus. That's how undo/redo, remote control and automation stay consistent.

### Workspace layout

```
apps/estream/            # Slint desktop app (binary)
crates/estream-core/     # IDs, time/clock types, pixel formats, errors, shared traits
crates/estream-gpu/      # wgpu device, texture pool, shaders (WGSL), color conversion
crates/estream-engine/   # command bus, undo/redo, render graph, frame clock, sources, video thread
crates/estream-project/  # project model (the document), persistence, migrations, OBS import
crates/estream-capture/  # source implementations (platform-specific, behind traits)
crates/estream-audio/    # audio graph, mixer, filters, meters
crates/estream-media/    # encoders, muxers, recording, streaming outputs
crates/estream-api/      # remote control (obs-websocket v5)
crates/estream-plugin/   # WASM plugin host + WIT interfaces
```

## Project Conventions

### Code Style
- `cargo fmt` (default rustfmt), `cargo clippy --all-targets -- -D warnings` must pass.
- No `unwrap()`/`expect()` in non-test code except for proven invariants, which need a comment.
- Errors: `thiserror` in library crates, `anyhow` only in the app binary.
- Hot paths (video/audio threads): no allocation, no locks held across frames, no blocking I/O,
  no `tokio`. Use pre-allocated pools and lock-free channels.
- `unsafe` only inside platform/FFI modules, each block with a `// SAFETY:` comment.
- Naming: crates `estream-*`, modules snake_case, types PascalCase, commands are verbs
  (`AddSource`, `SetTransform`).

### Architecture Patterns
- Command/event architecture (see above). Commands are serializable and invertible (for undo).
- Engine state is owned by the engine thread. The UI receives immutable snapshots (`Arc`) and events.
- Sources implement `VideoSource` / `AudioSource` traits. Filters implement `VideoFilter` / `AudioFilter`.
- Platform-specific code lives in `platform/{windows,macos,linux}` modules behind `cfg`.

### Testing Strategy
- Unit tests per crate. Compositor golden-image tests (headless wgpu, tolerance-based compare).
- Audio DSP tests with known signals (sine sweeps, impulse).
- Integration tests: record a synthetic scene for N seconds and verify the file with `ffprobe`.
- Performance budgets are enforced in CI benchmarks (`criterion`) on the compositor and mixer.
- Every OpenSpec scenario should map to at least one automated test where feasible.

### Git Workflow
- Trunk-based: short-lived branches named `<change-id>`, PRs that reference the OpenSpec change.
- Conventional Commits (`feat:`, `fix:`, `perf:`, `refactor:`, `docs:`, `chore:`).
- A change is archived (`openspec archive <id>`) after it merges.

## Domain Context

- **Canvas**: an output video surface with its own resolution/FPS (e.g. 1920×1080@60 landscape,
  1080×1920@30 vertical). OBS has exactly one; eStream supports many.
- **Scene**: an ordered list of scene items composited onto a canvas.
- **Source**: a producer of video and/or audio (display capture, camera, media file, text, browser…).
  A source can be referenced by many scene items.
- **Scene item**: a placement of a source in a scene (transform, crop, visibility, blend mode, filters).
- **Filter**: per-source or per-scene processing (color correction, chroma key, noise suppression…).
- **Transition**: an animated switch between scenes (cut, fade, slide, stinger, shader).
- **Studio mode**: separate Preview (edit) and Program (live) views with a transition between them.
- **Output**: a consumer of encoded media: recording, stream, replay buffer, virtual camera.
- **Track**: one of up to 6 mixed audio buses, which can be routed to outputs independently.

## Important Constraints

- **Latency**: capture-to-preview ≤ 2 frames. Audio monitoring round trip ≤ 20 ms on WASAPI.
- **Performance target** (reference: 8-core CPU, RTX 3060): 1080p60 scene with display capture,
  camera and 3 overlays, streaming + recording with NVENC, at < 5% CPU of the UI+engine process
  and with zero dropped frames.
- **A/V sync**: all media is timestamped against one monotonic engine clock. Drift stays < 1 ms/hour.
- **Licensing**: FFmpeg is linked dynamically with an LGPL build. No GPL-only components in the default build.
- **Accessibility**: full keyboard navigation and screen-reader labels on all controls.

## External Dependencies

- FFmpeg 7.x shared libraries (bundled with the app on Windows/macOS).
- CEF (Chromium Embedded Framework) binary distribution, used only by the browser-source helper process.
- GPU drivers that expose NVENC / AMF / QSV for hardware encoding (software x264/SVT-AV1 fallback).

## Roadmap (planned OpenSpec changes)

Each change below gets its own `openspec/changes/<id>/` when work starts. ✅ = proposal written.

| Phase | Change ID | Delivers | OBS parity / beyond |
|---|---|---|---|
| 1 – Foundation | ✅ `add-engine-core` | Workspace, command bus, scene graph, wgpu compositor, frame clock, persistence, undo/redo | Parity + undo/redo |
| 1 | ✅ `add-studio-shell` | Slint app, live preview with on-canvas editing, scenes/sources panels, properties, command palette | Better UX |
| 1 | ✅ `add-video-sources` | Display, window, game capture (Windows), camera, image, color, text | Parity |
| 1 | ✅ `add-audio-engine` | Device/app capture, mixer, meters, monitoring, 6 tracks, core filters | Parity + per-app capture |
| 1 | ✅ `add-recording` | HW/SW encoders, MKV/MP4/fMP4, multi-track audio, remux, replay buffer | Parity + crash-safe fMP4 |
| 2 – Go live | ✅ `add-streaming-outputs` | RTMP(S), Enhanced RTMP, SRT, WHIP; service presets; auto-reconnect | Parity + WHIP/eRTMP |
| 2 | ✅ `add-multistream` | N simultaneous destinations; shared or per-destination encoders | Beyond OBS |
| 2 | ✅ `add-studio-mode-transitions` | Preview/Program, cut/fade/slide/stinger/shader transitions | Parity |
| 2 | ✅ `add-video-filters` | Color correction, LUT, chroma/luma key, crop/pad, blur, mask, sharpen, custom WGSL shaders | Parity + shaders built in |
| 2 | ✅ `add-media-sources` | Media files/playlists, image slideshow, VLC-like playlist | Parity |
| 3 – Pro | ✅ `add-multi-canvas` | Multiple canvases (vertical + horizontal), per-canvas outputs | Beyond OBS |
| 3 | ✅ `add-browser-source` | Out-of-process CEF, transparent overlays, custom CSS, audio | Parity + isolation |
| 3 | ✅ `add-remote-api` | obs-websocket v5 compatible server | Parity (ecosystem compatible) |
| 3 | ✅ `add-virtual-camera` | Virtual webcam output (Windows/macOS/Linux) | Parity |
| 3 | ✅ `add-hotkeys-macros` | Global hotkeys, macros and triggers (scene switch on window focus, timers…) | Beyond OBS (built-in Advanced Scene Switcher) |
| 3 | ✅ `add-stats-health` | Stats dock, network health, encoder overload hints (auto-quality fallback deferred) | Better |
| 3 | ✅ `simplify-first-run-and-go-live` | Purpose-based first run with device detection; two-step Go Live box (platform + key) | Simpler than OBS |
| 3 | ✅ `add-clips` | Game-aware replay buffer, clip hotkey, auto-clip (game integrations + audio hype), clip library with trim | Beyond OBS (Medal-style clipping) |
| 3 | ✅ `add-montage-editor` | Highlight scan of recordings, beat-synced auto-montage, single-track timeline editor, export | Beyond OBS |
| 3 | ✅ `refine-sound-controls` | Sound knobs (click mutes, drag sets volume, name opens settings), closable settings panel | Better UX |
| 3 | ✅ `add-voice-effects` | Pitch shift, robot, radio, echo, reverb; one-click voice presets; hear yourself | Beyond OBS |
| 4 – Ecosystem | `add-plugin-system` | WASM plugin host (sources, filters, docks, commands) | Better (sandboxed) |
| 4 | `add-ai-features` | Background blur and replacement (virtual background), face masks and avatars for camera privacy, auto captions, AI voice conversion | Beyond OBS |
| 4 | `add-iso-recording` | Per-source isolated recordings | Beyond OBS |
| 4 | `add-obs-import` | Import OBS scene collections & profiles | Migration path |
| 4 | ✅ `add-linux` | Linux backends (PipeWire, portals, V4L2) and an AppImage | Parity |
| 4 | `add-macos` | macOS backends (ScreenCaptureKit, AVFoundation) | Parity |
