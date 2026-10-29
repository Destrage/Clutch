## ADDED Requirements

### Requirement: Virtual Camera Output
On Windows 11, the system SHALL provide a webcam named "eStream Virtual Camera" while the virtual
camera is on. It SHALL show the chosen canvas, scaled to the size the consuming app requests and
letterboxed to keep its aspect ratio. Media Foundation and DirectShow apps SHALL be able to use
it. When the virtual camera is off, the camera SHALL disappear.

#### Scenario: Used in a call
- **WHEN** the virtual camera is on and a video call app selects "eStream Virtual Camera" at 1280×720
- **THEN** the app receives the canvas at 1280×720, 30 fps, and eStream shows the camera as in use

#### Scenario: eStream stops sending
- **WHEN** an app is using the camera and eStream exits
- **THEN** the camera is removed and the app reports the camera is gone (no crash in the app)

### Requirement: Work Only While Used
The system SHALL render and copy frames for the virtual camera only while at least one app is
receiving frames from it.

#### Scenario: Camera on, nobody watching
- **WHEN** the virtual camera is on but no app has opened it
- **THEN** no extra frames are read back from the GPU for it

### Requirement: One-Time Installation
The camera component SHALL be installed for all users once, with the user's consent through a
system elevation prompt. After that, starting and stopping the camera SHALL need no elevation.

#### Scenario: First start
- **WHEN** the user starts the virtual camera for the first time
- **THEN** eStream explains why it needs permission, shows the system prompt, and starts the camera
  once it's approved

#### Scenario: Permission refused
- **WHEN** the user declines the prompt
- **THEN** the camera stays off, eStream says why, and everything else keeps working
