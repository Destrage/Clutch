## ADDED Requirements

### Requirement: Settings Survive Version Changes
When the settings file contains entries this version can't read, Clutch SHALL keep every other
setting, drop only those entries, and first save a copy of the original file next to it as
`settings.invalid-<date>-<time>.json`. A file that can't be read at all SHALL be copied the same
way before defaults are used.

#### Scenario: Older build meets a newer integration
- **WHEN** the settings list a game integration this build doesn't know
- **THEN** that integration is left out, the destinations, stream keys and account logins are kept, and the original file is copied to `settings.invalid-…json`

#### Scenario: Damaged file
- **WHEN** the settings file isn't valid JSON
- **THEN** Clutch starts with default settings and the damaged file is kept as `settings.invalid-…json`
