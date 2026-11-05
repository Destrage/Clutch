## MODIFIED Requirements

### Requirement: Studio Mode
The system SHALL offer a studio mode with an editable Preview scene shown next to the Program.
Changes to the Preview SHALL NOT affect the Program or outputs until it is transitioned to the
Program. The Preview SHALL show the canvas being edited, and Transition SHALL also take every
canvas that switches scenes together with it to its scene of the same name.

#### Scenario: Prepare off-air
- **WHEN** studio mode is on and the user selects scene B while scene A is on program
- **THEN** the Preview shows B, the Program and all outputs keep showing A, and edits apply to B

#### Scenario: Take to program
- **WHEN** the user presses Transition
- **THEN** B becomes the program scene using the selected transition

#### Scenario: Take to program on every canvas
- **WHEN** the 1080p canvas and its 1440p duplicate are live and the user presses Transition
  with "Be right back" in the Preview
- **THEN** both canvases go to their "Be right back" scenes with the same transition

## ADDED Requirements

### Requirement: Studio Mode While Live
The Live deck SHALL have a studio mode switch. With studio mode on, the deck SHALL show the
Preview and the Program side by side, its scene buttons SHALL pick the Preview, and it SHALL
offer Transition and Cut. With more than one canvas live, the deck SHALL let the user choose
which canvas it shows.

#### Scenario: Prepare a scene during a stream
- **WHEN** the user is live, turns on studio mode in the Live deck and taps "Be right back"
- **THEN** the deck shows "Be right back" in the Preview while the Program and streams stay
  on "Gameplay", until the user presses Transition
