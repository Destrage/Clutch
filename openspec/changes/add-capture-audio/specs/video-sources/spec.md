## ADDED Requirements

### Requirement: Capture audio
Game capture and Window capture SHALL capture the audio of the application they show, with a
"Capture audio" setting that is on by default for Window capture (Game capture: see
ask-game-capture-audio). The audio SHALL follow the captured application,
including when "Any fullscreen application" mode switches to another game.

#### Scenario: Game capture with sound
- **WHEN** a game capture captures a fullscreen game
- **THEN** the game's sound appears in the mixer under the game capture's name

#### Scenario: Switching games
- **WHEN** the user closes one game and starts another in "Any fullscreen application" mode
- **THEN** the capture's audio follows the new game without any change in settings

#### Scenario: Application audio already set up
- **WHEN** an Application audio source captures the same application
- **THEN** the capture's audio stays silent, so the game is not heard twice

### Requirement: Desktop audio leaves out captured games
Desktop audio on the default output device SHALL leave out the application whose audio a game or
window capture is capturing, so that its sound is heard only once.

#### Scenario: Game and desktop audio together
- **WHEN** a game capture captures a game's audio and Desktop audio is on
- **THEN** Desktop audio carries every other app's sound but not the game's
