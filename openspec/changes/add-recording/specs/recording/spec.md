## ADDED Requirements

### Requirement: Recording
The system SHALL record a canvas and its enabled audio tracks to a file, and SHALL NOT stall
rendering while recording. Frames the encoder cannot keep up with SHALL be dropped and counted.

#### Scenario: Start and stop
- **WHEN** the user starts recording, waits 5 seconds and stops
- **THEN** a playable file exists whose duration is 5 s ± 0.2 s at the canvas frame rate and resolution

#### Scenario: Multi-track audio
- **WHEN** tracks 1 and 2 are enabled for recording
- **THEN** the file contains two audio streams, each carrying its track's mix

#### Scenario: Encoder overload
- **WHEN** the encoder cannot keep up with the frame rate
- **THEN** the canvas keeps rendering at full rate and the dropped-frame counter increases

### Requirement: Encoder Selection
The system SHALL offer hardware encoders (NVENC, AMF, QuickSync) and software encoders for H.264,
HEVC and AV1, SHALL pick the best available hardware encoder automatically, and SHALL fall back to
software when no hardware encoder can be opened.

#### Scenario: Automatic encoder
- **WHEN** the encoder is set to "automatic" on a machine with an NVIDIA GPU
- **THEN** recording uses NVENC

### Requirement: Audio and Video Sync
Video and audio SHALL be timestamped from the engine clock so they stay in sync for the
whole recording.

#### Scenario: Sync over time
- **WHEN** a 60-second recording is made of a test pattern and test tone
- **THEN** the audio stream's duration matches the video duration within 50 ms

### Requirement: Containers
The system SHALL write MKV (default), fragmented MP4, MP4 and MOV files, and SHALL optionally
remux MKV recordings to MP4 when they stop.

#### Scenario: Crash-safe default
- **WHEN** the application terminates unexpectedly during an MKV recording
- **THEN** the file is playable up to the last written cluster

#### Scenario: Automatic remux
- **WHEN** auto-remux is enabled and an MKV recording stops
- **THEN** an MP4 with the same streams is written next to it without re-encoding

### Requirement: Replay Buffer
The system SHALL keep the last N seconds (configurable, 5–300) of encoded output in memory while
the replay buffer is active, and SHALL save them to a file on demand without stopping the buffer.

#### Scenario: Save replay
- **WHEN** the replay buffer has been running for 60 seconds with a 30-second length and the user saves
- **THEN** a file of 30 s ± 2 s (starting at a keyframe) is written and the buffer keeps running

### Requirement: Recording Status
The system SHALL show recording state, elapsed time, file size and dropped frames while recording.

#### Scenario: Status visible
- **WHEN** recording is active
- **THEN** the Record button shows a running timer and the status bar shows size and dropped frames
