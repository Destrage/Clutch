## ADDED Requirements

### Requirement: Live Statistics
The system SHALL sample, once per second, the frame rate, render time and skipped frames of
every canvas; the frame and drop counts and bitrate of every encoder; the bitrate, dropped
frames, send delay and reconnects of every live destination; and the CPU, memory, GPU and
free disk space of the machine. It SHALL keep the last 5 minutes of history and show it in a
Stats view.

#### Scenario: Stats while recording
- **WHEN** the user opens Stats while recording
- **THEN** they see the current values with sparklines, plus a row for the recording's encoder
  with its frame rate, dropped frames and bitrate

### Requirement: Health Alerts
The system SHALL detect rendering lag, encoder overload, network congestion and reconnects,
sustained high CPU or GPU load, and low disk space while recording. It SHALL raise an alert
only after the condition persists and clear it after it has subsided. Each alert SHALL explain
the problem and suggest a fix. The studio SHALL show the overall health at all times and
announce each new alert once.

#### Scenario: Encoder can't keep up
- **WHEN** the encoder drops more than 1 % of frames for several seconds
- **THEN** the status bar turns to a warning, a toast says the encoder is overloaded, and Stats
  suggests a hardware encoder or a faster preset

#### Scenario: Problem goes away
- **WHEN** the condition behind an alert stops for the calm period
- **THEN** the alert clears and the status bar returns to good

#### Scenario: Disk filling up
- **WHEN** recording with less than 10 minutes of space left at the current bitrate
- **THEN** a problem alert shows the minutes remaining and suggests freeing space
