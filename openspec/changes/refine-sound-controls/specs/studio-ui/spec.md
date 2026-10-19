## MODIFIED Requirements

### Requirement: Live deck
While streaming, the studio SHALL show the Live deck: stream time, viewers, connection health,
the program picture with a clip button, scene buttons with their shortcuts, and sound knobs.

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
and sound knobs with level meters.

#### Scenario: Switching scenes
- **WHEN** the user clicks a scene card
- **THEN** that scene becomes active and its card is highlighted

## ADDED Requirements

### Requirement: Sound knobs
Each audio source SHALL have a round knob that shows its volume and live level. Clicking it SHALL
mute or unmute, dragging up or down or scrolling SHALL change the volume, double-clicking SHALL
reset it to 0 dB, and clicking the source's name SHALL open its audio settings.

#### Scenario: Click mutes
- **WHEN** the user clicks a knob without dragging
- **THEN** the source is muted, or unmuted if it was muted, and its volume is unchanged

#### Scenario: Drag changes volume
- **WHEN** the user drags a knob upward
- **THEN** the volume rises, the dB value is shown during the drag, the mute state is unchanged,
  and the whole drag is one undo step

#### Scenario: Name opens settings while live
- **WHEN** the user clicks a source's name on the Live deck
- **THEN** that source's audio settings open over the Live deck

### Requirement: Closing the settings panel
The item settings panel SHALL have a close button, and Escape SHALL close it.

#### Scenario: Close button
- **WHEN** the user clicks the close button on the settings panel
- **THEN** the panel closes and nothing is selected
