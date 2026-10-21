# Change: Stream details from the connected account, and YouTube's live status

## Why
Streamers paste a server address and stream key for each platform, and on YouTube the stream
can connect while nothing goes live: auto-start is off and the broadcast waits for "Go live" in
YouTube Studio, the key belongs to another stream than the broadcast, or YouTube is still starting.
From the user's side it looks like YouTube randomly doesn't work. The connected accounts can give
Clutch the right details and tell it what YouTube is doing.

## What Changes
- **Get stream key** on a destination whose account is connected (Settings → Stream): fills in the
  server and key from the account, so nothing is copied by hand.
  - YouTube: `liveStreams.list` (mine): the stream bound to the current or upcoming broadcast,
    otherwise the default one; its `cdn.ingestionInfo` address and stream name.
  - Twitch: Helix `GET streams/key` (`channel:read:stream_key`, added to the sign-in).
  - Kick: `GET /public/v1/channels` → `stream.url`, `stream.key` (`streamkey:read`, added to the
    sign-in).
- **YouTube status while live:** with YouTube connected, Clutch checks every 20 seconds (every 2
  minutes once live) which state YouTube is in, and says so on the YouTube row of the Live deck
  and in a toast when it changes:
  - "Live on YouTube";
  - "YouTube is starting your broadcast" (auto-start on, or starting);
  - "Waiting for you to start the broadcast" (auto-start off), with a **Start broadcast** button
    that starts it (`liveBroadcasts.transition` to live);
  - "YouTube isn't receiving video yet";
  - "No YouTube broadcast uses this stream key" (open Studio → Go live);
  - "This stream key isn't one of your YouTube streams" (use Get stream key).
- Quota: two list calls (1 unit each) per check; starting a broadcast is one transition call.

## Impact
- Affected specs: streaming (account stream setup, YouTube live status).
- Affected code: `apps/estream` (`youtube.rs`, `twitch.rs`, `kick.rs`, a YouTube status checker,
  Settings → Stream account row, Live deck rows, toasts).
- Needs from the project owner: tick **Read stream key** on the Kick app. Twitch and Kick users
  connect again once (new permissions); YouTube's permission already covers it.
