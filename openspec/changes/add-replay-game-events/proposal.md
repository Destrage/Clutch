# Change: Game events from local replay files

## Why
Auto-clips from game events work only where a game publishes an official local event feed:
Counter-Strike 2 and Dota 2 (Game State Integration) and League of Legends (Live Client Data
API). Popular games like PUBG, Fortnite, Rainbow Six Siege and Rocket League have no such feed,
so for them Clutch can only clip on audio hype.

Many of these games write replay files to the player's own disk, and those files record kills,
knocks, goals and round results. Reading files the game has already written never touches the
game process, so it is safe with anti-cheat (BattlEye, Easy Anti-Cheat), unlike memory reading
or injection, which can get a player banned. Other clipping apps rely on the same source: their
support pages tell users to turn on the game's replays when auto-clips don't trigger.

## What Changes
- **Replay watchers:** an integration type that watches a game's replay folder, parses new or
  growing replay files, and reports the local player's events. Watchers are opt-in per game,
  only read, never write into game folders, and run on their own thread at low priority.
- **Live and post-match delivery:** when a game writes its replay during the match, events
  arrive within seconds and trigger auto-clips from the replay buffer like other integrations.
  When the replay is only complete after the match, events are added as markers to the
  recording that covered the match, and the user can save clips from them.
- **Games, in order:**
  1. PUBG: Battlegrounds: kills and knocks (`%LOCALAPPDATA%\TslGame\Saved\Demos`).
  2. Fortnite: eliminations and knocks (`%LOCALAPPDATA%\FortniteGame\Saved\Demos`).
  3. Rocket League: goals, from saved replay headers.
  4. Rainbow Six Siege: kills, headshots and plants, per round (`MatchReplay` `.rec` files).
- **New event kinds:** `Knock` (downed, not eliminated) and `Goal`.
- **Version tolerance:** replay formats change with game patches. A file a parser can't read
  is skipped and logged once per game version, and never crashes or stalls the app.

## Not included
- Apex Legends, Valorant kills, Overwatch 2 and Call of Duty: no local replay files or legitimate
  local feeds exist. They need picture-based event detection, a separate change.
- Online match-history APIs (PUBG developer API, Riot match API): a possible later addition for
  post-match markers.

## Impact
- Affected specs: `clips`.
- Also tunes audio hype: off for games with a connected event integration, stricter on low.
- Affected code: `estream-clips` (replay watcher framework, per-game parsers, new event kinds,
  marker alignment), `apps/estream` (per-game toggles in clip settings, post-match markers and
  clips in the recordings view).
- New dependency: `boxcars` (Rust Rocket League replay parser, MIT). Other parsers are written
  in-house from the formats documented by community projects.
