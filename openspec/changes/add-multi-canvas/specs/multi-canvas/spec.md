## ADDED Requirements

### Requirement: Multiple Canvases
The system SHALL let the user create, rename and remove canvases of any size and frame rate in
one scene collection, switch between them in the studio, and use any source in scenes of any
canvas.

#### Scenario: Add a vertical canvas
- **WHEN** the user adds a canvas from the "Vertical 1080 × 1920" preset
- **THEN** a canvas with one empty scene appears as a new tab and becomes the one being edited

#### Scenario: Remove a canvas
- **WHEN** the user removes a canvas and then presses Undo
- **THEN** the canvas and its scenes come back

### Requirement: Audio Across Canvases
The audio mix SHALL include the sources visible in the active scene of every canvas, with each
source mixed once.

#### Scenario: Camera on two canvases
- **WHEN** the same camera source is visible on the horizontal and the vertical canvas
- **THEN** its audio is heard once

### Requirement: Per-Canvas Outputs
Recordings, the replay buffer and every stream destination SHALL choose the canvas they output,
and recording SHALL be able to capture all canvases at once.

#### Scenario: Vertical stream
- **WHEN** a destination is set to the vertical canvas
- **THEN** it streams 1080 × 1920 video while other destinations stream the horizontal canvas

#### Scenario: Record everything
- **WHEN** recording is set to all canvases
- **THEN** one file per canvas is written, each at its canvas's size
