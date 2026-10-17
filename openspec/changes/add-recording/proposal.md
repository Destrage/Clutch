# Change: Add recording (encoders, muxers, replay buffer)

## Why
eStream can compose video and mix audio, but nothing leaves the app yet. Recording is the first
output and the foundation for streaming: it needs GPU→encoder frame delivery, hardware encoding,
audio encoding for multiple tracks, and muxing with correct A/V sync.

## What Changes
- New `estream-media` crate built on FFmpeg 9 (LGPL shared build, dynamically linked):
  - Video encoders: NVENC, AMF, QuickSync (H.264 / HEVC / AV1) with software fallbacks
    (OpenH264, Kvazaar, SVT-AV1); automatic best-encoder selection; CBR / VBR / constant quality.
  - Audio encoders: AAC (default) or Opus, one encoder per enabled track.
  - Muxers: MKV (default, crash-safe), fragmented MP4 (crash-safe), MP4, MOV.
  - Optional automatic remux to MP4 when an MKV recording stops.
- **recording** capability (new):
  - Start/stop recording of a canvas with all enabled audio tracks.
  - Replay buffer: keep the last N seconds in memory and save them on demand.
  - Timestamps on the engine clock, so audio and video stay in sync.
  - Statistics: duration, file size, bitrate, encoder lag and dropped frames.
- Engine: canvases can feed encoders with NV12 frames read back asynchronously (no stalls on the
  video thread); audio tracks reach encoders through the existing `TrackSink`.
- Studio: working Record button with a timer, replay buffer controls, and a Settings dialog
  (output folder, format, encoder, quality, audio tracks, replay length).

## Impact
- Affected specs: `recording` (new)
- Affected code: new `crates/estream-media`; `estream-engine` (encoder feed), `estream-gpu`
  (NV12 readback), `apps/estream` (record UI, settings dialog)
- External: FFmpeg 9 LGPL shared libraries (bundled with the app), LLVM/Clang to build bindings
