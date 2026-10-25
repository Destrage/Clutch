## 1. Spike and license
- [ ] 1.1 Load `DeepFilterNet3` and `DeepFilterNet3_ll` with a current `tract`; enhance 48 kHz test recordings (typing, mouse clicks, fan, speech); record CPU per 10 ms hop (release build, p50/p99), delay and remaining typing level against RNNoise in design.md
- [ ] 1.2 Check a warmed-up model run for allocations (counting allocator); decide audio thread vs own thread and vendored vs pinned git dependency; record in design.md
- [ ] 1.3 Confirm the model weights are covered by MIT/Apache-2.0 (ask upstream if no file says so); add DeepFilterNet to the third-party notices

## 2. Engine
- [ ] 2.1 `estream-audio::denoise`: DeepFilterNet DSP and model runner (license headers kept), model embedded, no allocation per hop
- [ ] 2.2 Model built on a helper thread and handed over at a block boundary; Light runs until then and if loading fails (logged once)
- [ ] 2.3 Noise suppression `strength` (Light/Strong) and `amount` (dry signal delayed to match); Strong denoises a mono mix to both channels; filter reports its delay
- [ ] 2.4 Tests: synthetic keyboard clicks over speech-like signal are reduced more by Strong than Light; mono out; Amount 0% returns the input delayed by the filter's delay; missing keys read as Light / 100%; switching Light → Strong leaves no silent or unprocessed blocks

## 3. UI
- [ ] 3.1 `FilterParam` choices; settings draw them as a segmented button row; the Noise suppression row shows Strength, Amount and, for Strong, its delay
- [ ] 3.2 Tests: choice index round-trips through settings; a filter saved before this change loads as Light

## 4. Verify
- [ ] 4.1 `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`
- [ ] 4.2 Bench with Strong on the mic: audio thread CPU recorded, zero audio gaps over 10 minutes, game FPS unchanged
- [ ] 4.3 Windows 10 GTX 1650 Ti laptop: Strong runs without gaps; decide whether the mic preset defaults to Strong
- [ ] 4.4 Linux build in WSL
