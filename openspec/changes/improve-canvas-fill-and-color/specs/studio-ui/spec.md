## ADDED Requirements

### Requirement: Color picker
Every color setting in the inspector, dialogs and settings SHALL open the same color picker. The
picker SHALL have a hue ring, a saturation and brightness square, an opacity slider, hex and
RGB fields, the preset swatches, the 12 most recently picked colors, and an eyedropper. Changes
SHALL show live while dragging, and one picker session SHALL be one undo step. The eyedropper
SHALL pick a color from the Preview on every platform, and from anywhere on screen where the
platform supports it.

#### Scenario: Picking with the wheel
- **WHEN** the user opens a Color source's color, drags around the hue ring and in the square,
  then closes the picker
- **THEN** the source's color follows the pointer while dragging, and one Ctrl+Z restores the
  color it had before the picker opened

#### Scenario: Matching the game's color
- **WHEN** the user clicks the eyedropper, chooses the Preview and clicks a pixel of the game
- **THEN** the color becomes that pixel's color

#### Scenario: Picking from another window
- **WHEN** on Windows the user presses the picker's screen button, drags to a point in another
  app's window and releases
- **THEN** the color becomes the color shown at that point, and the app does not get a click

#### Scenario: Recent colors
- **WHEN** the user picks a color for one source and then opens the picker for another
- **THEN** the first color is listed under recent colors

#### Scenario: Transparency without typing
- **WHEN** the user drags the opacity slider to 50%
- **THEN** the hex field shows the color with alpha `80`

### Requirement: Removing sounds from the mixer
Each mixer strip and the sound settings panel SHALL offer Remove. For a source heard in all
scenes, Remove SHALL remove the source. For a source placed in the current scene, Remove SHALL
remove it from that scene the same way deleting it from the source list does. Remove SHALL be
undoable.

#### Scenario: Removing Desktop Audio
- **WHEN** the user right-clicks the Desktop Audio strip, which is heard in all scenes, and picks
  Remove
- **THEN** Desktop Audio is removed from the mixer and from every scene's mix, a message says
  Ctrl+Z undoes it, and Ctrl+Z brings it back with its volume and filters

#### Scenario: Adding it back later
- **WHEN** the user has removed Desktop Audio and later clicks "Add audio source" in the mixer
  and picks Desktop audio
- **THEN** a new Desktop audio source is added and heard
