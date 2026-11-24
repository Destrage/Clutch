## ADDED Requirements

### Requirement: HDR Screen Capture
On Windows, when the captured monitor has HDR on, display, window and game capture SHALL
capture in 16-bit float and map the picture to SDR. The mapping SHALL scale by the monitor's SDR
white level, so SDR content keeps its on-screen brightness, and SHALL compress brighter content
up to the monitor's peak without clipping, keeping hues. Captures SHALL follow HDR being
toggled and windows moving between monitors.

#### Scenario: Desktop on an HDR monitor
- **WHEN** the user captures a monitor with HDR on and the SDR content brightness slider raised
- **THEN** the desktop in the preview has the same brightness and contrast as on the screen,
  not brighter or washed out

#### Scenario: HDR game highlights
- **WHEN** a game renders in HDR with highlights at four times SDR white
- **THEN** the highlights in the recording show detail below full white instead of a clipped
  white area, and darker parts of the picture are unchanged

#### Scenario: HDR switched off
- **WHEN** the user turns HDR off in Windows while a capture runs
- **THEN** the capture continues in 8-bit with no mapping and no gap in the picture longer than
  one second

### Requirement: HDR Mapping Option
Each display, window and game capture source SHALL offer an "HDR screens" option: Automatic
(the default, the mapping above) or As Windows gives them (the 8-bit picture without mapping).
The option SHALL be saved with the source.

#### Scenario: Mapping turned off
- **WHEN** the user sets HDR screens to "As Windows gives them" on a source
- **THEN** that source shows the 8-bit picture Windows provides, and other sources keep the mapping
