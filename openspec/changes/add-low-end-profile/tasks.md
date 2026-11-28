# Tasks: add-low-end-profile

## 1. Output size and fps
- [ ] 1.1 GPU scaling to the output size in the conversion pass (golden-image test)
- [ ] 1.2 Half-fps outputs: drop every other frame before the encoder, timestamps stay on the engine clock
- [ ] 1.3 Output size/fps settings for recording, replay and each destination; Settings rows
- [ ] 1.4 Preview frame rate cap while live or recording

## 2. Profile
- [ ] 2.1 Low-end PC profile values; Custom detection covers the new settings
- [ ] 2.2 Recommendation rules, with a test per hardware case
- [ ] 2.3 Suggestions list (noise suppression, background removal, browser fps), off by default
- [ ] 2.4 Lighter-profile alert action steps Gaming → Low-end PC

## 3. Testing aids
- [ ] 3.1 `CLUTCH_SIMULATE_HW` in the hardware probe and encoder list
- [ ] 3.2 `tools/low-end-sim.ps1` (job object: CPU rate, affinity, memory limit)

## 4. Check
- [ ] 4.1 Tests; record and stream 720p30 from a 1080p60 canvas and verify with ffprobe
- [ ] 4.2 Run under the simulator and on a real low-end laptop; compare game fps with and without Clutch live
