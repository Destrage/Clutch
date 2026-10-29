## MODIFIED Requirements

### Requirement: Live Streaming
The system SHALL stream the program canvas to one or more destinations at the same time. Each
destination uses RTMP, RTMPS, SRT, RIST or WHIP (chosen by its URL) and has its own key, codec,
bitrate, audio and reconnect settings.

#### Scenario: RTMP stream
- **WHEN** the user goes live to an RTMP server with a stream key
- **THEN** the server receives H.264 (or HEVC/AV1 via Enhanced RTMP) video and AAC audio at the
  canvas resolution and frame rate

#### Scenario: SRT stream
- **WHEN** the destination URL starts with `srt://`
- **THEN** the stream is sent as MPEG-TS over SRT

#### Scenario: Two platforms at once
- **WHEN** two destinations are enabled and the user goes live
- **THEN** both servers receive the stream

#### Scenario: Shared encoder
- **WHEN** two destinations have identical encoder settings
- **THEN** a single video encoder feeds both

#### Scenario: Incompatible settings
- **WHEN** one destination is invalid (e.g. missing stream key) and another is valid
- **THEN** the valid destination goes live and the invalid one reports what to change

## ADDED Requirements

### Requirement: Destination Isolation
A destination's network problems, reconnects or failure SHALL NOT interrupt other destinations.
Each destination SHALL be stoppable and startable on its own while live.

#### Scenario: One destination fails
- **WHEN** one destination's server is unreachable
- **THEN** that destination reports an error and the others keep streaming

#### Scenario: Stop one destination
- **WHEN** the user stops one destination while live
- **THEN** its connection ends and the other destinations continue uninterrupted

#### Scenario: Add a destination while live
- **WHEN** the user starts another destination while already live
- **THEN** it connects and starts at a keyframe without restarting the others

### Requirement: Settings Migration
Settings from the single-stream format SHALL be migrated to one destination, keeping its server
and stream key.

#### Scenario: Upgrade
- **WHEN** settings with a single configured stream are loaded
- **THEN** they appear as one enabled destination with the same server and key
