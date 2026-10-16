## ADDED Requirements

### Requirement: Unified Command Bus
The engine SHALL apply all project state mutations through a single serializable command bus.
The UI, hotkeys, remote API, macros and plugins SHALL all submit commands through the same bus.

#### Scenario: UI edit goes through the bus
- **WHEN** the user moves a scene item in the preview
- **THEN** an `UpdateItem` command with a transform patch is submitted to the engine
- **AND** the engine emits a change event naming the affected item and publishes a new project snapshot

#### Scenario: Invalid command is rejected
- **WHEN** a command references a source ID that does not exist
- **THEN** the engine rejects the command with a typed error
- **AND** project state is unchanged

### Requirement: Transactions
The engine SHALL support grouping multiple commands into an atomic transaction that either applies
completely or not at all.

#### Scenario: Partial failure rolls back
- **WHEN** a transaction of three commands is submitted and the third fails
- **THEN** the effects of the first two commands are reverted
- **AND** no event for the failed transaction is emitted

### Requirement: Non-Blocking UI Communication
The engine SHALL publish state to consumers as immutable snapshots and events, so readers never
block the engine or render threads.

#### Scenario: Slow consumer does not stall rendering
- **WHEN** a UI consumer stops reading events for 5 seconds
- **THEN** the video thread continues to render at the configured frame rate
- **AND** the consumer receives the latest snapshot when it resumes

### Requirement: Monotonic Engine Clock
The engine SHALL provide a single monotonic clock with nanosecond resolution. Every video and
audio timestamp in the system SHALL use it.

#### Scenario: Timestamps are monotonic
- **WHEN** frames are produced continuously for one hour
- **THEN** every frame timestamp is strictly greater than the previous one

### Requirement: Fixed-Rate Frame Clock
The engine SHALL render each canvas at its configured rational frame rate (e.g. 30, 60, 60000/1001)
without cumulative drift. It SHALL count frames that miss their deadline as lagged.

#### Scenario: Stable pacing
- **WHEN** a canvas is configured at 60 fps on an idle system for 10 minutes
- **THEN** the p99 frame interval jitter is below 1 ms
- **AND** the number of rendered frames equals elapsed time × 60 ± 1

#### Scenario: Overload is reported, not accumulated
- **WHEN** rendering a frame takes longer than one frame interval
- **THEN** the next frame deadline stays aligned to the original schedule
- **AND** the lagged-frame counter increments

### Requirement: Real-Time Thread Discipline
Video and audio threads SHALL NOT perform blocking I/O, SHALL NOT allocate heap memory in steady
state, and SHALL NOT wait on locks held by other threads.

#### Scenario: Steady-state allocation check
- **WHEN** a static scene renders for 60 seconds under an allocation-counting test allocator
- **THEN** the video thread performs zero heap allocations after the first second

### Requirement: Diagnostics and Tracing
The engine SHALL emit structured logs and per-frame tracing spans. Logs SHALL be written to a
rotating log file in the user data directory.

#### Scenario: Log file created
- **WHEN** the application starts
- **THEN** a log file with a timestamped name is created in the logs directory
- **AND** log files beyond the 10 most recent are deleted
