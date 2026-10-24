## ADDED Requirements

### Requirement: Replay File Game Events
The system SHALL support game events read from replay files that a game writes to the local disk,
for PUBG: Battlegrounds, Fortnite, Rocket League and Rainbow Six Siege. Supported events SHALL
include kills, knocks, goals, headshots and objective plants where the replay records them, and
SHALL only count for the local player. The system SHALL only read replay files, never write into
game folders, and never read or modify game process memory.

#### Scenario: PUBG knock during a live replay
- **WHEN** PUBG replay events are enabled, the replay is written during the match, and the local player knocks an enemy down
- **THEN** a clip containing the knock is saved from the replay buffer and tagged "PUBG: Battlegrounds" with a knock event

#### Scenario: Another player's kill
- **WHEN** a replay records a kill by a player other than the local player
- **THEN** no event is raised for it

### Requirement: Post-Match Replay Markers
When a replay's events become available only after the match, the system SHALL add them as
markers to the recording that covered the match, aligned to the recording's timeline, and SHALL
let the user save a clip from any such marker. Without a covering recording, post-match events
SHALL be discarded.

#### Scenario: Rocket League goals after the match
- **WHEN** a Rocket League match is recorded and its replay is saved at the end
- **THEN** each goal by the local player appears as a marker in that recording at the moment of the goal

#### Scenario: No recording
- **WHEN** a replay is finalized after a match that was not recorded
- **THEN** no markers or clips are created from it

### Requirement: Replay Integration Setup
Replay events SHALL be off by default and enabled per game by the user. When a game's replay
folder is missing, the integration SHALL tell the user to turn on replays in that game.

#### Scenario: Replays turned off in the game
- **WHEN** the user enables replay events for Fortnite and no Fortnite replay folder exists
- **THEN** the integration shows "Turn on replays in Fortnite" and raises no events

### Requirement: Replay Format Tolerance
A replay the system cannot parse, such as one from a newer game version, SHALL be skipped and
logged once per game and version. Parsing SHALL run off the video, audio and UI threads, and a
parser failure SHALL NOT crash, stall or degrade capture, rendering or outputs.

#### Scenario: Game patch changes the format
- **WHEN** a game update changes the replay format to a version the parser does not know
- **THEN** that replay is skipped, one log entry names the game and version, and the watcher keeps running for later replays

### Requirement: Hype Alongside Live Game Events
While the running game has a connected integration that reports events live (Counter-Strike 2,
Dota 2, League of Legends), game-audio hype SHALL NOT make clips; microphone hype still can.
Replay-file integrations report only after the match, and some modes write no replay, so
game-audio hype SHALL stay on for them.

#### Scenario: CS2 connected
- **WHEN** CS2 is connected and a gunfight makes the game audio jump
- **THEN** no hype clip is saved; kills reported by the game are clipped instead

#### Scenario: PUBG Team Deathmatch
- **WHEN** PUBG replay events are connected and the player is in a mode that writes no replay
- **THEN** game-audio hype still makes clips

### Requirement: Low Game Hype Sensitivity
On low sensitivity, game-audio hype SHALL require loud audio outright (about −16 dBFS) as well as
a jump over a baseline of at least −34 dBFS, so ordinary fights in shooters don't count.

#### Scenario: Ordinary fight on low
- **WHEN** game-audio hype is on with low sensitivity and a fight rises to −18 dBFS after a quiet stretch
- **THEN** no hype clip is saved
