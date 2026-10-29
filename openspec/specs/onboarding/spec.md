# onboarding Specification

## Purpose
The first run and going live: a setup chosen by purpose, optional setup parts, and a two-step Go Live box.

## Requirements

### Requirement: Purpose-Based First Run
On first run the system SHALL ask what the user wants to do: stream games, talk to camera,
record videos, or make vertical video. It SHALL create a ready-to-use scene for that purpose
from the detected devices: primary screen, game capture, webcam, desktop audio, and the default
microphone with noise suppression enabled. Before creating it, it SHALL show what it found.

#### Scenario: Gamer with a webcam
- **WHEN** a new user picks "Streaming games" on a PC with a webcam
- **THEN** the scene shows the screen, switches to the game when one is fullscreen, places the
  webcam in a corner, and has desktop audio and a noise-suppressed microphone

#### Scenario: No webcam
- **WHEN** a new user picks "Talking to camera" and no webcam is connected
- **THEN** the screen is used instead, and the wizard says no webcam was found

### Requirement: Two-Step Go Live
When the user clicks Go Live with no stream destination, the system SHALL ask only for the
platform and the stream key. It SHALL fill in the server, encoder and bitrate automatically,
and it SHALL recognise the platform from the key's format. A button SHALL open the platform's
page where the key is found.

#### Scenario: First stream to Twitch
- **WHEN** the user clicks Go Live, pastes a key starting with `live_` and clicks Go Live
- **THEN** Twitch is selected, the destination is saved with Twitch's server and a 6000 kb/s
  bitrate for 1080p60, and the stream starts

#### Scenario: Server pasted as the key
- **WHEN** the user pastes an `rtmp://` address into the key field
- **THEN** the box explains that this is the server address and asks for the key instead

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
