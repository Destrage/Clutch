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
  - it starts with about 90 popular PC games (only exe names known to be right), and grows over time;
  - users can add any exe as a game, or mark one as "not a game".
- **Detection:**
  - the foreground window is polled at the existing 250 ms focus probe;
  - an exe counts as a game if it is in the table, or if it covers the whole monitor and the
    user approved it the first time it was seen;
  - "game running" ends 10 s after the process exits.
- **Clip mode starts and stops the replay buffer on these edges.** Where it buffers:
  - the user's own replay buffer, if they started it by hand;
  - otherwise the studio canvas, if a program scene shows a game, window or display capture;
  - otherwise a small **clip engine** of its own (a second `Engine` with no collection file, so
    nothing about it is saved and the user's scenes are untouched): one 1080p60 canvas with a
    game capture of the fullscreen app, desktop audio and the default mic. With nothing else
    to composite, the cost is one capture and one encode.
  - After the game exits, the buffer stops once pending clips are saved (at most 15 s).
  - Each engine has its own clock: pending windows are kept on the clipper's clock and
    converted through `Instant`s to the buffer's engine clock when saved.

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
  - the median of the raw levels over those 300 ms must clear the threshold too, so a short
    click that lingers in the 400 ms window doesn't count;
  - levels during a hype run aren't added to the baseline, so a long shout fires once;
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
  - `refresh()` rescans the folder (when the Clips view opens and every 30 s while it is open), so clips added or removed outside eStream show up without a watcher dependency;
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
  settings (the JSON settings file), like the recording settings. They are not saved in the
  scene collection.
- **Palette actions:** `clips.open` (Clips…) and `clips.mode` (clip mode on/off). Saving keeps
  the existing `replay.save` id (Ctrl+Shift+R), now titled "Save clip", so macros that use it
  keep working.
- **Saving is asynchronous everywhere:** the clip hotkey, Ctrl+Shift+R, the palette, macros
  and the remote API all ask the clipper for a window; `Studio::save_replay` now returns
  `Result<(), String>` and the file is reported through `last_replay` (which raises
  `ReplayBufferSaved`) once written.
- **Automation:**
  - `Trigger::GameEvent { game: Option<String>, event: GameEventKind }` (an ace also matches
    multi-kill);
  - `Step::SaveClip { before: u32, after: u32 }` through `Host::save_clip`.

## UI
- **Clips view:** a dialog opened from the toolbar (film icon) or the palette. It has:
  - a thumbnail grid (newest first) with the length and a favorite star on each tile;
  - a filter bar (game, all/automatic/manual/favorites, search by title);
  - the selected clip: title (rename), favorite, events with their times, Play, Show in folder,
    Copy (the file, for pasting into Discord or a folder) and Delete (also the Del key);
  - trim: in/out sliders and fields, Fast (keyframe) or Exact mode, "Save as new clip" or
    "Replace", with progress and cancel;
  - a frame preview of the trim point being moved (decoded with the thumbnail code in the
    background, always heading for the latest point).
  - **Play opens the system's video player.** An in-app player was deferred: the media
    `player` publishes NV12 frames and an audio ring meant for the engine's audio thread, so
    playing in a dialog needs a CPU color path and its own audio output. The frame preview
    covers what trimming needs.
- **Settings → Clips:**
  - clip mode on/off, how much to keep in memory, the clip hotkey and its length, the longest
    merged clip, the saved sound, the clips folder and the storage limit;
  - one row per integration with its status ("Receiving events", "Connected · waiting for
    the game", …) and Connect/Disconnect;
  - the auto-clip rules of one game at a time (picked from a list): each event with on/off and
    seconds before/after;
  - hype for game audio and for the mic, and the sensitivity;
  - the "is this a game?" question for an unknown fullscreen program.
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
