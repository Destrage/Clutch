# Design: Go live with your stream's details

## Flow
1. Go Live (button) with at least one enabled destination whose account is connected, and
   "Go live without asking" off → the window opens and loads each platform's details in the
   background (a spinner per section; the rest of the window is usable).
2. Go live in the window → a background job per platform applies the details (YouTube: create or
   update, then bind). The button shows "Setting up YouTube…" and so on.
3. When every job has finished, the video starts on every destination. When a job failed, the window
   stays open with the error on that section and two buttons: **Try again**, and **Go live without
   <platform>** (starts the others).
4. Without the window (skip setting, hotkey, macro, remote API), step 2 runs with the remembered
   details and step 3's failure becomes a toast. Video for that platform's destination still starts,
   so a remote "start stream" never stalls. YouTube then reports the problem through the live status
   checker as today.

Nothing here touches the real-time threads. All API calls run on short-lived worker threads and
report back over channels polled by the UI tick, like the YouTube status checker.

## YouTube
- **List:** `liveBroadcasts.list?broadcastStatus=upcoming` and `active`
  (`part=snippet,status,contentDetails`). Together with `liveStreams.list` (mine), this shows which
  broadcasts use the destination's key.
- **New:** `liveBroadcasts.insert` with `scheduledStartTime` = now, the details, `enableAutoStart`,
  `enableAutoStop`, `latencyPreference` and `monitorStream.enableMonitorStream=false`. Then
  `liveBroadcasts/bind` to the destination's stream. The category and made-for-kids flag live on the
  video, not the broadcast: `videos.update` (`snippet.categoryId`, `status.selfDeclaredMadeForKids`),
  using the broadcast id as the video id.
- **Existing, edited:** `liveBroadcasts.update` (snippet, status) and `videos.update` for the
  category. When it's bound to another of the user's streams, the destination switches to that
  stream's server and key, as `use_youtube_key` does now. When it isn't bound, it's bound to the
  destination's stream.
- **Categories:** `videoCategories.list?regionCode=<channel country or US>`, cached for the session.
- **Auto-start off:** the broadcast is left waiting, and the existing "Start broadcast" toast
  covers it.
- **Cleanup:** the created broadcast id is kept until YouTube reports it live. When the stream ends
  first, Clutch reads the broadcast once more and deletes it only if its `lifeCycleStatus` is still
  `created`, `ready` or `testing`, and it has no `actualStartTime`.
- Beta 2's `ensure_broadcast` inside the status checker and the reconnect after it are removed.
  The checker goes back to only reporting.

## Twitch
- Read: `GET helix/channels?broadcaster_id=` gives `title`, `game_id`, `game_name`, `tags` and
  `broadcaster_language`.
- Write: `PATCH helix/channels?broadcaster_id=` with the changed fields. This needs
  `channel:manage:broadcast`.
- Categories: `GET helix/search/categories?query=` as the user types (debounced, 300 ms).
- Tags: up to 10, each at most 25 characters, letters and digits only. They're checked in the
  window.

## Kick
- Read: `GET /public/v1/channels` gives `stream_title` and `category`.
- Write: `PATCH /public/v1/channels` with `stream_title` and `category_id`. This needs `channel:write`.
- Categories: `GET /public/v1/categories?q=`.

## Settings
Each destination gets an optional `details` object, saved in settings.json:

```json
"details": {
  "title": "", "description": "", "category": { "id": "", "name": "" },
  "youtube": { "privacy": "public", "made_for_kids": false, "latency": "normal",
               "auto_start": true, "auto_stop": true },
  "twitch": { "tags": [], "language": "en" }
}
```

A new top-level setting, `ask_details` (default on), controls whether the window opens.

Pre-filling:
- **A new YouTube broadcast** uses the saved details, else the channel's last broadcast.
- **Twitch and Kick** keep the title on the channel itself, which may have changed in their
  dashboards since. So the window shows the channel as it is now, and falls back to the saved
  details only when it can't be read.

## Edit while live
The live panel's **Edit details** opens the same window in "live" mode:
- no broadcast choice;
- YouTube's latency and auto-start/auto-stop are read-only, because YouTube doesn't allow changing
  them once live;
- **Save** applies the changes without reconnecting.

## Alternatives considered
- **Keep the quiet auto-create and improve its message.** It still makes broadcasts the user never
  saw, and it still has to reconnect after the video started. Rejected.
- **Always open YouTube Studio's Stream page in the browser before going live.** It's an extra
  window and does nothing for Twitch or Kick. Rejected.
