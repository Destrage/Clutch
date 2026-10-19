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
