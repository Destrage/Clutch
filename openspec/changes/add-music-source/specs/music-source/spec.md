## ADDED Requirements

### Requirement: Music source
The studio SHALL offer a Music source that plays Creative Commons music from Jamendo by genre,
mood or tags, into the mix like any audio source, with play/pause and skip.

#### Scenario: Background music
- **WHEN** the user adds a Music source and picks "Lo-fi"
- **THEN** lo-fi tracks from Jamendo play one after another, with their own sound knob

### Requirement: Licenses and credit
The Music source SHALL by default offer only tracks whose license allows commercial use, and SHALL
always show the current track's title, artist and license so it can be credited.

#### Scenario: Safe for monetized streams
- **WHEN** "Safe for monetized streams" is on
- **THEN** no track with a NonCommercial license is played

#### Scenario: Now playing
- **WHEN** a track starts
- **THEN** its title, artist and license are shown in the source's settings and, when the
  now-playing line is on, in the picture

### Requirement: Without a client ID
A build without a Jamendo client ID SHALL show the Music source as unavailable with the reason,
and nothing else SHALL change.

#### Scenario: Source-built copy
- **WHEN** Clutch was built without `CLUTCH_JAMENDO_CLIENT_ID`
- **THEN** the Music source says it needs a Jamendo client ID and plays nothing
