## ADDED Requirements

### Requirement: Performance profiles
The studio SHALL offer the profiles Gaming, Balanced and Quality, each setting the encoder,
encoder preset, rate control and quality, stream bitrate and clip-mode buffer length. When the
user changes one of those settings by hand, the profile SHALL show as Custom. A profile SHALL
NOT change canvases, scenes, sources, recording folder, formats or stream keys.

#### Scenario: Choosing a profile
- **WHEN** the user picks Gaming on the Performance page and confirms
- **THEN** recordings, the replay buffer and every stream destination use the hardware
  encoder's fastest preset, and Undo last profile change restores the previous settings

#### Scenario: Custom
- **WHEN** the user changes the encoder preset on the Recording page after choosing Balanced
- **THEN** the Performance page shows Custom

### Requirement: Optimize for this PC
The studio SHALL recommend Gaming or Balanced from the PC's hardware encoders, CPU cores, memory
and GPU, show what it found and every setting it would change, and apply them only after confirmation.

#### Scenario: PC with a hardware encoder
- **WHEN** the user clicks Optimize for this PC on a PC whose GPU has a hardware encoder
- **THEN** the recommendation uses that encoder and lists each change before applying it

#### Scenario: No hardware encoder
- **WHEN** no hardware encoder can be opened
- **THEN** Gaming is recommended with the software encoder's fastest preset, and a warning says
  games will lose frames while recording or streaming

### Requirement: Lighter profile from health alerts
When encoder overload or rendering lag is raised while a game is running, the alert SHALL offer
to switch to the next lighter profile.

#### Scenario: Encoder overload while gaming
- **WHEN** encoder overload is raised during a game on the Balanced profile
- **THEN** the alert offers Use a lighter profile, which switches to Gaming
