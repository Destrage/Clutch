## ADDED Requirements

### Requirement: Windows 10 Virtual Camera
On Windows 10, the system SHALL provide "Clutch Virtual Camera" to DirectShow apps through a
video source filter that shows the chosen canvas while the virtual camera is on, and a
placeholder picture otherwise. The filter SHALL be registered only where the Windows 11 Media
Foundation camera is unavailable, so the camera is never listed twice.

#### Scenario: Used in a call on Windows 10
- **WHEN** the virtual camera is on in Clutch on Windows 10 and a call app selects "Clutch Virtual Camera"
- **THEN** the app receives the canvas, and Clutch shows the camera as in use

#### Scenario: Windows 11
- **WHEN** the camera component is installed on Windows 11
- **THEN** only the Media Foundation camera is registered, and apps list "Clutch Virtual Camera" once

#### Scenario: Clutch not sending
- **WHEN** an app opens the camera on Windows 10 while Clutch's virtual camera is off
- **THEN** the app receives the placeholder picture and keeps working
