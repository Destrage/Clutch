# Change: Stats and health

## Why
When a stream stutters, the streamer needs to know why, and fast. The cause could be:
- the GPU can't render in time;
- the encoder can't keep up;
- the network is dropping frames;
- the disk is filling up.

Raw numbers alone don't say what's wrong or what to do about it.
eStream should watch these signals continuously, show them with a little history, and **tell the
user** in plain words when something is going wrong, with a concrete fix.

## What Changes
- **stats-health** capability (new):
  - **Sampling:** once per second, for every canvas, encoder, live destination and the system.
    - **Canvases:** frame rate against target, render time, skipped frames.
    - **Encoders:** frames, frames dropped before encoding, bitrate, the encoder used.
    - **Destinations:** bitrate, network-dropped frames, send queue delay, reconnects, state.
    - **System:** CPU (eStream and total), memory, GPU 3D load, free disk space on the
      recording drive.
  - **History:** the last 5 minutes, drawn as sparklines.
  - **Health checks** with hysteresis (raised after a few seconds, cleared after a calm period):
    - rendering lag;
    - encoder overload;
    - network congestion or reconnecting;
    - sustained high CPU or GPU;
    - low disk space while recording (in GB, and in minutes left at the current bitrate).

    Each alert says what's happening, how bad it is, and what to try.
- **UI:**
  - The status bar shows a health dot (good / warning / problem) and the main alert.
  - A new alert raises a toast once.
  - A **Stats** dialog (status bar click, palette action) shows tiles with sparklines, a table of
    outputs and destinations, and the active alerts with advice.
- **Remote API:** `GetStats` reports real CPU use, memory use and free disk space, instead of
  0.

## Out of scope
- Per-source performance profiling (render cost per source).
- Long-term logging and charts beyond 5 minutes.
- Audio buffer underrun detection; the audio engine doesn't count underruns yet.
- Automatic quality fallback (lowering bitrate or resolution by itself when overloaded).
  Alerts suggest the fix for now; acting on it automatically is a later change.

## Impact
- Affected specs: `stats-health` (new), `remote-api` (`GetStats` values)
- Affected code:
  - new `crates/estream-health` (system sampler, monitor, rules);
  - `apps/estream` (sampling, status bar, dialog);
  - `estream-api` (stats fields via `Studio`).
