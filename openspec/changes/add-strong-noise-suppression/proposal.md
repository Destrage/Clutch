# Change: Strong noise suppression for typing and clicks

## Why
Noise suppression uses RNNoise. It removes steady noise (fans, hum, room tone) well, but short,
sharp sounds get through: keyboard typing and mouse clicks are clearly audible on stream while the
streamer plays or chats. Gamers type and click constantly, so this is the noise that matters most.

DeepFilterNet3 is an open-source speech enhancement model (MIT or Apache-2.0) built for full-band
48 kHz audio. Published tests show it removing keyboard noise that RNNoise leaves in. It runs on
one CPU core well below real time and never touches the GPU, so games and the encoders keep their
GPU time.

## What Changes
- The Noise suppression filter gets a **Strength** choice:
  - **Light**: RNNoise, as today. Default, and what existing filters keep.
  - **Strong**: DeepFilterNet3, for typing, clicks and other non-steady noise.
- An **Amount** slider (0–100%, default 100%) blends the cleaned voice with the original, for
  streamers who find full suppression sounds processed. It works in both strengths.
- Strong mode processes the mic as mono (a mix of both channels) and outputs it on both channels.
  Mics are mono in practice.
- Strong mode delays that source by a fixed 20 ms (10 ms model, 10 ms thread handoff). The filter
  row shows it. The model loads off the audio thread; Light runs until it is ready, so
  switching never causes a gap.
- Filter settings gain a choice control (a dropdown) alongside number fields, so a filter
  parameter can be a small set of named options.

## Impact
- Affected specs: audio-engine (Audio Filters).
- Affected code: `estream-audio` (filters, new `denoise` module with the DeepFilterNet DSP and
  model runner, on one `estream-denoise` thread per filter), `apps/estream` (choice
  parameter control in the filter settings, filter delay label), third-party notices.
- New dependency: an ONNX runtime written in Rust (`tract`), plus the DeepFilterNet3
  low-latency model embedded in the binary (36 MB). No new DLLs.
- CPU: Strong measured 10.6% of one core per mic on its own thread, never the GPU; to be
  measured on the Windows 10 laptop. Light is unchanged.
- Settings: filters saved without `strength` load as Light; an older Clutch ignores the new keys
  and runs Light.
