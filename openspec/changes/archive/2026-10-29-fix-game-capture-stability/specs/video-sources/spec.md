## ADDED Requirements

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
