## ADDED Requirements

### Requirement: Clip Engine Audio
When clip mode buffers on its own engine, that engine SHALL mix the studio's microphone sources
with their device, filters, volume, mute and sync offset (the default microphone when the studio
has none), together with the game capture's sound and desktop audio.

#### Scenario: Noise suppression in clips
- **WHEN** the studio microphone has noise suppression and clip mode buffers on its own engine
- **THEN** the microphone in the clips is noise-suppressed as on the stream

### Requirement: Clip Player Tracks
The clip player SHALL offer the full mix and each further named audio track of the selected
clip or recording. Choosing source tracks SHALL play only those, mixed; choosing the full mix SHALL play
the first audio stream. The choice SHALL be usable from the keyboard and labelled for screen readers.

#### Scenario: Game without the mic
- **WHEN** the user opens a clip and selects only the "Game capture" track
- **THEN** the player plays the game's sound without the microphone

#### Scenario: Clip without source tracks
- **WHEN** the clip has a single audio stream
- **THEN** no track choice is shown

### Requirement: Trim Keeps Tracks
Trimming a clip (lossless or precise) SHALL keep every audio stream with its name and default
flag.

#### Scenario: Trimmed clip keeps the mic track
- **WHEN** a clip with Game capture and Microphone tracks is trimmed
- **THEN** the trimmed file has the same tracks with the same names
