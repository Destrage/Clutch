## ADDED Requirements

### Requirement: Local Overlay Server
Clutch SHALL serve its overlay pages and stream events to them only on the loopback address, and
every request SHALL require the install's secret token. User files SHALL be served only by id,
never by path. The server SHALL run while at least one overlay source exists in the open setup.

#### Scenario: Request without the token
- **WHEN** a request reaches the overlay server without the correct token
- **THEN** it is rejected and nothing is served

#### Scenario: Another computer on the network
- **WHEN** another computer tries to connect to the overlay server
- **THEN** the connection fails, because the server listens on the loopback address only

### Requirement: Alerts Source
The system SHALL provide an Alerts source that shows stream events on the canvas. For each event
kind the user SHALL be able to turn alerts on or off and choose an animation (including video with
transparency), a sound, a text template with font, size and colour, a duration, and for kinds with
amounts a minimum amount. Alerts SHALL play one at a time in arrival order, and the alert sound
SHALL be the source's own audio.

#### Scenario: Sub alert
- **WHEN** a viewer named "sam" subscribes on Twitch and the sub alert is on
- **THEN** the sub animation plays over the scene with "sam subscribed" in the chosen style, and its sound plays on the Alerts source's fader

#### Scenario: Several events at once
- **WHEN** three follows arrive within a second
- **THEN** their alerts play one after another, not on top of each other

#### Scenario: Below the minimum
- **WHEN** the bits minimum is 100 and a viewer cheers 50 bits
- **THEN** no alert plays (the chat still shows the event)

#### Scenario: Test alert
- **WHEN** the user presses Test for the raid alert
- **THEN** the raid alert plays with a placeholder name, without any real event
