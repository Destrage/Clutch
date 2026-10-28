## ADDED Requirements

### Requirement: Visible Canvas Outputs
With more than one canvas, the studio SHALL show on each canvas tab which outputs use it
(stream, recording, replay buffer, virtual camera), and SHALL mark the canvas being streamed
while live.

#### Scenario: Streaming another canvas than the one edited
- **WHEN** the destinations stream "Full HD" and the user edits "2K"
- **THEN** the "Full HD" tab is tagged "Stream" and the "2K" tab has no stream tag

### Requirement: Choose The Output Canvas From The Studio
The user SHALL be able to point every destination, the recording and the replay buffer at a
canvas from that canvas's tab, and from the Go live window.

#### Scenario: Stream and record this canvas
- **WHEN** the user picks "Stream and record this canvas" on the "2K" tab and goes live
- **THEN** every destination streams 2560 × 1440 and the recording uses the same canvas

### Requirement: Duplicate Canvas
The user SHALL be able to duplicate a canvas at its size or at another canvas preset. The copy
SHALL contain every scene of the canvas with the same sources and items, scaled to the new
size, and SHALL undo in one step.

#### Scenario: Full HD to 1440p
- **WHEN** the user duplicates a 1920 × 1080 canvas as 2560 × 1440
- **THEN** a new canvas has the same scenes, each item at the same relative place and size

#### Scenario: Landscape to vertical
- **WHEN** the user duplicates a 1920 × 1080 canvas as 1080 × 1920
- **THEN** items that filled the old canvas fill the new one, and the rest of the layout is
  scaled to fit and centered

### Requirement: Copy Scene To Canvas
The user SHALL be able to copy a scene to another canvas, scaled the same way, in one undoable
step.

#### Scenario: Copy a scene
- **WHEN** the user copies "Apex" from "Full HD" to "2K"
- **THEN** "2K" gains a scene "Apex" with the same items scaled to 2560 × 1440
