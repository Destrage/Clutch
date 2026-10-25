## 1. Spike and license
- [x] 1.1 Load `DeepFilterNet3` and `DeepFilterNet3_ll` with `tract` (0.21.4: libDF does not build on current tract); enhance 48 kHz TTS speech with synthetic keyboard typing; record CPU per 10 ms hop (release build, p50/p99), delay and remaining typing level against RNNoise in design.md
- [x] 1.2 Check a warmed-up model run for allocations (counting allocator); decide audio thread vs own thread and vendored vs pinned git dependency; record in design.md
- [ ] 1.3 Confirm the model weights are covered by MIT/Apache-2.0 (ask upstream if no file says so); add DeepFilterNet to the third-party notices

## 2. Engine
- [x] 2.1 `estream-denoise` crate (own crate so the vendored code keeps upstream style and edition 2021): DeepFilterNet DSP and model runner vendored (license headers kept), `tract-*` pinned `=0.21.4`, low-latency model embedded
- [x] 2.2 One `estream-denoise` thread per Strong filter: builds the model, SPSC rings of 10 ms blocks, 20 ms fixed delay; Light output until the first block, for missed blocks and if loading fails (logged once); no allocation on the audio thread
- [ ] 2.3 Port the runner to current `tract`; re-measure CPU with the spike harness and keep the faster one
- [x] 2.4 Noise suppression `strength` (Light/Strong) and `amount` (dry signal delayed to match); Strong denoises a mono mix to both channels; its delay is `STRONG_DELAY_MS` (20 ms) for the settings
- [x] 2.5 Tests: synthetic keyboard clicks over faint noise are reduced more by Strong than Light (speech kept: spike, task 1.1); mono out; Amount 0% returns the input delayed by the filter's delay; missing keys read as Light / 100%; switching Light → Strong leaves no silent or unprocessed blocks

## 3. UI
- [ ] 3.1 `FilterParam` choices; settings draw them as a segmented button row; the Noise suppression row shows Strength, Amount and, for Strong, its delay
- [ ] 3.2 Tests: choice index round-trips through settings; a filter saved before this change loads as Light

## 4. Verify
- [ ] 4.1 `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`
- [ ] 4.2 Bench with Strong on the mic: audio thread CPU recorded, zero audio gaps over 10 minutes, game FPS unchanged
- [ ] 4.3 Windows 10 GTX 1650 Ti laptop: Strong runs without gaps; decide whether the mic preset defaults to Strong
- [ ] 4.4 Linux build in WSL
