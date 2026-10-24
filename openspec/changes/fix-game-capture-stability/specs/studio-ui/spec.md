## ADDED Requirements

### Requirement: Preview viewport scroll panning
The canvas preview SHALL NOT pan when the mouse wheel is scrolled unless preview scroll panning is
explicitly enabled in Settings. Middle mouse button drag and Space plus left mouse button drag
SHALL pan the preview regardless of the setting.

#### Scenario: Mouse wheel scrolled with default settings
- **WHEN** the user scrolls the mouse wheel over the canvas preview and scroll panning is disabled
- **THEN** the preview position does not pan

#### Scenario: Middle mouse button dragged
- **WHEN** the user presses and drags the middle mouse button over the preview
- **THEN** the preview pans following the mouse movement
