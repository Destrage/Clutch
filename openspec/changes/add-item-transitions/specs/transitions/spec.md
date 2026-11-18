## ADDED Requirements

### Requirement: Moving Matching Items
When "Move matching items" is on for a scene transition, the system SHALL move every item whose
source is visible in both the scene being left and the new scene (on the same canvas) from its
place in the old scene to its place in the new one over the transition's duration, instead of
including it in the scene transition. The rest of the scene SHALL use the chosen transition, with
the moving items drawn whole on top of it. Outputs SHALL show the moves exactly as the program
preview does.

#### Scenario: Camera moves to its new place
- **WHEN** a camera is at (40, 40) sized 480×270 in scene A and at (720, 300) sized 960×540 in
  scene B, and the program changes from A to B with a 600 ms Fade, Linear ease and moves on
- **THEN** the frame 300 ms in shows a single camera, centered halfway between the two centers,
  and the frame at 600 ms shows it only at its place in B

#### Scenario: Only matching items move
- **WHEN** scene A has a camera and an overlay and scene B has the same camera and a different
  overlay
- **THEN** the camera moves, and the two overlays crossfade with the scene transition

#### Scenario: Moves off
- **WHEN** "Move matching items" is off
- **THEN** the transition renders exactly as it did before this change

### Requirement: Move Motion
A moving item's position, size, rotation, crop and opacity SHALL change smoothly along the chosen
ease curve (Smooth, Spring or Linear), with rotation taking the shorter way round, and SHALL follow
a curved path when that option is on.

#### Scenario: Smooth ease
- **WHEN** a move uses the Smooth ease
- **THEN** the item starts and ends slowly, and is at the midpoint of its path halfway through

#### Scenario: Short-way rotation
- **WHEN** an item is rotated 350° in scene A and 10° in scene B
- **THEN** it turns 20° during the move, not 340°

### Requirement: Item Matching
Items SHALL be paired by source in list order within a canvas; extra items of that source on
either side SHALL take part in the scene transition. An item with "Move between scenes" off SHALL
take part in the scene transition instead of moving.

#### Scenario: Opted-out item
- **WHEN** the camera has "Move between scenes" off
- **THEN** it takes part in the scene transition and does not move

#### Scenario: Source used twice
- **WHEN** scene A shows a camera once and scene B shows it twice
- **THEN** the camera moves to B's upper item in the list, and B's second camera item fades in
  with the scene transition

### Requirement: Shape Morph
When a moving item has a mask at either end, the system SHALL change its outline smoothly from the
old shape to the new one, without showing both outlines at once, and SHALL blend its edge effects
from the old look to the new one.

#### Scenario: Rounded rectangle to circle
- **WHEN** a camera with a rounded-rectangle mask in scene A moves to scene B where it has a
  circle mask
- **THEN** halfway through, its outline lies between the two shapes as a single outline, and at
  the end it is the circle

#### Scenario: Mask to no mask
- **WHEN** a camera masked as a circle in scene A has no mask in scene B
- **THEN** the circle grows smoothly into the full rectangle of the item

### Requirement: Item Show and Hide Animations
The system SHALL let each scene item have a show and a hide animation (None, Fade, Zoom, Slide,
Pop or Wipe) with a duration and an ease curve. The show animation SHALL run when the item is made
visible and the hide animation when it is hidden, by any means (eye button, hotkey, macro, remote
API). Reversing visibility during an animation SHALL continue from the current state without a
jump.

#### Scenario: Hide with Fade
- **WHEN** an overlay with a 400 ms Fade hide animation is hidden
- **THEN** the frame 200 ms later shows it at half its opacity, and from 400 ms on it is gone

#### Scenario: Reverse mid-animation
- **WHEN** an item with a 400 ms Zoom show animation is hidden 100 ms after being shown
- **THEN** it shrinks from its current size rather than jumping to full size first

### Requirement: Item Animations During Scene Changes
During a scene transition, an item only in the scene being left SHALL play its hide animation and
an item only in the new scene its show animation, when it has one; without one it SHALL take part
in the scene transition.

#### Scenario: Item leaving with a scene change
- **WHEN** the program changes scenes and an alert box only in the old scene has a Slide hide
  animation
- **THEN** the alert box slides out while the rest of the old scene uses the scene transition

### Requirement: Item Transition Settings Persistence
The move setting and each item's animations SHALL be saved with the scene collection, and changing
them SHALL be undoable and possible through the remote API. Collections saved before this change
SHALL load with moves off and no item animations.

#### Scenario: Old collection
- **WHEN** a collection saved before item transitions existed is opened
- **THEN** "Move matching items" is off and no item has a show or hide animation
