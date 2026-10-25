## 1. Measure
- [x] 1.1 Move the matched benchmark (`bench.ps1`, `run-pair.ps1`, `compare.ps1`) into `tools/bench/` with a README; record the 2026-10-25 baseline in it
- [x] 1.2 Per-stage video thread timings (tick, filters, render, convert, submit, deliver) as rolling averages and p95 in the stats panel and `GetStats` log, without allocating on the video thread
- [ ] 1.3 Take a memory breakdown of the bench scene (VMMap + D3D12 allocations) and note the largest items in this change's design

## 2. Capture without CPU waits
- [x] 2.1 Shared D3D11/D3D12 fence on the capture interop device; capture copies signal it instead of `flush_and_wait`
- [x] 2.2 Engine queue waits on the newest fence value of the slots a frame samples; slot reuse waits for the engine's reading frame to finish (non-blocking check, frame dropped if no slot is free); ring of 4, WGC pool of 3
- [x] 2.3 Fallback without D3D11.4 fences: wait for the copy as before, sleeping between polls instead of spinning (see design.md for why not a query handoff)
- [x] 2.4 Capture rate cap: `MinUpdateInterval` where supported plus early frame release, at half the video thread's measured frame interval
- [ ] 2.5 Tests: interop test for fence ordering; unit tests for the rate cap and busy slots; bench shows the capture thread no longer spinning

## 3. GPU frames to hardware encoders
- [x] 3.1 Spike: can wgpu 30 render to NV12 plane views on DX12? Record the answer in design.md
- [ ] 3.2 Shared R8/RG8 plane targets (D3D11-created, rendered by wgpu) and an NV12 texture ring per canvas, filled by a D3D11 plane-copy pass after a shared fence
- [ ] 3.3 FFmpeg D3D11VA device and frames context over the ring (`av_buffer_pool_init2`), slots returned when FFmpeg releases them
- [ ] 3.4 Video consumers receive GPU frames (slot + fence value) when their encoder takes D3D11 frames; NVENC and AMF first
- [ ] 3.5 QuickSync through a derived QSV frames context, CPU fallback when that fails
- [ ] 3.6 Automatic per-session fallback to the CPU readback (logged) on any setup or first-frame failure; hidden `performance.gpu_encode` switch
- [ ] 3.7 Tests: encode 120 frames of a known pattern through the GPU path and the CPU path and compare decoded output; fallback test with a forced failure

## 4. Encode once, fan out
- [ ] 4.1 `EncoderHub` with sessions keyed by canvas and resolved encoder settings; per-subscriber bounded packet queues
- [ ] 4.2 Keyframe requests from subscribers; new subscribers start on a keyframe
- [ ] 4.3 Recording, the replay/clip buffer and stream groups use the hub; multistream's own grouping moves onto it
- [ ] 4.4 Shared audio encoders keyed by codec, bitrate and track
- [ ] 4.5 Encoder session count in the stats panel
- [ ] 4.6 Tests: recording + clip buffer share one session; a stalled subscriber doesn't block others; a stream reconnect's keyframe reaches every subscriber

## 5. Skip unseen work
- [ ] 5.1 UI publishes preview demand and display refresh rate per canvas; the video thread renders the preview only when wanted, at most at that rate
- [ ] 5.2 Single-source pass-through when the program scene is one opaque, unfiltered, full-canvas item with no transition
- [ ] 5.3 Fused NV12 + sRGB preview conversion in one pass over the canvas
- [ ] 5.4 Golden-image tests: pass-through vs composite, fused vs separate conversion
- [ ] 5.5 Measure an 8-bit sRGB canvas against the 16-bit float canvas; record the result and decision in design.md

## 6. UI loop
- [ ] 6.1 Replace the 4 ms timer with frame-arrival wakes (limited to the display rate), event wakes and slow timers for chat, details and health
- [ ] 6.2 Minimized or hidden: only event wakes and slow timers run
- [ ] 6.3 Check every panel still updates (chat, meters, stats, outputs, scene thumbnails)

## 7. Memory
- [ ] 7.1 Encoder probing in a child process (`--probe-encoders`), cached in settings per GPU driver version
- [ ] 7.2 Re-run the memory breakdown; open tasks here for anything else over 50 MB

## 8. Verify
- [ ] 8.1 `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`
- [ ] 8.2 Matched bench against obs-studio meets the targets in proposal.md; measure the clip buffer's memory with clip mode on
- [ ] 8.3 Real setup (clip mode + recording + three destinations): 4 encoder sessions, zero dropped frames over 10 minutes
- [ ] 8.4 Windows 10 test machine (GTX 1650 Ti laptop): GPU path or a clean fallback, no stalls
