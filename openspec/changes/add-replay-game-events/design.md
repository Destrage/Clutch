# Design: Game events from local replay files

## Existing pattern
Integrations in `estream-clips::integrations` split into a pure **tracker** (game data →
`EventKind`s for the local player, unit-tested) and a **source** that runs on its own thread and
sends `GameEvent { game, kind, at }` on a channel (`GsiServer`, `LeaguePoller`). Replay watchers
follow the same split.

## Replay watcher
```rust
pub trait ReplayFormat: Send + 'static {
    /// Game name as in games.json.
    fn game(&self) -> &'static str;
    /// Folder holding the replays, if this system has one.
    fn folder(&self) -> Option<PathBuf>;
    /// The replays (one path per match) currently in `folder`.
    fn replays(&self, folder: &Path) -> Vec<PathBuf>;
    /// Reads the local player's events from a finished replay.
    fn parse(&self, replay: &Path) -> Result<ParsedReplay, ReplayError>;
}

pub struct ParsedReplay { pub started: SystemTime, pub events: Vec<ReplayEvent> }
pub struct ReplayEvent { pub kind: EventKind, pub match_time: Duration }
```

`ReplayWatcher` (`estream-clips::integrations::replay`) owns one thread per watched game:
- It runs while the game is detected and the integration is connected, including when the user
  connects with the game already open, and stops when the game exits, after one last scan.
  Replays already in the folder when it starts are ignored.
- It polls the folder every 2 s. Polling is simpler and more robust than `ReadDirectoryChangesW`
  for folders written in bursts, and the cost is a directory listing.
- Rust's `std::fs` opens files with full sharing on Windows, so the game can keep writing,
  renaming and deleting.
- A replay that isn't finished (`NotReady`, I/O errors) is retried on the next poll, up to 30
  times. `Unsupported` replays are logged once per format description and skipped. A parser panic
  is caught at the thread boundary, logged, and that replay skipped.
- Parsing never runs on the video, audio or UI threads.

**Incremental parsing deferred.** No supported game writes its replay during a match (PUBG was
checked on real replays, below), so formats parse finished replays only. A per-replay `Cursor`
for growing files is added when a live format is confirmed (Fortnite, task 1.1).

**Times.** Each event's wall-clock time is `started + match_time`, turned into an `Instant`
(`instant_of`) and sent as a normal `GameEvent`. Events still covered by clip mode's buffer
(`now - at + pre <= buffer_seconds`) go through the auto-clip scheduler like any integration;
older ones are not clipped.

**Post-match markers.** Every event reaches the existing `Notice::Event` path, which writes a
`Marker` into the running recording's `<recording>.markers.json` at `at - recording start`. The
app keeps the last 8 recordings finished this session with their start and stop times; an event
that falls in one of those (the recording stopped before the replay appeared) is added to that
recording's marker file. Events before a recording started, or with no covering recording, add
no marker.

## Per-game notes
**PUBG (implemented).** Checked against 14 real replays (game version 2610.1.1.65):
- PUBG keeps the replay in memory and writes the whole folder `Saved\Demos\match.<…>\` within half
  a second at the moment the player's match ends (death or win); the folder's creation time equals
  `Timestamp + LengthInMS`. `bIsLive` is always false on disk. So PUBG is post-match only: events
  arrive seconds after the player is out, which still lets clip mode clip the last ~2 minutes.
- Every file is a little-endian `u32` length followed by the payload (NUL-padded).
- `PUBG.replayinfo` (JSON, written last): `Timestamp` (Unix ms, replay start), `LengthInMS`,
  `bIsLive`, `RecordUserNickName`, `GameVersion`.
- `events/kill<n>`, `events/DBNO<n>`: JSON `{id, group, time1}`, `time1` in ms from the start.
- `data/<id>`: JSON `{instigatorName, victimName, bDBNO, damageReason, …}` for that event.
- `data/ReplaySummary<n>`: every byte shifted down by one (`z!` is `{"`); the last one lists
  `playerStateSummaries` with each player's `ranking` (1 = win).
- The kill feed shows clan tags (`[MYR] Name`) while `RecordUserNickName` doesn't; names are
  compared with a leading `[tag] ` removed.
- Events: kill by the player → `Kill`; knock by the player → `Knock`; kill of the player →
  `Death`; ranking 1 → `MatchWin` at `LengthInMS`. Being knocked is not an event.

**Fortnite (implemented).** Checked against real replays from Season 12 (2020) and Chapter 5
Season 5 (2024), from the MIT-licensed FortniteReplayDecompressor test set:
- `%LOCALAPPDATA%\FortniteGame\Saved\Demos\*.replay`, an Unreal local replay: file info (magic
  `0x1CA2E27F`, start time as .NET ticks, live flag, AES key), then chunks `(type u32, size i32)`.
  Only the header (type 0: engine network version, branch) and events (type 3) are read; the
  game data is Oodle-compressed and skipped.
- Finished replays are encrypted (AES-256-ECB, PKCS#7) with a key that is only written once the
  match is over, so Fortnite is post-match only, like PUBG.
- `playerElim` events give eliminated and eliminator (account id, named bot or bot), whether it
  was a knock, and the time. `AthenaMatchStats` holds the recording player's own elimination
  count; `AthenaMatchTeamStats` their placement (1 → `MatchWin`).
- Nothing outside the game data names the recording player. They are taken from the account
  ids in `Saved\Logs\FortniteGame.log` when exactly one of them played the match (or one of
  several fits the stats); otherwise as the only player whose final eliminations equal the
  stats (or one fewer: a knock a teammate finished still counts in the stats). With no unique
  match, only the win is reported. On the samples: 7 kills + win (S12), and 7 kills, 9 knocks
  and a death for a 2nd-place team whose stats say 8 (C5S5).

**Rocket League (implemented).** Replays are saved at the end of a match (with the in-game
"Auto Save Replays" option) to `Documents\My Games\Rocket League\TAGame\Demos` (Steam) or
`DemosEpic` (Epic). `boxcars` parses the header only: goals with frame, scorer and team, the
final score, the recording player and its team (`PlayerStats`, or `PrimaryPlayerTeam` when the
name is missing). Times are `frame / RecordFPS`, the start is the file's write time minus
`NumFrames / RecordFPS`. Checked against four boxcars test replays from 2016 to 2025.

**Rainbow Six Siege.** One zstd-compressed `.rec` file per round in `MatchReplay\<match>\`, with
a `dissect` magic and version. Kills, headshots and plants are decoded as in the community
r6-dissect project. Rounds arrive as they finish, so events are about one round late: they are
post-match markers if a recording exists, and also live clips when the round file appears within
the replay buffer's length.

## Settings
Each game gets a "Use replay files" toggle in clip settings, off by default, with a one-line note
on what it reads and that the game's replay setting must be on. Enabling it never writes into
game folders. If a game's replay folder is missing, the integration shows "Turn on replays in
<game>".

## Failure handling
Parsers return `ReplayError::Unsupported { version }` for formats they don't know. The watcher
logs it once per game and version, stops parsing that replay, and keeps watching for the next.
Truncated or locked files are retried on the next poll. A panic inside a parser is caught at the
watcher thread boundary, logged, and disables that game's watcher for the session.

## Alternatives considered
- **Memory reading, injection, network capture:** rejected; anti-cheat can ban users for it.
- **Overwolf Game Events Provider:** covers these games but only for apps on the Overwolf platform.
- **Picture-based detection:** needed for games without replays (Apex, Valorant kills); a separate change.
