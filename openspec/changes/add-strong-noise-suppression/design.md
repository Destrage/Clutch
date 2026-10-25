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

## Spike result (2026-10-25)
39 s of Windows TTS speech (48 kHz, peak -6 dBFS) with synthetic mechanical-keyboard typing
(press transient + key resonance + release click, bursts of 3–9 keys, peak -14 dBFS) over both
speech and pauses. Each denoiser runs hop by hop (480 samples), single thread, release build,
`tract` 0.21.4 (upstream's lock file), on the dev machine. Delay found by cross-correlation with
the clean speech.

| | RNNoise (Light) | DFN3 | DFN3 low-latency |
|---|---|---|---|
| CPU per 10 ms hop, mean / p99 | 50 / 497 µs | 392 / 789 µs | 1199 / 2181 µs |
| Share of one core | 0.5% | 3.9% | 12.0% |
| Allocations per hop after warm-up | 0 | ~1350 | ~1790 |
| Delay | 10 ms | 30 ms | 10 ms |
| Typing in pauses, reduction | 0.5 dB | 21.6 dB | 41.0 dB |
| Click-free speech level vs clean | -0.2 dB | -2.0 dB | -0.5 dB |
| SI-SDR vs clean speech (input 18.6 dB) | 10.7 dB | 5.3 dB | 13.5 dB |
| Model size (tar.gz) | built in | 8.0 MB | 36.4 MB |
| Model build time | — | 260 ms | 448 ms |

- RNNoise leaves typing essentially untouched, which matches what streamers hear.
- The low-latency model is better on every quality measure and has a third of the delay, at
  three times the CPU of the standard model and 4.5× its size.
- Synthetic clicks and TTS speech only approximate a real mic; a recording of the user typing and
  talking is the confirmation (output WAVs kept from the spike for listening).
- Upstream `libDF` does not build against current `tract` 0.21.x (17 errors; tract breaks API
  within a minor) and has 25 errors against 0.23. It builds with all `tract-*` crates pinned to
  `=0.21.4`.

## Decisions

### Vendor the inference path, not the crate
Depend on neither the stale crates.io crate nor a git pin of a dormant repo. Copy into
`estream-audio/src/denoise/` only what real-time inference needs (STFT/ISTFT, ERB and complex
feature normalisation, the deep-filter step, and the model runner), keep the upstream copyright
and license headers. The model tarball is embedded with `include_bytes!`. This is allowed by the
license and keeps the dependency tree under our control.

### Model choice: low-latency model
Strong uses `DeepFilterNet3_ll`: it removes about twice as much typing (in dB) as the standard
model, keeps the voice level, and adds 10 ms instead of 30 ms. Its cost is about 12% of one core,
on one thread. On a 6-core or larger CPU that is under 2% of the whole processor, roughly what the
spin removed from the audio thread in improve-pipeline-performance gave back. Shipping cost is
36 MB more in the installer. Revisit after the `tract` 0.23 port (task 2.1) re-measures CPU.

### Vendored, `tract` pinned, then ported
Vendor the inference code first with `tract-*` pinned to `=0.21.4` (known to build and measured),
then port it to current `tract` within this change and keep whichever is faster.

### Where it runs: its own thread
A `tract` run allocates ~1800 times per hop, so it can't run on the audio thread. Each Strong
filter owns one `estream-denoise` thread fed by lock-free SPSC rings of 10 ms blocks. The filter
on the audio thread pushes the input block and pops the block finished one hop earlier, which adds
one block (10 ms) to the model's 10 ms: 20 ms in total. If the thread misses a block, the filter
outputs the Light-processed block for that hop (RNNoise keeps running alongside for this), so the
audio thread never waits. One thread per filter, never a pool, so it can't spread onto the cores a
game uses.

### Loading without gaps
Building the model takes ~450 ms, far too long for the audio thread. `filters::create` returns at
once and the filter's `estream-denoise` thread builds the model itself. Until its first block
comes back, and if loading fails (logged once), the filter outputs Light, so speech is never left
unprocessed. The switch happens at a block boundary.

### Delay
Strong delays the voice by a fixed 20 ms (10 ms model, 10 ms thread handoff). This change does
not delay everything else to match: 20 ms is under the usual lip-sync tolerance, and the filter
row shows the figure. The Amount blend delays the
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
- **CPU on weak laptops**: measured 24× RNNoise (12% of one core) on the dev machine. Measured
  on the Windows 10 GTX 1650 Ti laptop before Strong becomes a preset default.
- **Music**: DeepFilterNet removes music almost entirely. The filter is offered on all sources
  like today, but presets only put it on mics.

## Spike harness
`spike/main.rs` (with `spike/Cargo.toml.txt`) is the comparison above. It builds outside the
workspace against an unpacked DeepFilterNet checkout (`libDF` path dependency, upstream
`Cargo.lock` so `tract-*` resolve to 0.21.4) and takes the audio directory (`speech.wav`, 48 kHz
mono 16-bit) and models directory as arguments. It writes `noisy.wav` and one `out_*.wav` per
denoiser for listening.
