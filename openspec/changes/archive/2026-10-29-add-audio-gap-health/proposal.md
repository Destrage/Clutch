# Change: Show audio dropouts in Stats

## Why
Outputs now keep audio on the clock: audio that never reaches the encoder (a stalled mixer, a
full queue) is filled with silence instead of pulling later audio early. That keeps streams in
sync, but the dropout itself is silent and invisible. Streamers should be able to see that it
happened, how much, and what to do about it, the same way they see dropped video frames.

## What Changes
- Each output counts the audio it had to fill with silence (milliseconds).
- The Stats page's output table gets an **Audio gaps** column with the total for each output.
- A health alert ("Audio dropped out on the recording / stream") appears as soon as an output
  fills a gap, and clears after 10 quiet seconds. It is a warning, or a problem when more than
  a second was missing in the last 10 seconds. The advice: close CPU-heavy apps, use a hardware
  encoder or lower the encoder preset.
- Tests: the realign decision, and an encoder test that silence moves later audio to the right
  timestamps.

## Impact
- Affected specs: stats-health (new requirement).
- Affected code: `estream-media` (`OutputStats::audio_gap_ms`, realign function and tests),
  `estream-health` (counter, column data, alert), `apps/estream` (sample and table column).
