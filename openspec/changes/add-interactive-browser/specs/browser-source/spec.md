## ADDED Requirements

### Requirement: Interactive browser sources
A browser source SHALL have an Interactive setting, off by default. While it is on, pointer and
wheel input over the source in the preview SHALL go to its page at the matching page position,
and keyboard input SHALL go to the page after it has been clicked, until Escape is pressed.
Holding Alt SHALL edit the source like any other.

#### Scenario: Clicking a button on the page
- **WHEN** a browser source is Interactive and the user clicks a button drawn on it in the preview
- **THEN** the page receives the click at that button's position and the item is not moved

#### Scenario: Moving it anyway
- **WHEN** the user holds Alt and drags an Interactive browser source
- **THEN** the item moves as any source would, and the page receives nothing

#### Scenario: Not interactive
- **WHEN** Interactive is off
- **THEN** clicks select and drag the source as before and the page receives no input
