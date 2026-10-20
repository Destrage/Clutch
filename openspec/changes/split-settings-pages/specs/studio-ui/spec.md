## ADDED Requirements

### Requirement: Settings pages
Settings SHALL be split into pages listed on the left: Stream, Recording, Clips, Virtual camera,
Remote control and Appearance. One page SHALL be shown at a time, and each page SHALL scroll on
its own.

#### Scenario: Switching pages
- **WHEN** the user clicks Clips in the Settings page list
- **THEN** only the replay buffer, clip and auto-clip settings are shown

#### Scenario: Opening at the stream settings
- **WHEN** the user clicks Manage… in the destinations panel next to Go Live
- **THEN** Settings opens at the Stream page

#### Scenario: Reopening
- **WHEN** the user closes Settings on the Recording page and opens it again from the top bar
- **THEN** Settings opens at the Recording page
