## ADDED Requirements

### Requirement: Live Streaming
The system SHALL stream the program canvas and one audio track to a destination over RTMP,
RTMPS, SRT, RIST or WHIP, chosen by the destination URL.

#### Scenario: RTMP stream
- **WHEN** the user goes live to an RTMP server with a stream key
- **THEN** the server receives H.264 (or HEVC/AV1 via Enhanced RTMP) video and AAC audio at the
  canvas resolution and frame rate

#### Scenario: SRT stream
- **WHEN** the destination URL starts with `srt://`
- **THEN** the stream is sent as MPEG-TS over SRT

#### Scenario: Incompatible settings
- **WHEN** the destination is WHIP and the video codec is not H.264
- **THEN** going live is refused with a message explaining what to change

### Requirement: Network Resilience
Streaming SHALL NOT block rendering or encoding. When the network cannot keep up, the system
SHALL drop video from the send queue at keyframe boundaries, keep audio, and count the drops.

#### Scenario: Congestion
- **WHEN** the queued video exceeds the drop threshold
- **THEN** video packets up to the next keyframe are dropped, audio packets are kept, and the
  network-dropped counter increases

### Requirement: Automatic Reconnect
When the connection fails, the system SHALL reconnect with exponential back-off up to a
configurable number of attempts, starting each new connection with a keyframe.

#### Scenario: Server comes back
- **WHEN** the server is unreachable at first and becomes reachable later
- **THEN** the stream shows "Reconnecting", then connects and delivers video starting at a keyframe

#### Scenario: Stop while connecting
- **WHEN** the user stops while a connection attempt is hanging
- **THEN** the stream stops within 2 seconds

### Requirement: Stream Key Protection
The stream key SHALL NOT be stored in plain text or written to logs.

#### Scenario: Settings file
- **WHEN** a stream key is saved
- **THEN** the settings file contains only an encrypted form of it, and log lines show the URL
  with the key redacted
