# Tasks: add-replay-game-events

## 1. Research
- [ ] 1.1 Capture real replay folders for PUBG and Fortnite: one copy mid-match and one after the match; record which files exist mid-match and whether kill events are already present
- [ ] 1.2 Document each format's layout, local-player identification and event fields in `design.md`; mark each game live or post-match

## 2. Framework
- [ ] 2.1 Add `EventKind::Knock` and `EventKind::Goal` with labels and default clip windows
- [ ] 2.2 Add the `ReplayFormat` trait, `ReplayEvent`, `Cursor` and `ReplayError`
- [ ] 2.3 Add `ReplayWatcher`: per-game polling thread, shared-mode reads, incremental cursors, low priority, panic isolation
- [ ] 2.4 Send live events through the existing auto-clip scheduler
- [ ] 2.5 Align post-match events to recordings and write them as markers

## 3. Games
- [ ] 3.1 PUBG: kills and knocks
- [ ] 3.2 Fortnite: eliminations and knocks
- [ ] 3.3 Rocket League: goals via `boxcars` header parsing
- [ ] 3.4 Rainbow Six Siege: kills, headshots, plants per round

## 4. UI
- [ ] 4.1 Per-game "Use replay files" toggle in clip settings (off by default) with the "Turn on replays" hint when the folder is missing
- [ ] 4.2 Show replay markers in the recordings view and save clips from them

## 5. Verification
- [ ] 5.1 Parser unit tests against small captured replay fixtures (or synthetic ones where redistribution isn't allowed)
- [ ] 5.2 Watcher tests: growing file, truncated file, unsupported version, deleted mid-read
- [ ] 5.3 Run `cargo test --workspace` and `cargo clippy --workspace --all-targets`
- [ ] 5.4 Play one match of each game with recording on; confirm clips or markers line up with the moments
