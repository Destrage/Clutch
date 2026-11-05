# Tasks: improve-live-multi-canvas

## 1. Scenes switch together
- [ ] 1.1 `Canvas::follow_scenes` (serde default true) and a command to set it (undoable)
- [ ] 1.2 Engine: a transition on one canvas also transitions every following canvas with a
      same-named scene (same transition); unit tests incl. a canvas with no match and one
      that opted out
- [ ] 1.3 Canvas tab menu: "Switch scenes with other canvases" toggle

## 2. Per-canvas audio
- [ ] 2.1 `AudioNode` carries the canvases it is heard on; `audio_graph` fills it (global =
      all canvases; leaving scene during a transition counts)
- [ ] 2.2 `AudioCore`: one track bus per canvas, allocated in `sync`; sinks registered per
      canvas; master meter per canvas; tests (source on one canvas is silent in the other's
      sink, shared source read once)
- [ ] 2.3 Outputs register their sink for their own canvas (stream, recording, replay buffer,
      record-all-canvases)
- [ ] 2.4 Mixer strips and master meter follow the edited canvas

## 3. Studio mode
- [ ] 3.1 Transition in studio mode switches every following canvas (via 1.2); Preview stays
      on the edited canvas only
- [ ] 3.2 Live deck: Studio mode switch; Preview and Program side by side; scene buttons pick
      the Preview; Transition and Cut buttons
- [ ] 3.3 Live deck: canvas picker when more than one canvas is live

## 4. Sound captured twice
- [ ] 4.1 Capture: expose the claimed processes and the one Desktop Audio leaves out
- [ ] 4.2 Mixer warning: two Application audio sources on the same app in one canvas's mix
- [ ] 4.3 Mixer warning: Desktop Audio carries an app another source captures (names both
      programs)

## 5. Go live
- [ ] 5.1 Per-destination canvas choice in the Go live window, saved to the destination; the
      all-destinations choice sets every row

## 6. Check
- [ ] 6.1 Build, clippy, tests
- [ ] 6.2 In-app check: two canvases, Discord only in Gameplay; switch to Be right back and
      check both canvases switch and the 1440p recording has no Discord audio
- [ ] 6.3 In-app check: Apex game capture with audio, Discord source, Desktop Audio on; the
      Desktop Audio strip shows the warning
- [ ] 6.4 In-app check by the user: live on Kick and Twitch (1080p) and YouTube (1440p);
      switch scenes from the deck in studio mode; each platform has one Discord and one game
