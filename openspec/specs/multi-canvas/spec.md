# multi-canvas Specification

## Purpose
Several canvases at once (for example landscape and vertical), each with its own scenes and outputs, sharing one audio mix.

## Requirements

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

### Requirement: Canvas Presets
The canvas presets SHALL include vertical 1080 × 1920 at 30 and 60 fps, 1440 × 2560 at 60 fps
and 2160 × 3840 at 30 and 60 fps, and landscape 3840 × 2160 at 30 and 60 fps.

#### Scenario: Add a vertical 4K canvas
- **WHEN** the user adds a canvas from the "Vertical 4K 2160 × 3840 · 60 fps" preset
- **THEN** a 2160 × 3840 canvas running at 60 fps is added

### Requirement: Canvas Settings
The user SHALL be able to change an existing canvas's name, size, frame rate and background in
a canvas settings dialog, as one undoable step. Changing the size SHALL scale the canvas's items
to the new size unless the user turns that off. Size and frame rate SHALL NOT be changeable
while a stream, recording, replay buffer or virtual camera uses the canvas.

#### Scenario: Change a canvas to 60 fps
- **WHEN** the user opens Canvas settings on a 1080 × 1920, 30 fps canvas, picks 60 fps and
  applies
- **THEN** the canvas renders at 60 fps, and one Ctrl+Z puts it back to 30 fps

#### Scenario: Resize a canvas with its items
- **WHEN** the user changes a 1080 × 1920 canvas to 1440 × 2560 with "Scale items to the new
  size" on
- **THEN** every item keeps the same relative position and size on the canvas

#### Scenario: Size locked while live
- **WHEN** a stream uses the canvas and the user opens Canvas settings
- **THEN** size and frame rate are disabled with a hint to stop the stream, recording, replay
  buffer and virtual camera on that canvas first, while name and background can still be
  changed

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

### Requirement: Canvas Background
Each canvas SHALL have a background, drawn wherever no item covers the canvas, that is either a
solid color or a linear or radial gradient. The default SHALL be solid black.

#### Scenario: Gradient background
- **WHEN** the user sets a vertical canvas's background to a linear gradient from purple to blue
- **THEN** the areas of the canvas no item covers show that gradient in the Preview and in
  streams and recordings of that canvas

### Requirement: Fill Around Fitted Items
A scene item whose source is fitted inside its bounds SHALL have a "Fill empty space" setting
with the choices Frosted, Color, Gradient and None. The fill SHALL cover the item's bounds box
behind the source. Frosted SHALL show a blurred, darkened copy of the same source frame, scaled
to cover the box, without capturing the source a second time.

#### Scenario: Game on a vertical canvas
- **WHEN** the user sets the fill of a 1920 × 1080 Game capture fitted in a 1080 × 1920 canvas
  to Frosted
- **THEN** the game stays fitted in the middle, and the areas above and below it show a blurred
  copy of the game that moves with it

#### Scenario: Solid fill
- **WHEN** the user sets the item's fill to Color and picks dark grey
- **THEN** the areas above and below the game are dark grey

#### Scenario: One capture
- **WHEN** an item uses Frosted fill
- **THEN** its source is captured once, and the blur is computed at reduced resolution

### Requirement: Default Fill
Every new item, and every item in a project saved by an earlier version, SHALL start with None
fill, so the space around a fitted source shows the canvas background.

#### Scenario: New game capture
- **WHEN** the user adds a Game capture of a 16:9 game to a vertical canvas
- **THEN** its fill is None and the areas above and below the game show the canvas background

#### Scenario: Existing project
- **WHEN** the user opens a scene collection saved by an earlier version
- **THEN** every item looks the same as before
