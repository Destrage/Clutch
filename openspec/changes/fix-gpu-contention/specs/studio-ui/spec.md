## ADDED Requirements

### Requirement: Window resizes while rendering
The studio window SHALL be drawn at its current size after any resize, maximize, restore, or
display mode change, including while canvases are rendering and outputs are live.

#### Scenario: Display mode changes during a fullscreen game
- **WHEN** a fullscreen game changes the display resolution and later restores it while Clutch is streaming
- **THEN** the studio window is drawn at its current size, with no stale or offset contents
