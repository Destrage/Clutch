# browser-source Specification

## Purpose
Web pages as sources (overlays, alerts, widgets), rendered by a separate helper process so a page that crashes can't take the studio down.

## Requirements

### Requirement: Browser Source
The system SHALL render a web page (URL or local HTML file) at a configured size and frame rate
as a source, with transparent areas compositing over the scene and page audio going through the
mixer. Custom CSS SHALL apply to the page.

#### Scenario: Transparent overlay
- **WHEN** a browser source shows a page with a transparent background and a red box
- **THEN** the scene beneath shows through around the box

#### Scenario: Reload
- **WHEN** the user refreshes a browser source
- **THEN** the page reloads without recreating the source

### Requirement: Browser Crash Isolation
Pages SHALL run in a separate process. If that process exits unexpectedly, rendering, other
sources and all outputs SHALL continue, the browser sources SHALL report that they are
restarting, and the pages SHALL be restored automatically.

#### Scenario: Helper crash
- **WHEN** the browser helper process is killed while streaming
- **THEN** the stream continues, browser sources show "Browser restarting…", and the pages return
  within a few seconds
