## ADDED Requirements

### Requirement: Share Links
While streaming, the Live deck and the Live panel SHALL offer a Share window with a watch link for each streamed
destination whose link Clutch can build (Twitch and Kick from the channel name, YouTube from the
broadcast), a message filled from the share template that the user can edit and copy, and buttons
that open a pre-filled post on X, Bluesky, Reddit and WhatsApp in the browser. A destination
without a link SHALL be left out, with where to add it.

#### Scenario: Links for the streamed platforms
- **WHEN** the user streams to Twitch (channel `someone`) and to a YouTube broadcast, and opens Share
- **THEN** the Share window shows `https://www.twitch.tv/someone` and the broadcast's watch link, each
  with Copy

#### Scenario: Copy the message
- **WHEN** the user clicks Copy under the message
- **THEN** the clipboard holds the template filled with the stream's title and links, including
  any edits the user made in the Share window

#### Scenario: Missing title
- **WHEN** the stream has no title and the template is `Live now: {title}\n{links}`
- **THEN** the message holds only the links

#### Scenario: Share page
- **WHEN** the user clicks the X button
- **THEN** the browser opens X's post page with the message filled in

### Requirement: Discord Announcements
Clutch SHALL post the share message to each Discord webhook that has "Post when I go live" on,
once per stream session, after the first destination is live. It SHALL NOT post again for
reconnects, or for a new session within 10 minutes of its last announcement to that webhook.
Posts SHALL NOT mention anyone unless the webhook's "Mention @everyone" is on. A failed post
SHALL show a message naming the webhook and SHALL NOT affect the stream.

#### Scenario: Go live
- **WHEN** the user goes live with a webhook set to post on go-live
- **THEN** one message with the stream's links arrives in that Discord channel

#### Scenario: Reconnect
- **WHEN** a destination reconnects during the stream
- **THEN** nothing more is posted

#### Scenario: Quick restart
- **WHEN** the user stops and goes live again 3 minutes later
- **THEN** nothing is posted automatically, and Post to Discord in Share still posts on request

#### Scenario: Mentions in the title
- **WHEN** the title contains `@everyone` and Mention @everyone is off
- **THEN** the post pings no one

#### Scenario: Deleted webhook
- **WHEN** Discord answers that the webhook no longer exists
- **THEN** Clutch shows that the webhook was deleted and asks for a new URL, and keeps streaming

### Requirement: Webhook Protection
Discord webhook URLs SHALL be saved encrypted like stream keys and SHALL NOT appear in logs.
Clutch SHALL only accept Discord webhook URLs as webhooks.

#### Scenario: Saved settings
- **WHEN** the user adds a webhook and Clutch saves its settings
- **THEN** the settings file holds the URL encrypted, and log lines name the webhook by its label

#### Scenario: Wrong URL
- **WHEN** the user pastes a link that is not a Discord webhook URL
- **THEN** Clutch refuses it and says where to find the webhook URL in Discord
