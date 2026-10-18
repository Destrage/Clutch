# Tasks: add-montage-editor

## 0. Project context
- [x] 0.1 Narrow the editing non-goal and add `add-clips` / `add-montage-editor` to the roadmap in `openspec/project.md`

## 1. Media
- [ ] 1.1 Frame-accurate seeking decoder (hardware decode to GPU textures, look-ahead)
- [x] 1.2 Offline audio decode to 48 kHz f32 (mono mix for analysis, stereo for render)

## 2. Montage core (`crates/estream-montage`)
- [x] 2.1 Project model, timeline math, JSON save/load with migrations and relinking
- [x] 2.2 `MontageCommand`s with undo/redo
- [ ] 2.3 Highlight scan (markers + offline hype, merge, score)
- [x] 2.4 Beat detection (spectral flux, tempo, DP beat tracking)
- [ ] 2.5 Auto-assembly (allocation, anchoring, beat snapping, transitions, slow-mo, vertical crop, title)
- [ ] 2.6 Renderer: segment decoders, crop/scale, transition shaders, titles
- [ ] 2.7 Audio render: speed/time-stretch, music fades, ducking
- [ ] 2.8 Preview playback (real-time pacing, scrubbing, frame dropping)
- [ ] 2.9 Export (background, progress, cancel, presets)

## 3. Studio
- [ ] 3.1 Montages view: list, New montage wizard, Scan a recording
- [ ] 3.2 Timeline widget: blocks, transition chips, title lane, music waveform + beats, zoom, snapping, drag
- [ ] 3.3 Inspector and keyboard shortcuts; accessibility labels
- [ ] 3.4 Export dialog; add result to clip library

## 4. Verification
- [x] 4.1 Model tests: timeline durations with speeds and transitions; command undo round-trips
- [ ] 4.2 Highlight scan test on a synthetic recording with markers and a loud section
- [ ] 4.3 Beat detection tests (click tracks at known BPMs, test loops)
      (click tracks at 92–174 BPM with accents pass; real music loops still to add)
- [ ] 4.4 Auto-assembly snapshot tests (target length, beat-aligned cuts, dropped low scores)
- [ ] 4.5 Render golden tests (crossfade midpoint, dip, crop for 9:16, title)
- [ ] 4.6 Export test: ffprobe duration matches timeline, A/V in sync
- [ ] 4.7 In-app check: 10 CS2 clips + music → 60 s vertical montage, edit, export
