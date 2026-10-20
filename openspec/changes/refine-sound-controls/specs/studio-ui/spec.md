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
Each audio source SHALL have a round knob that shows its volume and live level. Double-clicking it
SHALL mute or unmute, dragging up or down or scrolling SHALL change the volume, Ctrl+clicking SHALL
reset it to 0 dB, and clicking the source's name SHALL open its audio settings. A single click
SHALL do nothing, so grabbing the knob to change the volume never mutes it.

#### Scenario: Double-click mutes
- **WHEN** the user double-clicks a knob
- **THEN** the source is muted, or unmuted if it was muted, and its volume is unchanged

#### Scenario: Single click does nothing
- **WHEN** the user clicks a knob once without dragging
- **THEN** neither the mute state nor the volume changes

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

### Requirement: Hearing media locally
Media files and browser sources SHALL be heard on the streamer's speakers by default, and sounds
Clutch plays to the speakers SHALL NOT be captured again by Desktop Audio when no game's audio is
being left out.

#### Scenario: New media source
- **WHEN** the user adds a media file with sound
- **THEN** it is heard on the streamer's speakers and sent to the stream

#### Scenario: No echo through Desktop Audio
- **WHEN** a monitored media source plays while Desktop Audio is on and no game is left out
- **THEN** the stream contains the media once
