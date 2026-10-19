## ADDED Requirements

### Requirement: Chat panel
The studio SHALL show a chat tab for each platform the user streams to, displaying that
platform's own pop-out chat page, in which the user can sign in and chat.

#### Scenario: Chatting on Kick
- **WHEN** the user has a Kick destination with a channel name and opens the Kick chat tab
- **THEN** Kick's pop-out chat for that channel is shown and accepts clicks and typing

#### Scenario: Sign-in is remembered
- **WHEN** the user signs in inside a chat tab and restarts Clutch
- **THEN** the chat tab is still signed in

### Requirement: Viewer widget
While streaming, the top bar SHALL show the total number of viewers across platforms that
report one, and a per-platform breakdown when clicked.

#### Scenario: Kick viewers
- **WHEN** the user is live on Kick
- **THEN** the widget shows the channel's current viewer count, refreshed about every 30 seconds

#### Scenario: Platform without a count
- **WHEN** a platform's account is not connected and it offers no public count
- **THEN** the breakdown lists the platform with "Connect your account to see viewers"

### Requirement: Connected accounts
Users SHALL be able to connect their Twitch and YouTube accounts, which enables viewer counts
for those platforms, the Twitch chatters list, and sending YouTube chat messages.

#### Scenario: Connecting Twitch
- **WHEN** the user clicks Connect for a Twitch destination and approves the code shown
- **THEN** the destination shows the connected account and its viewer count appears in the widget
