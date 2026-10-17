# Design: Audio Engine

## Goals / Non-Goals
- Goals: glitch-free mixing with bounded latency, correct metering, no allocation or locks on the
  audio thread, sources that can disappear and come back, testable with synthetic signals.
- Non-Goals: VST3/CLAP hosting (separate change), audio from media files and browser sources
  (their own changes), surround output.

## Decisions

### Format and timing
- Internal format: 48 000 Hz, stereo, planar `f32`, blocks of 480 frames (10 ms).
- The audio thread runs on the engine clock with a drift-free 100 Hz frame clock (shared
  `FrameClock`, moved to `estream-core`).
- Each capture device writes into its own SPSC ring buffer (`rtrb`) from the device callback,
  resampling to 48 kHz there (`rubato`) when the device runs at another rate.
- Each block the mixer reads exactly 480 frames per source. Underruns are padded with silence and
  counted. If a source buffers more than 60 ms, the excess is dropped so latency stays bounded
  (and clock drift between devices can't accumulate).

### Per-source chain
`read → sync delay → filters → mono/balance → volume/mute → meters (post-fader) → tracks / monitor`

- Sync offset: a fixed delay line (0–2000 ms). Negative offsets delay all other sources
  instead (deferred; positive offsets only in this change).
- Volume is set in dB and smoothed per block (linear ramp) to avoid zipper noise.

### Meters
- Peak and RMS per channel, post-fader, written to atomics (`f32` bits in `AtomicU32`) every block.
- Peak hold (1.5 s) is computed in the UI, which polls at 30 Hz.

### Filters
Gain, noise gate (open/close thresholds, attack, hold, release), compressor (threshold, ratio,
attack, release, makeup), limiter (threshold, release; fast attack), noise suppression (RNNoise
via `nnnoiseless`, 480-sample frames, which matches our block size).

### Active sources
`global` sources (flag in `AudioConfig`) are always mixed. Sources referenced by visible items in
the active scene of the first canvas are mixed too. A source is mixed once even if it appears several times.

### Source types
`SourceType` gets an optional audio factory next to the video one, so a type can be video-only,
audio-only (microphone) or both (later: cameras with built-in microphones, media files).

### Monitoring
A cpal output stream on the chosen device (default if empty), fed from an SPSC ring, resampled
when the device does not run at 48 kHz.

### Per-application capture (Windows)
WASAPI process loopback (`ActivateAudioInterfaceAsync` with
`AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK`, include process tree), asking for 48 kHz float stereo.
Applications are chosen by executable name and re-attached when the app restarts.

## Implementation notes (learned while building)
- cpal reports glitches (`Xrun`) and automatic reroutes (`DeviceChanged`) through the stream
  error callback. Only other kinds mean the device is gone; WASAPI loopback reports an xrun
  whenever sound starts after silence, so treating every error as fatal broke desktop audio.
- Per-app capture asks WASAPI for 48 kHz float stereo with `AUTOCONVERTPCM`, so no resampling is
  needed; the target's root process is found by executable name (parent not the same app).
- wgpu's default uncaptured-error handler panics. Slint's `Surface::configure` occasionally fails
  ("failed to wait for GPU to come idle") while the video thread submits at startup; errors are
  now logged instead (the next configure succeeds).

## Risks
- Device clocks drift relative to the engine clock. Mitigation: bounded buffers (drop/pad). Proper
  adaptive resampling is a follow-up.
- Process loopback needs Windows 10 2004+. Older systems report the source as unavailable.
