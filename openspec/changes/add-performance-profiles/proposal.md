# Change: Performance profiles and "Optimize for this PC"

## Why
How much recording, the replay buffer and streaming cost a game depends on a handful of settings
spread over several Settings pages (encoder, its preset, rate control and quality, bitrate, the
replay buffer's length). First run picks a sensible encoder once, but afterwards nothing looks at
the PC again, and players don't know which of those settings matter for frame rate. Testers asked
for one place that sets everything up so gameplay isn't affected, with a few ready profiles.

## What Changes
- **performance** capability (new), shown as a **Performance** page at the top of Settings.
- **Profiles**, each a named set of values for the settings that cost the game something:
  - **Gaming (lightest):** hardware encoder, its fastest preset, constant quality ~23 for
    recordings, the platform's recommended bitrate for streams, a 60 s replay buffer in clip mode.
  - **Balanced:** hardware encoder, balanced preset, quality ~20, 120 s clip-mode buffer.
  - **Quality:** hardware encoder (HEVC or AV1 when the GPU has it, for recordings), quality
    preset, quality ~18, 120 s buffer. For strong PCs or recording-focused sessions.
  - **Custom:** shown when the user changes one of those settings by hand.
- **Optimize for this PC** picks the profile from what Clutch can measure:
  - the hardware encoders FFmpeg can open (NVENC, AMF, QuickSync); without one, Gaming with
    the software encoder's fastest preset and a warning that a game will lose frames;
  - CPU core count and memory (the replay buffer lives in memory);
  - the GPU vendor and model, and whether its encoder supports HEVC/AV1.
  It shows what it found and each setting it will change (before → after) and applies them only
  when the user confirms. Applying is one undoable step.
- **Scope of a profile:** the recording/replay encoder settings, the clip-mode buffer length,
  and every stream destination's encoder, preset and bitrate. It never changes canvases, scenes,
  sources, the recording folder, formats or stream keys.
- **Stay light while playing:** when a health check raises encoder overload or rendering lag
  while a game is running, the alert offers **Use a lighter profile** (one step down).
- First run applies **Optimize for this PC** instead of choosing only the encoder.

## Impact
- Affected specs: performance (new), studio-ui (Settings gets a Performance page).
- Affected code: `apps/estream/src` (profiles, detection, applying them through the command bus
  and settings), `apps/estream/ui/overlays.slint` (Performance page and the confirmation),
  `crates/estream-media` (encoder capability probing), `crates/estream-health` (the alert action).
- Builds on `split-settings-pages`.
