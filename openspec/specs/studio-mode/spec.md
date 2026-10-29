# studio-mode Specification

## Purpose
Studio mode: editing a Preview scene while the Program scene stays live, and keeping its cost low.

## Requirements

### Requirement: Studio Mode
The system SHALL offer a studio mode with an editable Preview scene shown next to the Program.
Changes to the Preview SHALL NOT affect the Program or outputs until it is transitioned to the
Program.

#### Scenario: Prepare off-air
- **WHEN** studio mode is on and the user selects scene B while scene A is on program
- **THEN** the Preview shows B, the Program and all outputs keep showing A, and edits apply to B

#### Scenario: Take to program
- **WHEN** the user presses Transition
- **THEN** B becomes the program scene using the selected transition

### Requirement: Preview Cost
The Preview SHALL only be rendered while studio mode is on.

#### Scenario: Studio mode off
- **WHEN** studio mode is turned off
- **THEN** the engine stops rendering the Preview and frees its targets
