## ADDED Requirements

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
