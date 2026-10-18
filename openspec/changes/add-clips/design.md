# Design: add-clips

## Building on the replay buffer
Clips reuse the existing `ReplayBuffer` output (`estream-media/src/output.rs`). Nothing new runs
on the video or audio threads.

- **`PacketRing::save_range(from, to, path, container)`:**
  - writes the packets from the last video keyframe at or before `from` up to `to`, with
    timestamps rebased as in `save`;
  - `save` becomes `save_range(oldest, newest)`.
- **`Control::SaveRange { from, to, reply }`** sits next to `SaveReplay`. Saving runs on the
  output's thread as today: it clones the packets and muxes them.
- **Ring length:** in clip mode the ring length is `max(clip_seconds) + post_seconds + 5 s`. The
  clamp on the ring length rises from 300 s to 600 s.

### Clip window scheduler (`estream_clips::Scheduler`)
- A request `{ at, pre, post, reason }` asks for the window `[at - pre, at + post]`.
- Pending windows that overlap or touch merge, and their reasons are combined. For example, a
  kill and then a double kill 4 s later give one clip with two events.
- A window is saved once `now >= end + 0.5 s`, so the post-roll is already in the ring.
- A merged window is capped at `max_clip_seconds` (default 90). Past that, a new clip starts.
- Times are engine-clock seconds, the same clock as the packet timestamps.
- The scheduler is pure logic, ticked from the UI tick, and unit tested.

## Game detection (`estream_clips::games`)
- **Known-games table** (embedded JSON):
  - each entry has an exe name, display name, an optional Steam app id, and the integration it
    supports;
  - it starts with the top ~200 PC games;
  - users can add any exe as a game, or mark one as "not a game".
- **Detection:**
  - the foreground window is polled at the existing 250 ms focus probe;
  - an exe counts as a game if it is in the table, or if it covers the whole monitor and the
    user approved it the first time it was seen;
  - "game running" ends 10 s after the process exits.
- **Clip mode starts and stops the replay buffer on these edges.** If the program scenes have no
  game, window or display capture showing that window, it uses a hidden **clip canvas**:
  - the canvas follows the game's resolution, capped at the settings value (default 1080p60);
  - it holds a game capture of the window plus desktop audio and the default mic;
  - with nothing to composite, the cost is one capture and one encode.

## Game integrations (`estream_clips::integrations`)
All integrations turn game data into `GameEvent { game, kind, at, detail }`, where `kind` is one
of `Kill`, `MultiKill(n)`, `Ace`, `Death`, `RoundWin`, `MatchWin`, `Objective(String)`. They run
on the existing tokio runtime (I/O only).

- **CS2 / Dota 2 (Game State Integration):**
  - a local HTTP listener on `127.0.0.1:<port>`, port 3091 by default (configurable);
  - **"Connect"** finds the game folder through Steam (`libraryfolders.vdf` →
    `appmanifest_<id>.acf`). It then writes `gamestate_integration_estream.cfg` with the URI, a
    random auth token and the data blocks needed (`player_id`, `player_state`,
    `player_match_stats`, `round`, `map`);
  - payloads without the token are rejected;
  - events come from differences between payloads:
    - kills: `round_kills` / `kills` go up;
    - ace: `round_kills == 5`;
    - round win: `round.win_team == player.team`;
  - only the local player (`provider.steamid == player.steamid`) counts. Spectating doesn't.
- **League of Legends (Live Client Data API):**
  - polls `https://127.0.0.1:2999/liveclientdata/eventdata` every 500 ms while `League of
    Legends.exe` is running (self-signed certificate, accepted for this loopback host only);
  - `ChampionKill` events where the killer is the active player count as kills;
  - `Multikill`, `Ace`, `DragonKill`, `BaronKill` and `GameEnd(Win)` map to the matching
    event kinds.
- The **game clock** inside the event (the game's own time) is ignored. The event time is when it
  was received on the engine clock. GSI and polling add under 1 s of delay, which the default
  pre-roll (10 s) covers.

Default auto-clip windows (each one editable per event):

| Event | Pre | Post |
|---|---|---|
| Kill | 10 s | 3 s |
| Multi-kill / Ace | 15 s | 4 s |
| Round / match win | 15 s | 5 s |
| Objective | 12 s | 3 s |
| Hype (audio) | 12 s | 4 s |

## Audio hype detector (`estream_clips::hype`)
- **Input:** loudness of the chosen sources (default: desktop audio and mic), read from the
  existing per-source `Meter` RMS values on each UI tick (~60 Hz). The audio thread is not
  touched.
- **Signals:**
  - short-term loudness: 400 ms window;
  - baseline: 60 s median, so long loud fights don't keep raising the baseline;
  - fire when `short - baseline > threshold` for 300 ms or more, with a 20 s cooldown;
  - threshold by sensitivity: 18 / 14 / 10 LU (low / medium / high);
  - the mic counts only if it is above −30 dBFS, which ignores keyboard noise.
- **Offline use:** the same detector also runs on audio decoded from a file
  (`hype::scan(samples)`). The montage editor uses that to find highlights in long recordings.
- Tested with synthetic envelopes:
  - a steady level fires nothing;
  - a +20 dB step fires once;
  - repeated steps inside the cooldown fire once.

## Library (`estream_clips::library`)
- **Folder:** `<recording folder>/Clips/<Game>/<date> <time> <reason>.mp4`. Clips are MP4 by
  default for easy sharing, which is safe because a clip is written in one go.
- **Sidecar** `<clip>.json`:

```json
{ "version": 1, "game": "Counter-Strike 2", "created": "2026-10-18T20:14:03Z",
  "trigger": "auto", "events": [{ "kind": "multi_kill", "n": 2, "at": 11.2 }],
  "duration": 17.4, "favorite": false, "title": null }
```

- **Thumbnail:** `<clip>.jpg` (320 px) taken from the frame at the first event, or the middle
  of the clip.
- **Index:**
  - built in memory by scanning sidecars at startup, on a background thread;
  - a folder watcher picks up changes;
  - a clip file without a sidecar is still listed, as manual with no game;
  - deleting a clip removes the clip, its sidecar and its thumbnail.
- **Storage cap:** after each save, if the auto-clip total is over the cap, the oldest auto
  clips that aren't favorites are deleted until the total is under.

## Trim (`estream_media::trim`)
- **Lossless:** stream copy from the keyframe at or before `in` up to `out`.
- **Precise:** decode and re-encode the kept range (same codec, bitrate matched to the source,
  hardware encoder when available), with audio cut to the sample and progress shown.
  - A "smart cut" (re-encode only up to the next keyframe, stream-copy the rest) was considered
    and dropped: the two parts need identical encoder parameters, which breaks whenever the
    encoder differs from the one that recorded the clip. Clips are at most a few minutes, so
    re-encoding the whole range takes seconds with a hardware encoder.
- Both write to a temp file and then rename it, so the original is only replaced after success.

## Commands and settings
- **Settings:** clip mode, hotkeys, lengths, auto-clip rules and the storage cap are app
  settings (TOML), like the recording settings. They are not saved in the scene collection.
- **Palette actions:** `clips.save`, `clips.open`, `clips.toggle_mode`.
- **Automation:**
  - `Trigger::GameEvent { game: Option<String>, kind }`;
  - `Step::SaveClip { pre: u32, post: u32 }`.
  - Clip saves go through the same app surface (`Studio`) as `save_replay`.

## UI
- **Clips view:** a toolbar toggle beside the scene editor. It has:
  - a thumbnail grid with lazy loading, grouped by day;
  - a filter bar (game, auto/manual, favorites, search by title);
  - a player panel built on the existing media `player`, with trim handles and keyboard
    shortcuts (I/O for in/out, Space to play, Del to delete).
- **Settings → Clips:**
  - clip mode on/off, buffer length, quality preset;
  - hotkeys;
  - the auto-clip rules table (game × event, each with on/off and pre/post), with a Connect
    button for each integration;
  - hype sensitivity and sources;
  - the storage cap.
- **Status bar:** a clip-mode badge showing the detected game, plus a counter of clips saved
  this session.

## Risks
- **GSI config files:**
  - writing into a game folder needs the user's OK (the Connect button);
  - game updates can remove the file, so the integration shows "not connected" again when
    no payload arrives within 30 s of the game starting.
- **The League client** uses a self-signed certificate. TLS checks are skipped only for
  `127.0.0.1:2999`.
- **False positives from hype detection** are expected, which is why it's opt-in and has a
  storage cap.
- **Anti-cheat:** all integrations use official, documented local APIs, and the capture is the
  existing WGC capture, so nothing is injected into the game.
