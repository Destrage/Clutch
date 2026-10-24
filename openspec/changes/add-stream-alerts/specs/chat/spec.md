## ADDED Requirements

### Requirement: Typed Stream Events
Every stream event in the combined chat SHALL carry its kind (follow, sub, resub, gift, bits, raid
or paid message), platform, viewer name, and where the platform provides them the amount, months,
tier and message, alongside the chat line text.

#### Scenario: Twitch resub
- **WHEN** Twitch reports a 3-month Tier 2 resub with a message
- **THEN** the event has kind resub, months 3, tier "Tier 2" and the message, and the chat shows the same line as before

### Requirement: Event Sounds
The user SHALL be able to choose a sound file for each event kind. An event SHALL play its kind's
sound, or Clutch's built-in sound for that kind when none is chosen (the system chime only when
the built-in sound is unavailable). The chat's bell button SHALL mute all event sounds.

#### Scenario: Follow sound
- **WHEN** a follow sound is set and someone follows on Kick
- **THEN** that sound plays once, at the event sound volume

#### Scenario: Sound file missing
- **WHEN** the chosen sound file no longer exists
- **THEN** the built-in sound for that kind plays instead and the problem is logged once
