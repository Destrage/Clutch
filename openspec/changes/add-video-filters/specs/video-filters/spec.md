## ADDED Requirements

### Requirement: Source Filter Chains
Each source SHALL have an ordered list of video filters that can be added, removed, reordered,
enabled, disabled and adjusted. The filtered result SHALL be used wherever the source appears.
Changes SHALL be undoable.

#### Scenario: Disable a filter
- **WHEN** a filter is disabled
- **THEN** the source renders as if the filter were not there

#### Scenario: Undo a slider drag
- **WHEN** the user drags a filter slider and then presses Undo
- **THEN** the parameter returns to its value before the drag in one step

### Requirement: Built-in Filters
The system SHALL provide color correction, LUT, chroma key, luma key, crop/pad, blur, sharpen and
image mask filters, computed on the GPU without stalling rendering.

#### Scenario: Green screen
- **WHEN** a chroma key filter keyed to green is applied to a source with green and red areas
- **THEN** the green areas become transparent and the red areas stay opaque

#### Scenario: Padding
- **WHEN** a crop/pad filter pads 10 pixels on the left
- **THEN** the source becomes 10 pixels wider with a transparent strip on the left

#### Scenario: LUT file not yet loaded
- **WHEN** a LUT file is still loading or cannot be read
- **THEN** the source renders unfiltered by that LUT and rendering does not wait

### Requirement: Custom Shader Filters
The system SHALL accept a user WGSL function `effect(uv)` with access to the source, its size,
time and four parameters, and SHALL reject invalid code with a message naming the user's line.

#### Scenario: Invalid filter code
- **WHEN** the user applies filter code with an undefined identifier on line 2
- **THEN** the filter is not changed and the error names line 2
