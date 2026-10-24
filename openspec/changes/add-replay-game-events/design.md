# Design: Game events from local replay files

## Existing pattern
Integrations in `estream-clips::integrations` split into a pure **tracker** (game data →
`EventKind`s for the local player, unit-tested) and a **source** that runs on its own thread and
sends `GameEvent { game, kind, at }` on a channel (`GsiServer`, `LeaguePoller`). Replay watchers
follow the same split.

## Replay watcher
```rust
pub trait ReplayFormat: Send {
    /// Game name as in games.json.
    fn game(&self) -> &'static str;
    /// Folder holding the replays (resolved per user / Steam library).
    fn folder(&self) -> Option<PathBuf>;
    /// Whether this path is (part of) a replay this format reads.
    fn matches(&self, path: &Path) -> bool;
    /// Reads events past `cursor` from a replay that may still be growing.
    fn read(&mut self, replay: &Path, cursor: &mut Cursor) -> Result<Vec<ReplayEvent>, ReplayError>;
}

pub struct ReplayEvent {
    pub kind: EventKind,
    /// Time into the match, from the replay's own clock.
    pub match_time: Duration,
}
```

`ReplayWatcher` owns one thread per enabled game:
- It polls the folder every 2 s while the game is running (`games.rs` detection) and once more
  after it exits. Polling is simpler and more robust than `ReadDirectoryChangesW` for files that
  are rewritten in place, and the cost is a directory listing.
- It opens files with full sharing (`FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE`), so
  the game can keep writing, renaming and deleting.
- It keeps a `Cursor` per replay (byte offset or chunk index plus parser state), so a growing file
  is parsed incrementally, not from the start each time.
- The thread runs at below-normal priority with an I/O budget: no more than one parse per poll per
  game, and it never runs on the video, audio or UI threads.

**Live vs post-match.** A replay that is still growing while the game runs is *live*: its events
are sent as `GameEvent`s with `at = now - (replay_time_now - match_time)` and go through the
existing auto-clip scheduler, which cuts from the replay buffer. Events found only after the
replay is finalized are *post-match*.

**Post-match markers.** The replay's start is converted to wall-clock time (replay metadata
timestamp, or the file's creation time as a fallback). If a recording covered that time, each
event becomes a `Marker` in `<recording>.markers.json` at `start_offset + match_time`. The
recordings view lists them, and the existing clip trimmer saves a clip from any marker. Without a
recording, post-match events are dropped, since the replay buffer has already moved on.

## Per-game notes
**PUBG.** Unreal Engine 4. Replays start when the player boards the plane. Each match is a folder
in `Saved\Demos` with `PUBG.replayinfo` (JSON: length, timestamp, map, mode, `bShouldKeep`) and
the recorder's stream, checkpoint and event chunks. The recorder writes directly to disk, so
chunks may appear during the match; **task 1.1 confirms this with a captured replay**. Kill and
knock ("groggy") events are expected among the replay events; the local player is identified
from the replay's recorder field.

**Fortnite.** Unreal Engine `.replay` file (local file streamer). Its info header has a live
flag and separate checkpoint and event arrays; eliminations are replay events that community
parsers (FortniteReplayDecompressor) decode, including knock vs. final elimination. Event chunks
may be encrypted per match with a key held in the file header, as those parsers handle. Whether
events are flushed during the match is confirmed in task 1.1.

**Rocket League.** Replays are saved at the end of a match (automatically when the in-game
auto-save option is on). `boxcars` parses the header only, which holds goals with frame, player
and team and stays stable across patches. Post-match only.

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
