## ADDED Requirements

### Requirement: Audio Dropout Reporting
Each recording, replay and stream output SHALL count the milliseconds of audio it filled with
silence because the audio never arrived. The Stats view SHALL show each output's total, and the
health monitor SHALL raise a warning as soon as an output fills a gap (a problem when more than
one second was filled in the last 10 seconds), clearing it after 10 seconds without gaps.

#### Scenario: A stall during a stream
- **WHEN** the audio mixer stalls for 400 ms while streaming
- **THEN** the stream output's audio gaps grow by about 300 ms (beyond the 100 ms caught up)
- **AND** a warning that audio dropped out on the stream appears, with advice

#### Scenario: Gaps stop
- **WHEN** no audio is filled for 10 seconds
- **THEN** the warning clears and the total stays visible in the Stats view
