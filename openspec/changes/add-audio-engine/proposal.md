# Change: Add audio engine (capture, mixer, meters, monitoring, filters)

## Why
A stream or recording without audio is unusable. eStream needs to capture microphones, desktop
audio and individual applications, mix them with per-source control, show accurate meters, let
the streamer monitor, and route sources to multiple tracks for recording/streaming later.

## What Changes
- New `estream-audio` crate: real-time mixer (48 kHz, stereo, 32-bit float, 10 ms blocks), lock-free
  ring buffers, resampling, meters, filters and device I/O (cpal / WASAPI).
- **audio-engine** capability (new):
  - Audio sources: input devices (microphones, line-in), output loopback ("desktop audio"),
    per-application capture (Windows 10 2004+ process loopback), and a test tone.
  - Per-source volume (dB), mute, balance, mono downmix, sync offset, monitoring mode
    (off / monitor only / monitor and output) and routing to 6 tracks.
  - Global audio sources (mixed in every scene) and scene audio (sources in the active scene).
  - Audio filters: gain, noise gate, compressor, limiter, noise suppression (RNNoise).
  - Peak and RMS meters with peak hold, published to the UI at 30 Hz.
  - Monitoring output to a selectable device.
  - Mixed tracks exposed to outputs (consumed by `add-recording`).
- Model: `SourceDef.audio` settings and a new `UpdateSourceAudio` command (undoable).
- Source types can provide a video factory, an audio factory, or both.
- Studio: mixer panel with meters, volume faders and mute; per-source audio settings and filter
  chain in the inspector; new collections get "Desktop Audio" and "Microphone" global sources.

## Impact
- Affected specs: `audio-engine` (new)
- Affected code: new `crates/estream-audio`; `estream-core` (shared status/clock types),
  `estream-project` (audio settings), `estream-engine` (audio thread, graph, commands),
  `estream-capture` (per-app audio on Windows), `apps/estream` (mixer UI)
