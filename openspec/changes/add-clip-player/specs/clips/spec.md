## ADDED Requirements

### Requirement: In-App Clip Playback
The Clips view SHALL play the selected clip or recording inside Clutch, with play/pause, seeking,
elapsed and total time, volume and mute. Play and double-click SHALL start in-app playback, and an
action SHALL remain to open the file in the system's default player.

#### Scenario: Play a clip
- **WHEN** the user selects a clip and presses Play
- **THEN** the clip plays in the Clips view with its sound, and no external player opens

#### Scenario: Open externally
- **WHEN** the user chooses "Open in default player" for a clip
- **THEN** the clip opens in the system's default video player

### Requirement: Playback Event Markers
The player's seek bar SHALL show the clip's events, or the recording's markers, at their times,
and selecting one SHALL seek to shortly before it.

#### Scenario: Jump to a kill
- **WHEN** a recording has a kill marker at 12:30 and the user clicks it on the seek bar
- **THEN** playback continues from about 12:27

### Requirement: Trim While Watching
While trimming, the user SHALL be able to set the in and out points from the current playback
position, and playback SHALL loop within the trim range.

#### Scenario: Set the in point
- **WHEN** the user pauses at 8.2 s and presses "Set in"
- **THEN** the trim in point becomes 8.2 s

### Requirement: Preview Audio Isolation
Audio from in-app playback SHALL play on the monitoring device and SHALL NOT be mixed into any
output track, stream, recording or replay buffer.

#### Scenario: Watching a clip while live
- **WHEN** the user plays a clip in the Clips view while streaming
- **THEN** viewers do not hear the clip's audio
