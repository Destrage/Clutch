## MODIFIED Requirements

### Requirement: Color picker
Every color setting in the inspector, dialogs and settings SHALL open the same color picker. The
picker SHALL have a hue ring, a saturation and brightness square, an opacity slider, hex and
RGB fields, the preset swatches, the 12 most recently picked colors, and one eyedropper button.
Changes SHALL show live while dragging, and one picker session SHALL be one undo step. After the
eyedropper is clicked, the next click SHALL pick the color under the pointer: anywhere on screen
where the platform supports it, otherwise in the Preview. Esc or a right click SHALL cancel.

#### Scenario: Picking with the wheel
- **WHEN** the user opens a Color source's color, drags around the hue ring and in the square,
  then closes the picker
- **THEN** the source's color follows the pointer while dragging, and one Ctrl+Z restores the
  color it had before the picker opened

#### Scenario: Matching the game's color
- **WHEN** the user clicks the eyedropper and then clicks a pixel of the game in the Preview
- **THEN** the color becomes that pixel's color

#### Scenario: Picking from another window
- **WHEN** on Windows the user clicks the eyedropper and then clicks a point in another app's
  window
- **THEN** the color becomes the color shown at that point, and the app does not get the click

#### Scenario: Cancel picking
- **WHEN** the user clicks the eyedropper, moves over other colors and presses Esc
- **THEN** the color is the one it was before the eyedropper was clicked

#### Scenario: Recent colors
- **WHEN** the user picks a color for one source and then opens the picker for another
- **THEN** the first color is listed under recent colors

#### Scenario: Transparency without typing
- **WHEN** the user drags the opacity slider to 50%
- **THEN** the hex field shows the color with alpha `80`
