## ADDED Requirements

### Requirement: Video thread stall reporting
The engine SHALL monitor the video thread's progress and log a warning naming the current
render stage, and the source being ticked if any, when the thread makes no progress for 2
seconds. It SHALL log again when the thread recovers. Monitoring SHALL NOT allocate or take
locks on the video thread and SHALL continue while the engine shuts down.

#### Scenario: Video thread blocks inside a source
- **WHEN** a source's tick blocks the video thread for more than 2 seconds
- **THEN** the log contains a stall warning naming the tick stage and that source

#### Scenario: Video thread recovers
- **WHEN** a stalled video thread starts making progress again
- **THEN** the log contains a recovery message with the stall duration

### Requirement: Compositor independent of UI presentation
On Windows, the engine SHALL render on a GPU device separate from the one the UI presents
with, so UI presentation and window surface reconfiguration cannot block or fail because of
compositor submissions. Frames shown in the UI SHALL be shared between the devices without a
CPU copy, ordered by GPU fences. If sharing is unavailable, the engine SHALL fall back to a
single shared device.

#### Scenario: UI presentation stalls
- **WHEN** the UI's presentation is blocked, for example by a fullscreen game owning the display
- **THEN** the compositor keeps rendering canvases and feeding outputs at their frame rate

#### Scenario: Sharing unavailable
- **WHEN** shared GPU textures or fences cannot be created
- **THEN** the engine logs the reason and renders on the UI's device as before

### Requirement: GPU scheduling priority
On Windows, the application SHALL request a raised GPU scheduling priority for its process at
startup: high when hardware-accelerated GPU scheduling is enabled, realtime otherwise. It SHALL
skip Intel adapters and continue normally if the request is refused.

#### Scenario: Started without administrator rights
- **WHEN** the application starts without permission to raise GPU scheduling priority
- **THEN** it logs the refusal at debug level and starts normally
