## ADDED Requirements

### Requirement: Stream events in the chat
The combined chat SHALL show follows, subscriptions, gifts, donations and raids from each platform
the user streams to (as each platform allows) as highlighted lines among the messages, with an
optional chime the user can turn off.

#### Scenario: A Twitch follow
- **WHEN** someone follows the connected Twitch channel while Clutch is open
- **THEN** a highlighted line "name followed" with the Twitch badge appears in the chat

#### Scenario: A YouTube Super Chat
- **WHEN** a viewer sends a Super Chat in the live broadcast
- **THEN** a highlighted line shows the name, the amount and their message

#### Scenario: Chime turned off
- **WHEN** the user turns the chime off in the chat header and an event arrives
- **THEN** the line appears without a sound, and the choice is kept after a restart

### Requirement: Kick events through the relay
Clutch SHALL receive a connected Kick channel's follows, subscriptions, gifted subscriptions and
Kicks through the Clutch events relay, which accepts only events signed by Kick and gives each
connection only the events of the channel whose Kick sign-in it presented.

#### Scenario: A Kick follow
- **WHEN** someone follows the connected Kick channel while Clutch is open
- **THEN** Kick sends the event to the relay, the relay verifies it and passes it on, and a
  highlighted "name followed" line with the Kick badge appears in the chat

#### Scenario: A forged event
- **WHEN** a request reaches the relay's webhook without a valid Kick signature
- **THEN** the relay rejects it and nothing is passed on

#### Scenario: Someone else's channel
- **WHEN** a connection presents a Kick sign-in for channel A
- **THEN** it receives only channel A's events, never another channel's

#### Scenario: Relay unreachable
- **WHEN** the relay can't be reached
- **THEN** Kick chat keeps working, Clutch retries with growing pauses, and Kick events resume
  when it's back
