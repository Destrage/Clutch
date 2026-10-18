# Tasks: add-montage-editor

## 0. Project context
- [x] 0.1 Narrow the editing non-goal and add `add-clips` / `add-montage-editor` to the roadmap in `openspec/project.md`

## 1. Media
- [x] 1.1 Frame-accurate seeking decoder (software decode to NV12, uploaded like media sources; hardware decode later)
- [x] 1.2 Offline audio decode to 48 kHz f32 (mono mix for analysis, stereo for render)

## 2. Montage core (`crates/estream-montage`)
- [x] 2.1 Project model, timeline math, JSON save/load with migrations and relinking
- [x] 2.2 `MontageCommand`s with undo/redo
- [x] 2.3 Highlight scan (markers + offline hype, merge, score)
- [x] 2.4 Beat detection (spectral flux, tempo, DP beat tracking)
- [x] 2.5 Auto-assembly (allocation, anchoring, beat snapping, transitions, slow-mo, vertical crop, title)
- [x] 2.6 Renderer: segment decoders, crop/scale, transition shaders, titles
- [x] 2.7 Audio render: speed/time-stretch, music fades, ducking
- [x] 2.8 Preview playback (real-time pacing, scrubbing, frame dropping)
- [x] 2.9 Export (background, progress, cancel, presets)

## 3. Studio
- [x] 3.1 Montages view: list, New montage wizard, Scan a recording
- [x] 3.2 Timeline widget: blocks, transition chips, title lane, music waveform + beats, zoom, snapping, drag
- [x] 3.3 Inspector and keyboard shortcuts; accessibility labels
- [x] 3.4 Export dialog; add result to clip library

## 4. Verification
- [x] 4.1 Model tests: timeline durations with speeds and transitions; command undo round-trips
- [x] 4.2 Highlight scan test on a synthetic recording with markers and a loud section
- [ ] 4.3 Beat detection tests (click tracks at known BPMs, test loops)
      (click tracks at 92–174 BPM with accents pass; real music loops still to add)
- [x] 4.4 Auto-assembly snapshot tests (target length, beat-aligned cuts, dropped low scores)
- [x] 4.5 Render golden tests (crossfade midpoint, dip, crop for 9:16, title)
- [x] 4.6 Export test: ffprobe duration matches timeline, A/V in sync
- [ ] 4.7 In-app check: 10 CS2 clips + music → 60 s vertical montage, edit, export
      (done with 4 test clips and no music: wizard, 9:16 assembly with slow motion and whips,
      preview playback with the title, split + undo, 1080×1920 export into the clip library;
      music picked through the file dialog, beat snapping while trimming, and real CS2 clips
      are still to try)
