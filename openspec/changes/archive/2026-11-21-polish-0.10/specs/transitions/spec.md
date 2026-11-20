## ADDED Requirements

### Requirement: Border During a Shape Morph
While a moving item's outline morphs, its border SHALL be drawn along the in-between outline in
every frame, with its width, color and placement changing smoothly from the old border to the
new one (an end without a border counts as a border of width 0).

#### Scenario: Rectangle to circle with borders
- **WHEN** a camera with a 4 px white outside border and a rounded-rectangle mask moves to a
  scene where it has a circle mask and the same border
- **THEN** halfway through, the border is a single 4 px ring just outside the in-between outline
