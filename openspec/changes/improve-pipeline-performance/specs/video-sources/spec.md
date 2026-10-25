## ADDED Requirements

### Requirement: Capture without CPU waits
Window, display and game capture on Windows SHALL hand captured frames to the engine without
the capture thread waiting on the CPU for GPU work to finish. The engine SHALL order its reads
after the capture copies on the GPU.

#### Scenario: Game presenting quickly
- **WHEN** a captured game presents frames faster than the canvas frame rate
- **THEN** the capture thread does not spin or block while the GPU finishes its copies

#### Scenario: Older drivers
- **WHEN** shared fences are unavailable
- **THEN** capture falls back to non-blocking completion queries and still does not spin

### Requirement: Capture rate limited to need
A capture source SHALL copy at most about as many frames per second as the fastest canvas
showing it needs, releasing extra frames without copying them.

#### Scenario: 240 fps game on a 60 fps canvas
- **WHEN** a game presents at 240 fps and is shown only on a 60 fps canvas
- **THEN** the source copies about 60 frames per second
