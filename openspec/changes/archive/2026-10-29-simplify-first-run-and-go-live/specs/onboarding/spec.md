## ADDED Requirements

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
