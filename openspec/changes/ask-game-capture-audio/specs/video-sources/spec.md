## ADDED Requirements

### Requirement: Asking about game capture sound
A new Game capture SHALL start with "Capture audio" off. When it is added while a Desktop audio or
Application audio source exists, the app SHALL ask whether to capture the game's sound, offering
to remember the answer; with no such source, the capture's audio SHALL be turned on without
asking. Window capture and existing sources SHALL keep their setting.

#### Scenario: Desktop audio already present
- **WHEN** the user adds a Game capture and Desktop audio is in the collection
- **THEN** the app asks whether to capture the game's sound, and the capture's audio follows the
  answer

#### Scenario: Remembered answer
- **WHEN** the user ticks "Don't ask again" and answers
- **THEN** later Game captures follow that answer without asking, until it is changed in Settings

#### Scenario: No other sound source
- **WHEN** the user adds a Game capture and no Desktop or Application audio source exists
- **THEN** the capture's audio is on and no question is asked
