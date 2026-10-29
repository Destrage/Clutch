# Change: Clips (game-aware replay, clip library, auto-clip)

## Why
Most gamers don't stream. They want to grab the moment that just happened. "Clip first, think later"
is what they expect:
- press a key after something cool happens and the last 30 s is saved;
- or let the app notice the kill or the win and save it without a key press at all.

eStream already has the core of this: a replay buffer (an in-memory packet ring, saved on
Ctrl+Shift+R). It still isn't a clipping app, because:
- the buffer has to be started by hand;
- saved replays are just files in the recording folder, with no way to browse, trim or tag them;
- nothing clips on its own.

This change makes the replay buffer the base of a proper clipping workflow. It also produces the
library of tagged clips that the montage editor (`add-montage-editor`) builds on.

## What Changes
- **clips** capability (new):
  - **Clip mode:**
    - when enabled, the replay buffer starts by itself when a game is running (from a known-games
      list, or any fullscreen app the user marks as a game) and stops a short while after it closes;
    - the buffer length defaults to 2 minutes;
    - it doesn't need a scene: if the active scene has no game capture, clip mode captures the
      game directly.
  - **Manual clip:** a global hotkey (default F8, rebindable; Alt+F10 is avoided because a common GPU overlay already uses it) saves the last *N* seconds
    (default 30, set per hotkey). The buffer keeps running, a sound cue confirms the save, and
    the clip is tagged with the game.
  - **Time-window saves:** a clip can include time *after* the request (e.g. 10 s before a kill
    and 3 s after). Requests that overlap merge into one clip, up to a maximum length.
  - **Auto-clip** (opt in, per game and per event):
    - **Game integrations** use the games' own local event APIs:
      - Counter-Strike 2 and Dota 2 through Game State Integration;
      - League of Legends through the Live Client Data API.
      - Events: kill, multi-kill, ace/team wipe, round or match win, and objective (League
        dragon/baron).
      - Setup is one click: "Connect CS2" writes the GSI config file into the game folder,
        found through the Steam library.
    - **Audio hype detection** works in any game. It saves a clip when the short-term loudness of
      chosen sources (game audio, mic) jumps far above their recent baseline. Sensitivity has
      three levels, and there's a cooldown.
  - **Clip library:**
    - clips are saved in a Clips folder, with a JSON sidecar per clip holding the game, time,
      trigger and events (with timestamps inside the clip), duration, favorite flag and a
      thumbnail;
    - a **Clips** view shows clips in a grid with thumbnails, filters (game, manual/auto,
      favorites) and sorting;
    - a clip can be played, renamed, favorited, deleted, revealed in the file manager or copied
      to the clipboard as a file.
  - **Trim:**
    - lossless trim cuts at keyframes (stream copy);
    - precise trim re-encodes the kept range, so it starts on the exact frame;
    - either can be saved as a new clip or replace the original.
  - **Storage cap:** auto-clips beyond a size limit (default 20 GB) are deleted oldest first.
    Favorites and manual clips are never deleted automatically.
  - **Recording markers:** game events and hype peaks that happen during a normal recording are
    written to a sidecar marker file next to it. The montage editor uses them.
- **automation** capability:
  - a new **game event** trigger (e.g. "CS2: kill");
  - a new **save clip** step (seconds before and after).
- **Remote API:**
  - clip saves also raise `ReplayBufferSaved` so existing tools keep working;
  - `SaveReplayBuffer` saves a clip with the default length.

## Out of scope
- In-game overlay notifications. They need injection into the game process. A sound cue is
  used instead. (Windows notifications were considered and dropped: Windows holds them back
  while a fullscreen game runs, which is exactly when clips are saved.)
- An in-app clip player. Play opens the system's video player; the Clips view shows the exact
  frame at each trim point.
- Detecting kills on screen with an AI model (for games without an API, like Valorant and
  Fortnite). These games get audio hype only for now. The detector interface leaves room for it.
- Uploading or sharing to a cloud or a social feed. eStream is local-first. "Copy file" and
  "reveal" make posting easy.
- Editing beyond trim. That belongs to `add-montage-editor`.

## Impact
- Affected specs: `clips` (new), `automation` (game event trigger, save clip step)
- Affected code:
  - `estream-media`: `PacketRing::save_range`, trim (stream copy and precise re-encode), thumbnails;
  - new `crates/estream-clips`:
    - game detection;
    - game integrations (GSI HTTP listener, League poller);
    - the hype detector;
    - the clip window scheduler;
    - library index and storage cap;
  - `estream-project` (`Trigger::GameEvent`, `Step::SaveClip`);
  - `estream-automation`;
  - `apps/estream` (Clips view, settings, hotkey, sound cue);
  - `estream-api` (`ReplayBufferSaved` on clip).
- NVENC session use is the same as today's replay buffer (one encoder).
