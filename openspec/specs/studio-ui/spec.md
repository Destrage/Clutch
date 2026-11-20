# studio-ui Specification

## Purpose
The layout of the studio window: the Stage for building scenes, the Live deck while
streaming, and the sidebar sections (Studio, Clips, Montages, Chat).

## Requirements

### Requirement: Live deck
While streaming, the studio SHALL show the Live deck: stream time, viewers, connection health,
the program picture with a clip button, scene buttons with their shortcuts, and sound faders.

#### Scenario: Going live
- **WHEN** a stream starts
- **THEN** the studio switches to the Live deck

#### Scenario: Ending the stream
- **WHEN** the stream ends
- **THEN** the studio switches back to the Stage

#### Scenario: Editing while live
- **WHEN** the user clicks Edit layout while live
- **THEN** the Stage is shown and the stream continues

### Requirement: Stage
When not live, the studio SHALL show the preview as the main element, the scenes as a strip of
thumbnail cards, the current scene's layers and the selected item's settings over the preview,
and sound as mute buttons with level meters.

#### Scenario: Switching scenes
- **WHEN** the user clicks a scene card
- **THEN** that scene becomes active and its card is highlighted

### Requirement: Sections
The app SHALL have a sidebar with Studio, Clips, Montages and Chat, and the Clips and Montages
sections SHALL be pages rather than dialogs.

#### Scenario: Opening Clips
- **WHEN** the user clicks Clips in the sidebar
- **THEN** the clips page replaces the studio in the main area, and Studio returns to it

### Requirement: Setups
The studio SHALL let users keep several setups (scene collections) and manage them from a menu on
the setup name in the top bar: switch, create (through the first-run flow), rename, duplicate and
delete.

#### Scenario: Switching setups
- **WHEN** the user picks another setup from the menu while not streaming or recording
- **THEN** the current setup is saved and the chosen one opens

#### Scenario: Switching while live
- **WHEN** the user picks another setup while streaming or recording
- **THEN** nothing changes and a message asks them to stop first

#### Scenario: New setup
- **WHEN** the user chooses New setup and enters a name
- **THEN** the first-run setup opens for that setup, and Cancel returns to the previous setup

#### Scenario: Deleting a setup
- **WHEN** the user deletes the open setup and confirms
- **THEN** its file is removed and another setup opens
- **AND** Delete is not offered when only one setup exists

### Requirement: Preview viewport scroll panning
The canvas preview SHALL NOT pan when the mouse wheel is scrolled unless preview scroll panning is
explicitly enabled in Settings. Middle mouse button drag and Space plus left mouse button drag
SHALL pan the preview regardless of the setting.

#### Scenario: Mouse wheel scrolled with default settings
- **WHEN** the user scrolls the mouse wheel over the canvas preview and scroll panning is disabled
- **THEN** the preview position does not pan

#### Scenario: Middle mouse button dragged
- **WHEN** the user presses and drags the middle mouse button over the preview
- **THEN** the preview pans following the mouse movement

### Requirement: Settings Survive Version Changes
When the settings file contains entries this version can't read, Clutch SHALL keep every other
setting, drop only those entries, and first save a copy of the original file next to it as
`settings.invalid-<date>-<time>.json`. A file that can't be read at all SHALL be copied the same
way before defaults are used.

#### Scenario: Older build meets a newer integration
- **WHEN** the settings list a game integration this build doesn't know
- **THEN** that integration is left out, the destinations, stream keys and account logins are kept, and the original file is copied to `settings.invalid-…json`

#### Scenario: Damaged file
- **WHEN** the settings file isn't valid JSON
- **THEN** Clutch starts with default settings and the damaged file is kept as `settings.invalid-…json`

### Requirement: Inspector categories
The inspector SHALL show the selection's settings in tabs by category: Source, Layout, Look
and Sound, each shown only when it applies to the selection. Within a tab, settings SHALL be
grouped in panels that the user can fold and unfold. The chosen tab and the folded panels
SHALL be kept when the selection changes and when the app restarts.

#### Scenario: A camera with a microphone
- **WHEN** the user selects a camera source that has sound
- **THEN** the inspector shows the Source, Layout, Look and Sound tabs

#### Scenario: A sound only
- **WHEN** the user selects a desktop audio source
- **THEN** the inspector shows no Layout or Look tab

#### Scenario: Keeping the tab
- **WHEN** the user picks the Sound tab on one source and then selects another source with sound
- **THEN** the Sound tab is shown for the new source

#### Scenario: Folding a panel
- **WHEN** the user clicks the Crop panel's header, then restarts the app and selects an item
- **THEN** the Crop panel is folded

### Requirement: Value sliders
Bounded numeric settings in the inspector SHALL be edited with value sliders that show the
name, the value and a bar filled to the value. Dragging SHALL change the value without jumping
to the pointer, a click without dragging SHALL allow typing an exact value, and arrow keys
SHALL step it. Each drag SHALL be one undo step.

#### Scenario: Dragging the volume
- **WHEN** the user drags the volume slider of a microphone to the left and releases it
- **THEN** the volume goes down while dragging and one Ctrl+Z restores it

#### Scenario: Typing a value
- **WHEN** the user clicks a filter's Threshold slider without dragging, types -20 and presses Enter
- **THEN** the threshold becomes -20 dB

#### Scenario: Large ranges stay numbers
- **WHEN** the user selects a browser source
- **THEN** its width and height are number fields and its frame rate is a slider

### Requirement: Color picker
Every color setting in the inspector, dialogs and settings SHALL open the same color picker. The
picker SHALL have a hue ring, a saturation and brightness square, an opacity slider, hex and
RGB fields, the preset swatches, the 12 most recently picked colors, and one eyedropper button.
Changes SHALL show live while dragging, and one picker session SHALL be one undo step.

#### Scenario: Picking with the wheel
- **WHEN** the user opens a Color source's color, drags around the hue ring and in the square,
  then closes the picker
- **THEN** the source's color follows the pointer while dragging, and one Ctrl+Z restores the
  color it had before the picker opened

#### Scenario: Recent colors
- **WHEN** the user picks a color for one source and then opens the picker for another
- **THEN** the first color is listed under recent colors

#### Scenario: Transparency without typing
- **WHEN** the user drags the opacity slider to 50%
- **THEN** the hex field shows the color with alpha `80`

### Requirement: Eyedropper
After the color picker's eyedropper is clicked, the next click SHALL pick the color under the
pointer: anywhere on screen where the platform supports it, otherwise in the Preview. Esc or a
right click SHALL cancel and restore the color.

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

### Requirement: Mask Editing on the Preview
With an item selected, "Edit mask" SHALL let the user draw rectangles, ellipses, polygons and
freehand shapes on the preview, and move, resize and reshape them with handles and points. The
outside of the mask SHALL be dimmed in the preview while editing.

#### Scenario: Draw a polygon cut
- **WHEN** the user edits the mask of a game capture, picks Polygon and Cut, clicks four points and closes the shape
- **THEN** that area of the game becomes transparent

### Requirement: Mask Panel
The item inspector SHALL have a Mask panel listing the shapes with keep/cut, feather, reorder and
delete, invert, on/off, the shape presets, an Effects section (border, glow, shadow) and effect
styles. The panel and tools SHALL be usable from the keyboard and labelled for screen readers.

#### Scenario: Neon glow style
- **WHEN** the user picks the Neon glow style on a masked item
- **THEN** the item gets a bright outside glow and the Effects section shows its color and size

#### Scenario: Rounded camera preset
- **WHEN** the user picks Rounded corners on a camera item and sets the radius to 40 px
- **THEN** the camera is drawn with 40 px rounded corners

### Requirement: Move Settings in Transition Settings
The scene transition settings SHALL offer "Move matching items" with its ease curve (Smooth,
Spring, Linear) and a curved path option, available for every transition kind except Cut.

#### Scenario: Turning moves on
- **WHEN** the user turns on "Move matching items" and switches scenes
- **THEN** items in both scenes move to their new places, and the change can be undone

### Requirement: Item Transitions Panel
The item inspector SHALL have a Transitions panel with the item's show and hide animations (kind,
direction where it applies, duration, ease) and "Move between scenes". All controls SHALL be
reachable by keyboard and have screen-reader labels.

#### Scenario: Setting a hide animation
- **WHEN** the user picks Pop as the hide animation and then hides the item with its eye button
- **THEN** the item pops out, and the setting can be undone

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
