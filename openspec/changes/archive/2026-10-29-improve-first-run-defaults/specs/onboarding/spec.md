## ADDED Requirements

### Requirement: Optional Setup Parts
The setup wizard SHALL list each optional part of the setup it will build with a checkbox:
webcam, game capture, microphone, desktop audio, chat overlay, alerts, and the "Starting soon"
and "Be right back" scenes. It SHALL leave out every unticked part. The defaults SHALL follow
the purpose: chat overlay, alerts and break scenes on for streaming games and talking to camera,
off for recording and vertical video. A choice the user made SHALL survive a purpose change.

#### Scenario: Streamer without a webcam on screen
- **WHEN** the user picks "Streaming games", unticks the webcam and clicks Get started
- **THEN** the setup has the screen, game, chat overlay and alerts, and no webcam source

#### Scenario: Break scenes
- **WHEN** the user picks "Talking to camera" and keeps the defaults
- **THEN** the setup also has "Starting soon" and "Be right back" scenes with large text, the
  chat overlay and alerts

#### Scenario: Camera purpose with the webcam unticked
- **WHEN** the user picks "Talking to camera" and unticks the webcam
- **THEN** the screen is used, and the list shows it

#### Scenario: Choice kept across purposes
- **WHEN** the user unticks the webcam, then picks another purpose
- **THEN** the webcam stays unticked
