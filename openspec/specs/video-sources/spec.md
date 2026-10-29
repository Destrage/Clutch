# video-sources Specification

## Purpose
Capture sources (display, window, game capture, camera) and how they behave, including game capture sound and window latching on Windows.

## Requirements

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

### Requirement: Game capture window latching
In automatic fullscreen mode, Game capture SHALL latch onto an acquired game window and remain
attached while that window exists and its process is running. Focus shifts, alt-tabbing, and
temporary minimization SHALL NOT switch capture to another application.

#### Scenario: User alt-tabs during gameplay
- **WHEN** Game capture is capturing a fullscreen game and the user alt-tabs to another window
- **THEN** Game capture remains attached to the game window and does not capture the foreground app

#### Scenario: Game process exits
- **WHEN** the latched game process terminates
- **THEN** Game capture releases the latch and waits for the next foreground fullscreen application

### Requirement: Windows process priority
On Windows systems, the application SHALL configure its process priority to Above Normal on
startup to prevent the operating system scheduler from starving compositing workloads.

#### Scenario: Process startup on Windows
- **WHEN** the application starts on Windows
- **THEN** the process priority class is set to Above Normal without requiring elevated privileges
