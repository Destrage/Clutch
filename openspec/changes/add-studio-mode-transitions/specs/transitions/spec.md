## ADDED Requirements

### Requirement: Scene Transitions
The system SHALL change the program scene with a configurable transition: Cut, Fade, Fade to
color, Slide, Swipe, Wipe or a custom shader, with a duration. Outputs (recording, streaming)
SHALL show the transition exactly as the program preview does.

#### Scenario: Fade
- **WHEN** the program changes from scene A to scene B with a 500 ms Fade
- **THEN** the frame rendered 250 ms after the start is the even mix of A and B, and frames from
  500 ms on show only B

#### Scenario: Cut
- **WHEN** the transition is Cut
- **THEN** the next rendered frame shows only the new scene

#### Scenario: Audio during a transition
- **WHEN** a transition is running
- **THEN** audio from the sources of both scenes is mixed, and after it ends only the new
  scene's sources (and global sources) are mixed

### Requirement: Custom Shader Transitions
The system SHALL accept a user WGSL function `transition(uv, progress)` with access to both
scenes, and SHALL reject invalid code with a compiler message that points at the user's line.

#### Scenario: Invalid shader
- **WHEN** the user applies shader code with a syntax error on line 3
- **THEN** the transition is not changed and the error message names line 3

### Requirement: Transition Settings Persistence
The chosen transition SHALL be saved with the scene collection, and changing it SHALL be
undoable. Collections saved before transitions existed SHALL load with a 300 ms Fade.

#### Scenario: Old collection
- **WHEN** a collection without transition settings is opened
- **THEN** the transition is Fade with a 300 ms duration
