## ADDED Requirements

### Requirement: Source Property Schema
Every source type SHALL declare its settings as a typed property schema. The inspector SHALL render
the schema generically, and stored settings SHALL be validated against it.

#### Scenario: Invalid setting rejected
- **WHEN** a command sets a camera FPS value outside the device's supported range
- **THEN** the command is rejected with a validation error and settings are unchanged

### Requirement: Source Availability States
Sources SHALL report an availability state (active, unavailable, error) with a human-readable reason,
and SHALL recover automatically when the underlying device, window or file becomes available again.

#### Scenario: Device reconnect
- **WHEN** a webcam in use is unplugged and plugged back in
- **THEN** the source shows "unavailable" while it is disconnected and resumes automatically after reconnection

### Requirement: Display Capture
The system SHALL capture any connected monitor at its native resolution and refresh rate, with an
optional cursor. Captured frames SHALL stay on the GPU.

#### Scenario: Capture a monitor
- **WHEN** the user adds a display capture for monitor 2
- **THEN** the source renders monitor 2's content at its native resolution

#### Scenario: Low overhead
- **WHEN** a 2560×1440@60 display is captured for 60 seconds
- **THEN** the capture costs less than 1% CPU on the reference machine

### Requirement: Window Capture
The system SHALL capture a single application window, identified by match rules (executable, window
class, title), and SHALL re-acquire the window automatically when it reappears.

#### Scenario: App restarted
- **WHEN** the captured application is closed and relaunched
- **THEN** the window capture resumes on the new window without user action

### Requirement: Game Capture
The system SHALL capture game windows, including borderless-fullscreen games, at the game's frame rate.

#### Scenario: Borderless game
- **WHEN** a game runs in borderless fullscreen and is selected for game capture
- **THEN** the game's frames are rendered in the scene without black frames

### Requirement: Video Capture Devices
The system SHALL capture webcams and capture cards, with selectable resolution, frame rate and pixel
format, and SHALL convert YUV formats to RGB on the GPU with correct color matrix and range.

#### Scenario: Select camera mode
- **WHEN** the user selects 1920×1080 @ 60 MJPEG for a webcam
- **THEN** the source delivers 1080p frames at 60 fps

#### Scenario: Correct colors
- **WHEN** a BT.709 limited-range NV12 test signal is captured
- **THEN** rendered color bars match reference values within ±2 code values

### Requirement: Image Source
The system SHALL display PNG, JPEG and WebP images, and SHALL animate GIF and animated WebP files.

#### Scenario: Animated GIF
- **WHEN** an animated GIF is added as an image source
- **THEN** it plays in a loop using its embedded frame delays

### Requirement: Color Source
The system SHALL provide a solid color source with configurable size and RGBA color.

#### Scenario: Semi-transparent color
- **WHEN** a color source is set to RGBA (0, 0, 0, 128)
- **THEN** it renders as 50% transparent black

### Requirement: Text Source
The system SHALL render text with a selectable font, size, color or gradient, outline, drop shadow,
alignment and word wrap. It SHALL support complex scripts and emoji, and SHALL optionally read
text from a file and update when the file changes.

#### Scenario: Text from file
- **WHEN** a text source reads from a file and that file's content changes
- **THEN** the rendered text updates within 1 second

#### Scenario: Mixed scripts
- **WHEN** the text contains Latin, Arabic and emoji characters
- **THEN** all characters render with correct shaping and fallback fonts
