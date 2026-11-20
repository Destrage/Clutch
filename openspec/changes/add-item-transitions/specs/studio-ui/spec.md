## ADDED Requirements

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
