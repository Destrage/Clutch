## ADDED Requirements

### Requirement: Scene Collection Files
The system SHALL save and load scene collections as human-readable, versioned JSON files.

#### Scenario: Round trip
- **WHEN** a project is saved and loaded again
- **THEN** the loaded project is equal to the saved project

### Requirement: Schema Migrations
The system SHALL migrate older scene collection schema versions to the current version on load,
and SHALL refuse to load files from a newer schema version with a clear message.

#### Scenario: Older file migrates
- **WHEN** a schema v1 file is opened by a build using schema v3
- **THEN** migrations v1→v2 and v2→v3 run in order and the project loads

#### Scenario: Newer file is refused
- **WHEN** a schema v5 file is opened by a build using schema v3
- **THEN** loading fails with a message that names both versions
- **AND** the file is not modified

### Requirement: Safe Saving and Autosave
The system SHALL save atomically (write to a temp file, then rename) and SHALL autosave 2 seconds
after the last change.

#### Scenario: Crash during save
- **WHEN** the process terminates while writing a save
- **THEN** the previous scene collection file remains intact

#### Scenario: Recovery after crash
- **WHEN** the application restarts after an unclean shutdown
- **THEN** it offers to restore the most recent autosaved state

### Requirement: Undo and Redo
The system SHALL provide undo and redo for every project-mutating command, including edits from the
remote API. Rapid repeated edits to the same property SHALL coalesce into a single undo step.

#### Scenario: Undo a drag
- **WHEN** the user drags an item across the canvas (many transform updates) and then presses Ctrl+Z
- **THEN** the item returns to its position before the drag in a single undo step

#### Scenario: Undo a deletion
- **WHEN** the user deletes a source used in two scenes and then undoes
- **THEN** the source and both scene items are restored with their original IDs and settings

#### Scenario: Redo invalidated by new edit
- **WHEN** the user undoes an action and then performs a new edit
- **THEN** the redo history is cleared

### Requirement: Broken Reference Tolerance
The system SHALL load scene collections that contain unavailable sources (e.g. a missing device or
file) and mark those sources as unavailable rather than failing the whole load.

#### Scenario: Missing camera
- **WHEN** a collection references a camera that is not connected
- **THEN** the project loads and the camera source shows an "unavailable" state
