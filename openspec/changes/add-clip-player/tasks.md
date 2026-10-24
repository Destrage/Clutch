# Tasks: add-clip-player

## 1. Playback
- [ ] 1.1 Clip player state in the app: one non-looping `Player` for the selected clip or recording, dropped on selection change or view close
- [ ] 1.2 Upload new pictures through `YuvConverter` into a reused preview texture shown in the Clips view
- [ ] 1.3 Preview audio output on the monitoring device (default output when unset), with volume and mute, separate from the mixer

## 2. Controls
- [ ] 2.1 Play/pause, seek bar with elapsed and total time, volume and mute in the selected-clip pane
- [ ] 2.2 Event and marker ticks on the seek bar; clicking seeks 3 s before the event
- [ ] 2.3 "Set in" / "Set out" from the playback position; loop the trim range while trimming
- [ ] 2.4 Keyboard: Space, Left/Right, M
- [ ] 2.5 Play and double-click play in-app; "Open in default player" action

## 3. Verification
- [ ] 3.1 Unit tests: player lifecycle on selection change, trim-loop seeking, tick positions
- [ ] 3.2 Run `cargo test --workspace` and `cargo clippy --workspace --all-targets`
- [ ] 3.3 Play a 60 fps clip and a long recording: smooth video, audio in sync, no effect on a running stream's frame rate
