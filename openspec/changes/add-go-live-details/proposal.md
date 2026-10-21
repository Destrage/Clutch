# Change: Go live with your stream's details

## Why
On YouTube a stream key doesn't put anyone on the air. The video has to land on a broadcast (the
watch page with its title, privacy, chat and later the recording), each broadcast is used once,
and YouTube only prepares the next one when YouTube Studio's Stream page is opened. Clutch
0.4.0 beta 1 and 2 cover this by quietly creating a broadcast when none is waiting. In a real test
that went wrong:
- The broadcast was made after the video was already flowing, so YouTube never started it and it
  stayed on the channel as a public "Upcoming" stream.
- Opening Studio's Stream page then made a second broadcast, which failed with "Stream key is
  currently assigned".
- The user learned all this from YouTube Studio rather than from Clutch, and didn't recognise the
  broadcast as their own.

Streamers also change their title and category between streams on every platform, and today that
means a trip to each platform's dashboard.

## What Changes
- **Go live window.** Clicking Go Live, when a destination being streamed to has its account
  connected, opens a window with one section per such platform. Destinations without a connected
  account (custom servers, Facebook) are listed as "streams with your key". Go live in the window
  applies the details, then starts the video.
- **YouTube: choose or create the broadcast.**
  - Lists the channel's upcoming and ready broadcasts, with title, scheduled time, privacy and
    whether it uses this destination's key.
  - **New broadcast** is always offered, also when some exist, and is preselected when none does.
  - A new broadcast has a title, description, privacy, category, made-for-kids, latency (normal,
    low, ultra-low), and auto-start and auto-stop. Fields are pre-filled from the last details used.
  - Choosing an existing broadcast shows its details for editing; changes are saved to it.
- **YouTube is set up before the video starts.** Clutch creates or updates the broadcast, binds it to
  the destination's stream (or switches the destination to the stream the chosen broadcast
  uses), and only then connects. Auto-start then works as YouTube intends. This replaces 0.4.0
  beta 2's quiet creation after connecting and its reconnect workaround.
- **Twitch and Kick: channel details.** There is no broadcast to create on these platforms; the
  window shows and edits the channel's current details and saves them on Go live:
  - Twitch: title, category, tags, language (Helix `channels`; new scope
    `channel:manage:broadcast`).
  - Kick: title, category (`PATCH /public/v1/channels`; new scope `channel:write`).
- **Remembered, and skippable.**
  - Each destination keeps its last details, which are pre-filled next time.
  - "Go live without asking" (in the window, and in Settings → Stream) uses them without opening
    the window.
  - A hotkey, macro or remote-control "start stream" never opens the window: it uses the
    remembered details, with YouTube still set up before the video starts.
- **Edit while live.** The live panel gets **Edit details**, which changes the title, description,
  category and (YouTube) privacy on the platforms being streamed to, without reconnecting.
- **Clear messages.** After going live Clutch says what it did per platform, for example "YouTube:
  new broadcast 'Ranked tonight' (public)", with a link to it. Errors name the platform and what
  failed, and offer to go live on the other platforms anyway.
- **Cleanup.** A broadcast Clutch created that never went live is deleted when the stream ends,
  after checking with YouTube that it really never went live, so a short stream's recording is
  never deleted.

## Out of scope
- Thumbnails, YouTube playlists, scheduled (future) broadcasts, and Twitch branded content and
  content labels. They can follow in a later change.
- Platforms without an account connection in Clutch.

## Impact
- Affected specs: streaming (go live details; supersedes the "No broadcast to go live on"
  scenario of `add-account-stream-setup`).
- Affected code: `apps/estream`:
  - `youtube.rs`: broadcast list, insert, update, bind, delete, categories;
  - `twitch.rs` and `kick.rs`: channel details and categories;
  - the Go Live window (Slint), the live panel, toggle_stream and the remote "start stream" path;
  - settings, with details per destination.
- YouTube quota: listing costs about 3 units; creating and binding cost about 100. That is well
  within the default 10,000 a day.
- Needs from the project owner: tick **Write channel** (`channel:write`) on the Kick app. Twitch
  and Kick users connect again once for the new permissions; YouTube's permission
  (`youtube.force-ssl`) already covers it.
