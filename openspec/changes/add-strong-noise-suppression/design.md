# Design: Strong noise suppression

## Context
- The current filter (`estream-audio/src/filters.rs`) runs `nnnoiseless` (RNNoise) on 480-sample
  blocks, the mixer's native 10 ms block at 48 kHz.
- DeepFilterNet (github.com/Rikorose/DeepFilterNet) is MIT or Apache-2.0. Its Rust library `libDF`
  contains the DSP (STFT, ERB features, the deep-filtering step) and runs the three ONNX
  sub-models (encoder, ERB decoder, DF decoder) with `tract`. DeepFilterNet3 also comes as a
  low-latency variant (`DeepFilterNet3_ll`).
- State of upstream (checked 2026-10-25): last commit 2024-10-17, last release v0.5.6 (2023-08);
  crates.io only has `deep_filter` 0.2.5 from 2022; `main` pins `tract` 0.21 and `ndarray` 0.15.
- Audio thread rules (project.md): no allocation, locks or blocking I/O in steady state.

## Decisions

### Vendor the inference path, not the crate
Depend on neither the stale crates.io crate nor a git pin of a dormant repo. Copy into
`estream-audio/src/denoise/` only what real-time inference needs (STFT/ISTFT, ERB and complex
feature normalisation, the deep-filter step, and the model runner), keep the upstream copyright
and license headers, and run it on a current `tract`. The model tarball is embedded with
`include_bytes!`. This is allowed by the license and keeps the dependency tree under our control.
If the spike shows current `tract` can't load the models, fall back to a pinned git dependency on
`libDF` and record why.

### Model choice: measured, low-latency preferred
The spike runs both `DeepFilterNet3` and `DeepFilterNet3_ll` on 48 kHz recordings with typing,
mouse clicks, fan and speech, and records CPU per 10 ms hop, delay and how much of the typing
remains. Pick `_ll` unless it is clearly worse on typing; lower delay matters more for lip sync
than a small quality gain.

### Where it runs
On the audio thread, inside the filter, if the spike shows a `tract` run is allocation-free after
warm-up (checked with a counting allocator in a test) and its p99 stays under 2 ms of the 10 ms
block. Otherwise it runs on its own `estream-denoise` thread fed by lock-free rings, adding one
block (10 ms) of delay and keeping the audio thread within the rules. Either way it is one thread
per filter, never a pool, so it can't spread onto the cores a game uses.

### Loading without gaps
Building the model takes too long for the audio thread. `filters::create` returns at once; a
helper thread builds the model and hands it over through a one-shot channel. Until it arrives, and
if loading fails (logged once), the filter runs Light, so speech is never left unprocessed. The
switch happens at a block boundary.

### Delay
The model's lookahead delays the voice by a fixed amount. This change does not delay everything
else to match: with the low-latency model the mic lands roughly 20–40 ms behind video, under the
usual lip-sync tolerance, and the filter row shows the exact figure. The Amount blend delays the
dry signal by the same amount, so the blend never smears or doubles.

### Mono
DeepFilterNet models are mono. Strong mode denoises `(l + r) / 2` once and writes it to both
channels. A stereo mic loses its stereo image, which nobody streams on purpose.

### Choice parameters
`FilterParam` gets an optional `choices: &'static [&'static str]`. A parameter with choices is
stored as its index (as a number, so settings stay a flat map of numbers) and drawn as a
segmented button row instead of a slider. The Noise suppression filter's `strength` is
`["Light", "Strong"]`, default 0.

### Settings compatibility
A missing `strength` reads as Light and a missing `amount` as 100%, so existing collections behave
exactly as before. Older versions ignore the unknown keys and run Light.

## Risks
- **Model weights license**: the repository says all its contents are MIT or Apache-2.0, and the
  weights live in it, but no file names the weights explicitly. Confirm before release (task 1.3).
- **CPU on weak laptops**: ~7–11× RNNoise per a 2026 third-party benchmark. Measured on the
  Windows 10 GTX 1650 Ti laptop before Strong becomes a preset default.
- **Music**: DeepFilterNet removes music almost entirely. The filter is offered on all sources
  like today, but presets only put it on mics.
