## ADDED Requirements

### Requirement: Stream details per setup
Each setup SHALL keep its own go-live details for each destination. Going live and Save SHALL
store the details in the open setup. The Go live window SHALL pre-fill each section from the open
setup's details, and only without them from the channel's current details or the destination's
last details. Hotkey, macro and remote starts SHALL use the open setup's details.

#### Scenario: Switching games
- **WHEN** the user went live with the Apex setup titled "Apex with friends" in category Apex
  Legends, switches to the PUBG setup that was saved with "PUBG squads" in category PUBG, and
  clicks Go Live
- **THEN** the window shows "PUBG squads" and PUBG on Twitch and Kick, and PUBG's YouTube title and
  description

#### Scenario: Switching back
- **WHEN** the user then switches back to the Apex setup and clicks Go Live
- **THEN** the window shows "Apex with friends" and Apex Legends again, although the channels still
  show PUBG

#### Scenario: Setup without details
- **WHEN** the user opens a new setup that has never been saved or streamed with
- **THEN** the window pre-fills from the channel's current details, else the destination's last
  details, as before

#### Scenario: Hotkey start
- **WHEN** the user starts the stream with a hotkey in the PUBG setup
- **THEN** Clutch applies the PUBG setup's details on every connected platform before the video
  starts

#### Scenario: Existing details carried over
- **WHEN** a user with remembered details starts this version for the first time
- **THEN** those details belong to the open setup and the Go live window shows them as before

### Requirement: Saving stream details
The Go live window SHALL offer Save, which stores the details in the open setup and closes without
changing anything on the platforms or starting the stream. While offline, Stream details SHALL open
the same window with Save only.

#### Scenario: Preparing a setup ahead of time
- **WHEN** the user opens Stream details in the PUBG setup while offline, enters a title and
  category, and clicks Save
- **THEN** the window closes, nothing changes on Twitch, Kick or YouTube, and the next Go live in
  that setup is filled with them

#### Scenario: Setup deleted
- **WHEN** the user deletes a setup
- **THEN** its saved details are removed, and the other setups' details are unchanged
