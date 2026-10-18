## ADDED Requirements

### Requirement: Game Event Triggers
Macros SHALL be able to start from game events reported by the clip integrations (kill,
multi-kill, ace, round or match win, objective, hype), optionally limited to one game. The
existing rule that a macro already running is not started again SHALL apply.

#### Scenario: Kill shows an overlay
- **WHEN** a macro has the trigger "game event: kill" and the steps "show Kill banner, wait 3 s,
  hide Kill banner"
- **THEN** each kill shows the banner for about 3 s

### Requirement: Save Clip Step
Macros SHALL have a "save clip" step with before and after lengths, which saves a clip through
the same path as the clip hotkey.

#### Scenario: Clip from a macro
- **WHEN** a macro runs a "save clip (20 s before, 5 s after)" step while the replay buffer is
  running
- **THEN** a clip of about 25 s is added to the library
