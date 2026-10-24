## ADDED Requirements

### Requirement: Local Overlay Server
Clutch SHALL serve its overlay pages and stream events to them only on the loopback address, and
every request SHALL require the install's secret token. User files SHALL be served only by id,
never by path.

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

### Requirement: Live Alert Text
Alert animations SHALL carry no text of their own. For each event the system SHALL fill the
kind's text template with the event's values (viewer name, platform, amount, months, tier,
message) and show it in the animation's text box at the configured times, shrinking it to fit.

#### Scenario: Gift on Kick
- **WHEN** "jo" gifts 5 subs on Kick with the template "{name} gifted {amount}"
- **THEN** the gift animation shows "jo gifted 5 subs" in its text box, with the Kick icon if the template uses it

#### Scenario: Long name
- **WHEN** a viewer with a 25-character name follows
- **THEN** the text shrinks to stay inside the text box

### Requirement: Alert Packs
Alerts SHALL come from packs: per event kind an animation, one or more sounds and text settings.
Clutch SHALL ship a built-in pack, and users SHALL be able to choose another pack or replace
single files per Alerts source without changing the pack. A kind with several sounds SHALL play
them in turn or at random, and a kind MAY have a bigger variant used from a set amount.

#### Scenario: Replace one sound
- **WHEN** the user picks their own sound for follows on an Alerts source
- **THEN** follows play that sound and every other kind keeps the pack's files

#### Scenario: Big gift
- **WHEN** the gift kind has a big variant from 10 and someone gifts 20 subs
- **THEN** the big gift animation and sound play
