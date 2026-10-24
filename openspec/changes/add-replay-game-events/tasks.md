# Tasks: add-replay-game-events

## 1. Research
- [x] 1.1 PUBG: check real replays for when they're written (post-match: the folder appears when the player's match ends)
- [ ] 1.1b Fortnite: capture a replay folder mid-match and after the match (not installed on the dev machine)
- [x] 1.2 Document PUBG's layout, local-player identification and event fields in `design.md`

## 2. Framework
- [x] 2.1 Add `EventKind::Knock` and `EventKind::Goal` (and the matching trigger kinds) with labels and default clip windows
- [x] 2.2 Add the `ReplayFormat` trait, `ParsedReplay`, `ReplayEvent` and `ReplayError` (incremental `Cursor` deferred until a live format exists)
- [x] 2.3 Add `ReplayWatcher`: per-game polling thread, shared-mode reads, retries, panic isolation
- [x] 2.4 Send events through the existing auto-clip scheduler when clip mode's buffer still covers them
- [x] 2.5 Write events as markers into the running recording, or into a recording finished this session that covered them

## 3. Games
- [x] 3.1 PUBG: kills, knocks, deaths and wins
- [ ] 3.2 Fortnite: eliminations and knocks
- [ ] 3.3 Rocket League: goals via `boxcars` header parsing
- [ ] 3.4 Rainbow Six Siege: kills, headshots, plants per round

## 4. UI
- [x] 4.1 Per-game Connect toggle in clip settings (off by default) with the "Turn on replays" hint when the folder is missing (PUBG)
- [ ] 4.2 Show replay markers in the recordings view and save clips from them

## 5. Verification
- [x] 5.1 Parser unit tests against synthetic fixtures shaped like real replays (PUBG), plus an ignored test that reads the newest local replay
- [x] 5.2 Watcher tests: unfinished replay retried, truncated file, unsupported format, panicking parser, pre-existing replays ignored
- [x] 5.3 Run `cargo test --workspace` and `cargo clippy --workspace --all-targets`
- [ ] 5.4 Play one match of each game with recording on; confirm clips or markers line up with the moments

## 6. Hype
- [x] 6.1 Turn game-audio hype off while the running game reports events live (CS2, Dota 2, League); keep it on for PUBG, whose replay events come after the match and not in every mode
- [x] 6.2 Low game sensitivity: −16 dBFS loudness floor and a −34 dBFS baseline floor (tested against fight levels from real Valorant and PUBG sessions)
