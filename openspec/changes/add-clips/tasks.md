# Tasks: add-clips

## 1. Media
- [x] 1.1 `PacketRing::save_range` + `Control::SaveRange`; raise the ring clamp to 600 s
- [x] 1.2 Trim: lossless (keyframe stream copy) and precise (re-encode of the kept range), temp file + rename
- [x] 1.3 Thumbnail extraction (one frame → 320 px JPEG)

## 2. Clips core (`crates/estream-clips`)
- [ ] 2.1 Clip window scheduler (merge, post-roll wait, max length)
- [ ] 2.2 Known-games table, foreground game detection, start/stop edges
- [ ] 2.3 Hype detector (live from meters, offline `scan`)
- [ ] 2.4 CS2 / Dota 2 GSI: Steam library lookup, config writer, token-checked HTTP listener, event diffing
- [ ] 2.5 League of Legends Live Client Data poller
- [ ] 2.6 Library: sidecars, index scan + watcher, delete, storage cap

## 3. Automation
- [ ] 3.1 `Trigger::GameEvent` and `Step::SaveClip` (project model, runner, macro editor)

## 4. Studio
- [ ] 4.1 Clip mode: auto replay buffer on game start/stop; hidden clip canvas when no scene shows the game
- [ ] 4.2 Clip hotkey, sound cue, Windows notification, palette actions
- [ ] 4.3 Auto-clip wiring: events and hype → scheduler → save → library
- [ ] 4.4 Recording markers sidecar while recording
- [ ] 4.5 Clips view: grid, filters, player with trim, rename/favorite/delete/reveal/copy
- [ ] 4.6 Settings → Clips (mode, hotkeys, rules table with Connect buttons, hype, storage cap)
- [ ] 4.7 Remote API: `ReplayBufferSaved` on clip save; `SaveReplayBuffer` saves a default clip

## 5. Verification
- [ ] 5.1 Scheduler unit tests (merge, cap, post-roll)
- [ ] 5.2 `save_range` test: window within bounds, starts on a keyframe, ffprobe duration
- [ ] 5.3 Hype detector tests with synthetic envelopes
- [ ] 5.4 GSI tests: recorded CS2/Dota payload sequences → expected events; bad token rejected
- [ ] 5.5 League poller test against a stub server with recorded event data
- [ ] 5.6 Trim tests (lossless and precise: duration, A/V sync, first frame)
- [ ] 5.7 Library tests: index scan, missing sidecar, storage cap keeps favorites/manual
- [ ] 5.8 In-app check: CS2 deathmatch with auto-clip on kills; hotkey clip in a non-integrated game
