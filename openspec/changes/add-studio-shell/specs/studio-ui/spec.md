## ADDED Requirements

### Requirement: Main Window Layout
The application SHALL present a main window with a top bar, a scenes panel, a live preview, an
inspector panel, a sources (layers) panel and an audio mixer area. Panels SHALL be resizable and
collapsible, and the layout SHALL persist across restarts.

#### Scenario: Layout persists
- **WHEN** the user resizes the inspector and restarts the app
- **THEN** the inspector has the same size as before

#### Scenario: Reset layout
- **WHEN** the user runs "Reset layout"
- **THEN** all panels return to their default positions and sizes

### Requirement: Live Preview
The application SHALL display the active canvas in a live preview that updates every engine
frame. Capture-to-display latency SHALL be at most 2 frames, and frames SHALL NOT be read back to CPU memory.

#### Scenario: Preview follows the engine
- **WHEN** a moving test pattern is rendered at 60 fps
- **THEN** the preview shows the pattern updating smoothly without visible stutter

#### Scenario: Zoom and pan
- **WHEN** the user scrolls with Ctrl held over the preview
- **THEN** the preview zooms around the cursor position
- **AND** holding Space while dragging pans the view

### Requirement: On-Canvas Editing
The preview SHALL let users select, move, resize, rotate and crop scene items with direct
manipulation, snap to canvas edges, centers and other items, and nudge with the arrow keys.

#### Scenario: Snap to center
- **WHEN** the user drags an item near the horizontal center of the canvas
- **THEN** the item snaps to the center and a guide line is shown

#### Scenario: Crop with modifier
- **WHEN** the user Alt-drags the left handle of a selected item
- **THEN** the item's left crop changes instead of its scale

#### Scenario: Edits are undoable
- **WHEN** the user rotates an item and presses Ctrl+Z
- **THEN** the item's rotation is restored

### Requirement: Inline Inspector
The application SHALL show the properties of the current selection in a non-modal inspector,
and changes SHALL apply immediately.

#### Scenario: Instant apply
- **WHEN** the user changes the opacity slider of a selected item
- **THEN** the preview reflects the new opacity while dragging, without confirmation dialogs

### Requirement: Scenes and Sources Management
The application SHALL let users create, rename, duplicate, reorder and delete scenes and scene
items, toggle visibility and lock, and group items.

#### Scenario: Inline rename
- **WHEN** the user double-clicks a scene name
- **THEN** the name becomes editable in place and Enter confirms the rename

#### Scenario: Locked item
- **WHEN** an item is locked
- **THEN** it cannot be moved or resized in the preview but can still be edited in the inspector

### Requirement: Command Palette
The application SHALL provide a command palette (Ctrl+K) that fuzzy-searches every registered
action, scene and source, and shows each action's keyboard shortcut.

#### Scenario: Switch scene from palette
- **WHEN** the user presses Ctrl+K, types "brb" and presses Enter
- **THEN** the "BRB" scene becomes active

### Requirement: Theming
The application SHALL offer dark, light and follow-system themes and a configurable accent color.
All UI colors SHALL come from theme tokens.

#### Scenario: Follow system theme
- **WHEN** the theme is set to "system" and the OS switches to light mode
- **THEN** the application switches to the light theme without restarting

### Requirement: First-Run Setup
On first launch the application SHALL run a setup wizard that detects the GPU and available
hardware encoders and lets the user choose a canvas preset.

#### Scenario: Preset applied
- **WHEN** the user picks "Vertical 1080×1920 @ 30" in the wizard
- **THEN** the default canvas is created with that resolution and frame rate

### Requirement: Performance Status
The application SHALL show engine FPS, render time, lagged frames, and CPU and GPU usage in a status area.

#### Scenario: Lag is visible
- **WHEN** the engine reports lagged frames
- **THEN** the status area shows the lagged frame count highlighted with the warning color

### Requirement: Accessibility
All interactive controls SHALL be reachable by keyboard and SHALL expose accessible names and roles
to assistive technologies.

#### Scenario: Keyboard-only navigation
- **WHEN** the user presses Tab repeatedly from the top bar
- **THEN** focus moves through every panel's controls in a logical order with a visible focus ring
