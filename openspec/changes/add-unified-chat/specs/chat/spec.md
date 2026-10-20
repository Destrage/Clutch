## ADDED Requirements

### Requirement: Combined chat
The chat panel SHALL show messages from every platform the user streams to that has chat
(Twitch, YouTube, Kick) in one list, in the order they arrive, each marked with its platform,
the sender's name and the time.

#### Scenario: Messages from two platforms
- **WHEN** the user is live on Twitch and Kick and a viewer writes on each
- **THEN** both messages appear in the one list, each with its platform's badge

#### Scenario: Reading while scrolled up
- **WHEN** the user has scrolled up in the list and new messages arrive
- **THEN** the list stays where it is and a "New messages" button jumps to the newest

#### Scenario: Kick's feed changes
- **WHEN** Kick refuses the chat feed connection
- **THEN** Clutch retries with growing pauses and the panel says Kick chat can't be read right now,
  while the other platforms keep working

#### Scenario: YouTube without a connected account
- **WHEN** the user streams to YouTube without connecting the account
- **THEN** the panel says "Connect YouTube in Settings → Stream to see its chat here"

### Requirement: Replying from Clutch
The chat panel SHALL have a reply box that sends a message to all platforms at once or to one
chosen platform, through each platform's official API with the user's connected account.

#### Scenario: Reply to everyone
- **WHEN** the user types a message with "All platforms" chosen and presses Enter
- **THEN** it is sent to every platform the user is connected to, and each one that can't send
  (not connected, rejected) is named in a short note

#### Scenario: Connecting Kick
- **WHEN** the user clicks Connect for a Kick destination and approves Clutch in the browser
- **THEN** the destination shows the connected account and replies can go to Kick

### Requirement: Platform chat pages
Each platform's own chat page SHALL stay available from the chat panel, for actions the combined
list doesn't offer (moderation, polls, signing in to the platform's chat).

#### Scenario: Opening a platform's page
- **WHEN** the user picks Twitch in the panel's Pages menu
- **THEN** Twitch's own pop-out chat for the channel opens in the panel, with a way back to the
  combined list
