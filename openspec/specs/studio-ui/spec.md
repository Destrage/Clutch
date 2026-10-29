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
