## ADDED Requirements

### Requirement: Clip Mode
When clip mode is enabled, the system SHALL start the replay buffer automatically while a game is
running and SHALL stop it after the game has exited. A game SHALL be recognized from a built-in
list of known games or from programs the user has marked as games. If no program scene shows the
game, the system SHALL capture the game window and desktop audio for the buffer without changing
the user's scenes.

#### Scenario: Game starts and exits
- **WHEN** clip mode is on and the user launches a known game
- **THEN** the replay buffer starts within a few seconds, and it stops after the game process has
  exited

#### Scenario: No scene set up
- **WHEN** clip mode is on and the user's program scene contains only a camera
- **THEN** clips still contain the game picture and game audio

### Requirement: Manual Clips
The system SHALL save a clip of the last N seconds (configurable, default 30) when the user
presses the clip hotkey, including while another application is focused. The replay buffer SHALL
keep running after the save. The clip SHALL be tagged with the detected game and confirmed with a
sound cue.

#### Scenario: Hotkey in game
- **WHEN** the replay buffer has run for 2 minutes and the user presses the clip hotkey while
  the game is focused
- **THEN** a clip of about 30 seconds ending at the key press is added to the library under that
  game

### Requirement: Clip Time Windows
The system SHALL save clips covering a window before and after a request time, waiting for the
after part to be buffered. Requests whose windows overlap SHALL be merged into one clip that
lists every triggering event, up to a maximum clip length.

#### Scenario: Post-roll
- **WHEN** a kill triggers a clip with 10 s before and 3 s after
- **THEN** the saved clip lasts about 13 s and shows 3 s after the kill

#### Scenario: Overlapping events merge
- **WHEN** two kills 4 s apart each request a 10 s before / 3 s after clip
- **THEN** one clip of about 17 s is saved, listing both kills

### Requirement: Game Integration Auto-Clip
The system SHALL support automatic clips from game events reported by official local game APIs:
Counter-Strike 2 and Dota 2 (Game State Integration) and League of Legends (Live Client Data
API). Supported events SHALL include kill, multi-kill, ace, round or match win, and objectives
where the game reports them. Each event kind SHALL be enabled per game with its own before/after
lengths. Events SHALL only count for the local player.

#### Scenario: CS2 kill
- **WHEN** CS2 is connected, auto-clip on kills is enabled, and the local player gets a kill
- **THEN** a clip containing the kill is saved and tagged "Counter-Strike 2" with a kill event

#### Scenario: Spectating does not clip
- **WHEN** the user is spectating another player in CS2 who gets a kill
- **THEN** no clip is saved

### Requirement: Game Integration Setup
Setting up an integration that writes a config file into a game folder SHALL require an explicit
user action. The game state listener SHALL accept requests only from the local machine and only
with the token written into that config file.

#### Scenario: Connect writes the config
- **WHEN** the user presses "Connect" for CS2
- **THEN** the integration config file is written into the CS2 config folder found through the
  Steam library, and nothing is written before the user presses it

#### Scenario: Unauthenticated payload
- **WHEN** a request reaches the game state listener without the configured token
- **THEN** it is rejected and no event is raised

### Requirement: Audio Hype Auto-Clip
The system SHALL optionally save clips when the short-term loudness of selected audio sources rises
far above their recent baseline, with a selectable sensitivity and a cooldown between clips. It
SHALL work for any game.

#### Scenario: Shout
- **WHEN** hype detection is on for the mic and the user shouts after a quiet minute
- **THEN** one clip is saved, tagged as a hype clip

#### Scenario: Steady loud audio
- **WHEN** game audio stays loud at a constant level for several minutes
- **THEN** no hype clips are saved

### Requirement: Clip Library
The system SHALL store clips in a clips folder with metadata (game, time, trigger, events with
their times inside the clip, duration, favorite, title) and a thumbnail. A Clips view SHALL list
them with thumbnails, filter them by game, trigger and favorites, and let the user play, rename,
favorite, delete, reveal and copy a clip.

#### Scenario: Browse by game
- **WHEN** the user has clips from two games and filters the Clips view by one game
- **THEN** only that game's clips are shown, newest first

#### Scenario: Clip without metadata
- **WHEN** a video file without metadata is placed in the clips folder
- **THEN** it appears in the library as a manual clip with no game

### Requirement: Clip Trimming
The system SHALL trim a clip to an in and out point, either losslessly (cutting at keyframes) or
precisely (re-encoding the kept range), saving it as a new clip or replacing the original
only after the trimmed file has been written successfully.

#### Scenario: Precise trim
- **WHEN** the user trims a 30 s clip to start at 12.4 s with precise trim
- **THEN** the result's first frame is the frame at 12.4 s and audio stays in sync

### Requirement: Clip Storage Cap
The system SHALL delete the oldest automatic clips when their total size exceeds the configured
cap, and SHALL never automatically delete favorites or manual clips.

#### Scenario: Cap reached
- **WHEN** automatic clips exceed the cap after a new save
- **THEN** the oldest non-favorite automatic clips are deleted until the total is under the cap

### Requirement: Recording Markers
While a recording is running, the system SHALL write the game events and hype peaks it detects
to a marker file next to the recording, with their times relative to the recording start.

#### Scenario: Markers during a recording
- **WHEN** the user records a 20-minute CS2 match with auto-clip events enabled
- **THEN** the recording's marker file lists each kill with its time in the recording
