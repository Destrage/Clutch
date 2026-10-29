## ADDED Requirements

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
