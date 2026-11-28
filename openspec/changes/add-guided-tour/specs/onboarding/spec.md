## ADDED Requirements

### Requirement: Guided Tour
The system SHALL offer a guided tour that highlights, in order, setups, canvases, scenes,
sources, the preview, the inspector, the sound mixer, recording and clips, and Go Live. Each step
SHALL dim the rest of the studio and explain the highlighted part, with Back, Next, a step count
and Skip tour. A step whose part is hidden SHALL show it first, and a step whose part doesn't
exist SHALL be skipped. The tour SHALL not change the user's setup.

#### Scenario: Top level first
- **WHEN** a new user starts the tour
- **THEN** the first step highlights the setup pill and explains that a setup holds everything for one kind of stream, such as one per game

#### Scenario: Single canvas
- **WHEN** the setup has a single canvas and the canvas tabs are not shown
- **THEN** the canvas step explains canvases without a highlight, or is skipped, and the step count matches the steps shown

### Requirement: Skipping and Restarting the Tour
After a new user completes or skips the first-run wizard, the system SHALL offer to start the
tour or skip it. Finishing or skipping SHALL be remembered so the tour does not start on its
own again. Users updating from an earlier version SHALL not be offered it automatically. The
tour SHALL be available at any time from the command palette and the help menu, and SHALL close
when streaming or recording starts.

#### Scenario: Skip
- **WHEN** a new user clicks "Skip, I know my way around"
- **THEN** the studio is shown without the tour, and the tour does not appear on later launches

#### Scenario: Restart later
- **WHEN** the user runs "Take the tour" from the command palette
- **THEN** the tour starts at the first step

#### Scenario: Going live during the tour
- **WHEN** a stream or recording starts while the tour is open
- **THEN** the tour closes
