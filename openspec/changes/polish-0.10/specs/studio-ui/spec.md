## ADDED Requirements

### Requirement: Settings That Depend on a Switch
A source setting that only matters while one of the source's switches is on SHALL be shown only
while that switch is on.

#### Scenario: Text outline
- **WHEN** a Text source has Outline off
- **THEN** the inspector doesn't show Outline color and Outline width, and turning Outline on
  shows them

### Requirement: Preview Beside Its Panels
With the scene items panel or the inspector open, the canvas SHALL be fitted into the preview
space they leave free, as long as the canvas keeps at least half of the preview's width;
otherwise it is fitted to the whole preview as before.

#### Scenario: Inspector open
- **WHEN** an item is selected on a wide window
- **THEN** no part of the canvas is under the inspector or the scene items panel

### Requirement: Stable Lists
Lists whose rows hold controls SHALL keep their rows when the app refreshes for an unrelated
change, so an open dropdown or a slider being dragged in a row is not interrupted.

#### Scenario: Remote change while a dropdown is open
- **WHEN** a filter row's dropdown is open and the remote API renames a scene
- **THEN** the dropdown stays open and Clutch keeps running

### Requirement: Transition Card Shows Moves
The transition card under the scenes SHALL say when "Move matching items" is on.

#### Scenario: Moves on
- **WHEN** the transition is a 700 ms Fade with moves on
- **THEN** the card reads Fade, 700 ms and that items move
