## ADDED Requirements

### Requirement: Source Audio Tracks
Recordings and the replay buffer SHALL write, after the mixed tracks, one audio stream per audio
source that can be heard on the recorded canvas (global sources and sources placed in its
scenes), fixed when the output starts (at most 16).
Each source track SHALL hold what that source adds to the mix (after its filters, volume, mute
and balance) and SHALL be named after the source. Monitor-only sources SHALL get no source track.
The mixed tracks SHALL come first and be the default.

#### Scenario: Each source on its own track
- **WHEN** a recording runs with Game capture, Microphone and Discord in the canvas mix
- **THEN** the file has the mixed track first, then three audio streams named "Game capture", "Microphone" and "Discord"
- **AND** each holds only that source's sound

#### Scenario: Muted source stays silent
- **WHEN** the microphone is muted in the mixer during a recording
- **THEN** the Microphone source track is silent for that time, as in the mix

### Requirement: Source Track Setting
Settings → Recording SHALL have a "Separate track for each source" switch, on by default, that
turns source tracks on or off for recordings and the replay buffer. Stream outputs SHALL NOT carry
source tracks.

#### Scenario: Switched off
- **WHEN** "Separate track for each source" is off
- **THEN** recordings and clips contain only the selected mixed tracks

#### Scenario: Streams unchanged
- **WHEN** the user streams
- **THEN** the stream carries only its one mixed track

### Requirement: Silent Source Tracks Left Out
A clip saved from the replay buffer SHALL leave out source tracks whose source was not heard
(peak above −60 dBFS) during the clip. A recording remuxed to MP4 SHALL leave out source tracks
whose source was never heard in the recording.

#### Scenario: Unused alert sound
- **WHEN** a clip is saved and the Alerts source made no sound during it
- **THEN** the clip has no Alerts track
