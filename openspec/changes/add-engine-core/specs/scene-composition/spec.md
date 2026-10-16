## ADDED Requirements

### Requirement: Project Model
The system SHALL model a project as canvases, scenes, scene items and sources. A source SHALL be
able to appear in multiple scenes through separate scene items, with each scene item placing
exactly one source. Every object SHALL have a stable unique ID independent of its display name.

#### Scenario: Shared source across scenes
- **WHEN** the same camera source is added to two scenes
- **THEN** both scene items render the same live frames
- **AND** each item has its own independent transform

#### Scenario: Rename keeps references
- **WHEN** a source is renamed
- **THEN** all scene items referencing it remain valid

### Requirement: Scene Item Transform
Each scene item SHALL support position, rotation, scale, crop (left/top/right/bottom), bounds
(none, stretch, fit, fill) with alignment, opacity, visibility and z-order.

#### Scenario: Fit bounds preserve aspect ratio
- **WHEN** a 1920×1080 source is placed in a 1000×1000 bounding box with mode "fit" and center alignment
- **THEN** it is rendered at 1000×562.5 vertically centered inside the box

#### Scenario: Crop before transform
- **WHEN** an item has 100 px crop on the left and 2× scale
- **THEN** the cropped region is removed first and the remaining image is scaled by 2

### Requirement: Groups
The system SHALL support grouping scene items. Group transforms SHALL be applied to their children.

#### Scenario: Moving a group moves its children
- **WHEN** a group containing two items is moved by (100, 50)
- **THEN** both children render offset by (100, 50) while their local transforms are unchanged

### Requirement: Blend Modes
Scene items SHALL support normal, additive, multiply and screen blending, computed in linear light.

#### Scenario: Linear-light blending
- **WHEN** a 50% opaque white item is blended over black using normal mode
- **THEN** the resulting linear value is 0.5 (sRGB-encoded ≈ 188)

### Requirement: GPU Compositor
The system SHALL composite each canvas on the GPU into a linear-light, high-precision texture each
frame and SHALL provide format conversions for preview (sRGB 8-bit) and encoding (NV12).
Compositing SHALL NOT copy frames to CPU memory.

#### Scenario: Canvas renders every frame
- **WHEN** a canvas at 1920×1080@60 contains a moving test pattern
- **THEN** a new canvas texture is produced for every frame tick

#### Scenario: Missing source frame
- **WHEN** a source has not yet produced any frame
- **THEN** it is rendered as fully transparent without errors

### Requirement: Multiple Canvases in the Model
The project model SHALL support more than one canvas, each with its own resolution, frame rate and
scene list.

#### Scenario: Two canvases render independently
- **WHEN** a project has a 1920×1080@60 canvas and a 1080×1920@30 canvas
- **THEN** each canvas renders at its own resolution and frame rate

### Requirement: Headless Rendering
The compositor SHALL support rendering without a window, to support automated tests and servers.

#### Scenario: Golden image test
- **WHEN** a known scene is rendered headless
- **THEN** the output matches the stored reference image within the configured tolerance
