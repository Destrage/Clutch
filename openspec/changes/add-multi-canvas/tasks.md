# Tasks: add-multi-canvas

## 1. Engine and outputs
- [x] 1.1 Audio graph across all canvases (each source once)
- [x] 1.2 Sessions take a canvas (`RecordingSettings.canvas`); unknown ids fall back to the first canvas
- [x] 1.3 Recording: canvas choice and all canvases (one file per canvas, named after it)
- [x] 1.4 Replay buffer follows the recording canvas; stream destination canvas; destinations on
      different canvases never share an encoder

## 2. Studio
- [x] 2.1 Current canvas replaces "first canvas" throughout the app (preview, scenes, inspector,
      studio mode, transitions)
- [x] 2.2 Canvas tabs: select, add from presets (one undoable batch with a first scene), rename
      (double-click), remove (undoable; the last canvas stays)
- [x] 2.3 Settings: recording canvas (and "All canvases"), destination canvas; shown when the
      collection has more than one canvas

## 3. Verification
- [x] 3.1 Graph audio test (shared source mixed once)
- [x] 3.2 Recording all canvases (one file each at its own size); recording one canvas; unknown
      canvas falls back; separate encoders per canvas
- [x] 3.3 In-app check: add a vertical canvas from the tabs, switch back and forth

## Known issues / follow-ups
- Recording and destination canvases are stored by id in the global settings; with another scene
  collection loaded they fall back to the first canvas.
- The live panel and status bar don't show which canvas each destination streams.
