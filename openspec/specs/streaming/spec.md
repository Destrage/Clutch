# streaming Specification

## Purpose
Sending the program to streaming destinations over RTMP(S), SRT, RIST and WHIP, one or many at once, with reconnects and protected stream keys.

## Requirements

### Requirement: Live Streaming
The system SHALL stream the program canvas to one or more destinations at the same time. Each
destination uses RTMP, RTMPS, SRT, RIST or WHIP (chosen by its URL) and has its own key, codec,
bitrate, audio and reconnect settings.

#### Scenario: RTMP stream
- **WHEN** the user goes live to an RTMP server with a stream key
- **THEN** the server receives H.264 (or HEVC/AV1 via Enhanced RTMP) video and AAC audio at the
  canvas resolution and frame rate

#### Scenario: SRT stream
- **WHEN** the destination URL starts with `srt://`
- **THEN** the stream is sent as MPEG-TS over SRT

#### Scenario: Two platforms at once
- **WHEN** two destinations are enabled and the user goes live
- **THEN** both servers receive the stream

#### Scenario: Shared encoder
- **WHEN** two destinations have identical encoder settings
- **THEN** a single video encoder feeds both

#### Scenario: Incompatible settings
- **WHEN** one destination is invalid (e.g. missing stream key) and another is valid
- **THEN** the valid destination goes live and the invalid one reports what to change

### Requirement: Network Resilience
Streaming SHALL NOT block rendering or encoding. When the network cannot keep up, the system
SHALL drop video from the send queue at keyframe boundaries, keep audio, and count the drops.

#### Scenario: Congestion
- **WHEN** the queued video exceeds the drop threshold
- **THEN** video packets up to the next keyframe are dropped, audio packets are kept, and the
  network-dropped counter increases

### Requirement: Automatic Reconnect
When the connection fails, the system SHALL reconnect with exponential back-off up to a
configurable number of attempts, starting each new connection with a keyframe.

#### Scenario: Server comes back
- **WHEN** the server is unreachable at first and becomes reachable later
- **THEN** the stream shows "Reconnecting", then connects and delivers video starting at a keyframe

#### Scenario: Stop while connecting
- **WHEN** the user stops while a connection attempt is hanging
- **THEN** the stream stops within 2 seconds

### Requirement: Stream Key Protection
The stream key SHALL NOT be stored in plain text or written to logs.

#### Scenario: Settings file
- **WHEN** a stream key is saved
- **THEN** the settings file contains only an encrypted form of it, and log lines show the URL
  with the key redacted

### Requirement: Destination Isolation
A destination's network problems, reconnects or failure SHALL NOT interrupt other destinations.
Each destination SHALL be stoppable and startable on its own while live.

#### Scenario: One destination fails
- **WHEN** one destination's server is unreachable
- **THEN** that destination reports an error and the others keep streaming

#### Scenario: Stop one destination
- **WHEN** the user stops one destination while live
- **THEN** its connection ends and the other destinations continue uninterrupted

#### Scenario: Add a destination while live
- **WHEN** the user starts another destination while already live
- **THEN** it connects and starts at a keyframe without restarting the others

### Requirement: Settings Migration
Settings from the single-stream format SHALL be migrated to one destination, keeping its server
and stream key.

#### Scenario: Upgrade
- **WHEN** settings with a single configured stream are loaded
- **THEN** they appear as one enabled destination with the same server and key

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
