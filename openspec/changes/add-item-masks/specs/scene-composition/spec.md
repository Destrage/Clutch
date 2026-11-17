## ADDED Requirements

### Requirement: Item Masks
A scene item SHALL be able to carry a mask: an ordered list of shapes (rectangle with corner
radius, ellipse, polygon) in the item's own space, each keeping or cutting, with a feather width.
The item SHALL be transparent wherever the mask is not, in the preview and every output, and the
mask SHALL follow the item's position, size, rotation and crop.

#### Scenario: Round camera
- **WHEN** a camera item has a mask with one kept ellipse filling it
- **THEN** only the ellipse of the camera is drawn and the corners show what is below

#### Scenario: Cut-out over a game
- **WHEN** a game capture item keeps its whole area and cuts a rectangle over the minimap
- **THEN** the source below the game shows through where the minimap was

#### Scenario: Mask follows the item
- **WHEN** a masked item is moved, scaled or rotated
- **THEN** the mask moves, scales and rotates with it

### Requirement: Mask Options
A mask SHALL support invert and on/off, and each shape SHALL support a feather that softens its
edge. Items without a mask SHALL render exactly as before.

#### Scenario: Feathered edge
- **WHEN** a kept rectangle has a 20 px feather
- **THEN** the item fades from opaque to transparent over about 20 px at the rectangle's edge

#### Scenario: Inverted mask
- **WHEN** a mask with a kept circle is inverted
- **THEN** the circle is transparent and the rest of the item is drawn

### Requirement: Edge Effects
A scene item SHALL support a border (color, width; inside, centered or outside the edge), a glow
(color, size, strength; outside, inside or both) and a shadow (color, offset, softness, opacity)
that follow the edge of its mask, or of its visible rectangle when it has no shapes. Effects SHALL
be drawn past the item's bounds where they extend, in the preview and every output.

#### Scenario: Glow around a round camera
- **WHEN** a camera masked to a circle has an outside glow of 30 px
- **THEN** a glow of the chosen color surrounds the circle, outside the camera's rectangle too

#### Scenario: Border without shapes
- **WHEN** an unmasked game capture gets a 4 px outside border
- **THEN** a 4 px border is drawn around its visible rectangle

### Requirement: Mask Edits Are Commands
Every mask and effect change SHALL be a command that can be undone and redone, and masks SHALL be saved with
the scene collection.

#### Scenario: Undo a mask edit
- **WHEN** the user drags a mask handle and then presses Undo
- **THEN** the mask returns to its shape before the drag
