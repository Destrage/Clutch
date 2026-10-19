# Change: Voice effects

## Why
Streamers like to change their voice for characters, jokes or privacy without extra software.
Clutch already has an audio filter chain per source; voice effects fit in it as new filters, with
one-click presets so nobody has to understand DSP settings.

## What Changes
- **New audio filters** (real-time, no allocation on the audio thread):
  - **Pitch shift:** −12 to +12 semitones (deeper or higher voice), with a mix control.
  - **Robot:** ring-modulated, metallic voice, with a tone control.
  - **Radio:** narrow band-pass with light distortion (walkie-talkie, telephone).
  - **Echo:** delay with feedback and mix.
  - **Reverb:** room size, damping and mix (small room to cave).
- **Voice presets** in a source's audio settings, under "Voice": Off, Deeper, Higher, Robot,
  Radio, Cave, Echo. A preset adds or replaces one "Voice" filter group in the chain; the
  individual filters stay editable for fine tuning.
- **Hear yourself:** a toggle next to the presets that turns on monitoring for that source (the
  existing monitoring), so users can try voices before going live.
- Effects add at most ~20 ms of latency (pitch shift); the others add none beyond their design.

## Impact
- Affected specs: audio-engine (Audio Filters).
- Affected code: `crates/estream-audio/src/filters.rs` (new filter kinds and DSP),
  `apps/estream` (Voice presets and Hear yourself in the audio settings).
- Out of scope: AI voice conversion (speaking as a different person) — a later change.
