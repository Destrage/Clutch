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
- **THEN** the capture thread waits for each copy while sleeping between checks, not spinning

#### Scenario: Slot still in use
- **WHEN** every free capture slot may still be read by submitted GPU work
- **THEN** the new frame is dropped instead of overwriting a slot or waiting

### Requirement: Capture rate limited to need
A capture source SHALL copy at most about twice as many frames per second as the video thread
renders, releasing extra frames without copying them.

#### Scenario: 240 fps game on a 60 fps canvas
- **WHEN** a game presents at 240 fps and the fastest canvas runs at 60 fps
- **THEN** the source copies about 120 frames per second and every rendered frame has a new picture
