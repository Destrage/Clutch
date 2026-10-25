## ADDED Requirements

### Requirement: Shared encoder sessions
Outputs on the same canvas whose resolved encoder settings are identical SHALL share one
encoder session. This covers recordings, the replay and clip-mode buffer, and stream
destinations. Each output SHALL keep its own packet queue, so a slow output never blocks the
encoder or the other outputs.

#### Scenario: Recording while clip mode buffers
- **WHEN** recording starts while the clip-mode buffer runs with the same encoder settings
- **THEN** both use one encoder session

#### Scenario: Stalled destination
- **WHEN** one stream destination stops accepting data
- **THEN** the recording sharing its encoder keeps every frame

### Requirement: Keyframes on shared sessions
An output that joins a running shared session SHALL start on a keyframe, and a keyframe
requested by any output SHALL be delivered to every output of that session.

#### Scenario: Stream reconnects
- **WHEN** a destination reconnects and requests a keyframe
- **THEN** the shared session encodes an IDR frame within one frame interval
