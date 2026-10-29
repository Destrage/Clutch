# automation Specification

## Purpose
Macros that run actions when a trigger fires, and global hotkeys that work while another app has focus.

## Requirements

### Requirement: Macros
The system SHALL let users define named macros that are saved with the scene collection. Each
macro SHALL hold an ordered list of steps: switch scene, show/hide/toggle a source, mute/unmute/
toggle a source, start/stop/toggle an output, save a replay, studio transition, run any command
palette action, and wait. Macros SHALL run without freezing the studio, and every change they
make SHALL go through the command bus.

#### Scenario: Run now
- **WHEN** the user runs a macro "BRB" (switch to "Be right back", mute "Mic")
- **THEN** the program switches to "Be right back" and "Mic" is muted

#### Scenario: Wait step
- **WHEN** a macro switches to "Intro", waits 5 s, then switches to "Game"
- **THEN** "Game" goes on program about 5 s after "Intro"

### Requirement: Triggers
Macros SHALL start from triggers: a global hotkey, a window gaining focus (program name or title
text), an interval, a scene going on program, or an output starting or stopping. Disabled macros
SHALL NOT run, and a macro that is still running SHALL NOT be started again.

#### Scenario: Game window takes focus
- **WHEN** a macro has the trigger "window focus: valorant" and the game window becomes the
  foreground window
- **THEN** the macro runs once (not repeatedly while the game stays focused)

#### Scenario: Stream started
- **WHEN** a macro has the trigger "stream started" and the steps "start replay buffer"
- **THEN** going live also starts the replay buffer

### Requirement: Global Hotkeys
Hotkey triggers SHALL work while another application is focused. A hotkey that another program
has already registered SHALL be reported to the user rather than silently ignored.

#### Scenario: Hotkey while gaming
- **WHEN** the user presses Ctrl+F9 bound to "switch to BRB" while a game is focused
- **THEN** the program switches to "BRB"
