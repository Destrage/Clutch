# overlays Specification

## Purpose
Built-in overlay sources, starting with the chat overlay: how messages look, which ones show, and moderation.

## Requirements

### Requirement: Chat Overlay Source
The system SHALL provide a Chat overlay source that shows the combined chat from every connected
platform on the canvas, with platform badges, name colours and emotes, in arrival order. The user
SHALL be able to choose a style, font, size, colours, width, maximum lines and a fade-out time.

#### Scenario: Two platforms at once
- **WHEN** a Twitch message and then a Kick message arrive
- **THEN** both appear on the overlay in that order, each with its platform badge

#### Scenario: Messages fade
- **WHEN** the fade time is 30 s and chat goes quiet
- **THEN** each message disappears 30 s after it arrived and the overlay ends up empty

### Requirement: Chat Overlay Filters
The Chat overlay SHALL be able to hide chosen platforms, command messages starting with `!` and
messages from listed names, and SHALL optionally show stream events as highlighted lines.

#### Scenario: Bot hidden
- **WHEN** "Nightbot" is in the hidden names and Nightbot posts a message
- **THEN** the message shows in the chat panel but not on the overlay

### Requirement: Chat Overlay Moderation
A message removed on its platform, or all messages of a user who is timed out or banned, SHALL be
removed from the Chat overlay when the platform reports the removal.

#### Scenario: Deleted message
- **WHEN** a moderator deletes a Twitch message that is on the overlay
- **THEN** it disappears from the overlay
