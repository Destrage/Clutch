## ADDED Requirements

### Requirement: Emotes as images
The combined chat SHALL show Twitch and Kick emotes as images where they appear in the message,
animated when the emote is animated. It SHALL show YouTube's standard emoji in colour. While an
image loads, or when it can't be loaded, the emote's name SHALL be shown instead.

#### Scenario: Twitch emote
- **WHEN** a Twitch message contains Kappa sent as an emote
- **THEN** the chat shows the Kappa image in its place in the text

#### Scenario: Kick emote
- **WHEN** a Kick message contains `[emote:37234:Prayge]`
- **THEN** the chat shows the Prayge image, not the code

#### Scenario: Image unavailable
- **WHEN** an emote's image can't be downloaded
- **THEN** the chat shows the emote's name as text

### Requirement: Third-party emotes
When a channel's chat starts being read, Clutch SHALL load that channel's 7TV, BetterTTV and
FrankerFaceZ emotes and the global ones, and SHALL show words that match them as the emote. A
setting SHALL turn this off.

#### Scenario: 7TV emote on Twitch
- **WHEN** a Twitch viewer types a word that is one of the channel's 7TV emotes
- **THEN** the chat shows that emote's image, animated if it is animated

#### Scenario: Turned off
- **WHEN** the user turns off third-party emotes
- **THEN** those words are shown as plain text

#### Scenario: Service down
- **WHEN** a third-party service doesn't answer
- **THEN** the chat keeps working without that service's emotes

### Requirement: Emote cache and animation
Each emote image SHALL be downloaded once and kept in a size-capped disk cache. Animated emotes
SHALL play only while the chat is visible, and SHALL show their first frame when animation is
turned off.

#### Scenario: Same emote many times
- **WHEN** fifty messages use the same emote
- **THEN** its image is downloaded once

#### Scenario: Chat hidden
- **WHEN** the chat panel is hidden or Clutch is minimised
- **THEN** animated emotes stop updating
