## ADDED Requirements

### Requirement: Stream details from the connected account
A destination whose platform account is connected SHALL offer to fill in its server and stream key
from that account (YouTube, Twitch, Kick).

#### Scenario: YouTube key from the account
- **WHEN** the user clicks Get stream key on a YouTube destination with YouTube connected
- **THEN** the server and key of their YouTube stream are filled in and saved, and a toast says so

#### Scenario: Missing permission
- **WHEN** the sign-in lacks the permission to read the stream key
- **THEN** Clutch asks the user to connect the account again

### Requirement: YouTube live status
While streaming to YouTube with YouTube connected, Clutch SHALL show whether YouTube is live,
starting, waiting for the broadcast to be started, not receiving video, without a broadcast for the
key, or using a key that isn't the user's, and SHALL offer to start a broadcast that is waiting.

#### Scenario: Auto-start off
- **WHEN** YouTube receives the stream but the bound broadcast has auto-start off and isn't live
- **THEN** the YouTube row says it is waiting to be started and a Start broadcast button starts it

#### Scenario: Wrong key
- **WHEN** the key Clutch streams with is not one of the user's YouTube streams
- **THEN** Clutch says so and points to Get stream key

#### Scenario: Live
- **WHEN** the broadcast goes live
- **THEN** the YouTube row says it is live on YouTube
