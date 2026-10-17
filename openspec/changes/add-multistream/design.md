# Design: Multistream

## Model
```
MultiStream
 ├─ Group (one encoder Session per distinct encoder configuration)
 │    ├─ Destination A ── SendQueue ── sender thread ── network
 │    └─ Destination B ── SendQueue ── sender thread ── network
 └─ Group …
```
- **Grouping key**: `StreamSettings::encoder_settings()`, which covers codec, encoder name,
  bitrate, preset, keyframe interval, audio codec (from the protocol), audio bitrate and track.
  Destinations with equal keys share a group.
- The encoder session's network target is a list of `NetTarget`s. Every packet is pushed to every
  active destination queue. An `ffmpeg::Packet` clone is a reference-counted buffer, so this
  costs no copies.
- **Forced keyframes**: when any destination (re)connects it raises its own flag. The encoder
  ORs the flags, so other destinations in the group get an extra IDR, which is harmless.

## Per-destination control
- `DestinationStats` holds:
  - the name and the redacted URL;
  - bytes sent;
  - network state, attempt count, queue delay and drops;
  - the error.
- Each destination has its own stop flag. FFmpeg's interrupt callback and the send loop check
  "session stopping OR destination stopping".
- **Stopping one destination** sets its flag and closes its queue. The encoder's later pushes are
  ignored, and the sender flushes, writes the trailer and exits (bounded by the 1 s grace).
- **Starting a destination while live**: if a running group has the same encoder key, the
  destination is added through the session's control channel (`Control::AddDestination`). The
  run loop keeps the stream specs for this. Otherwise a new group (session) is started.
- `MultiStream::poll` retires groups whose destinations have all stopped or failed, stopping their
  sessions in the background.

## Failures
- A destination that gives up (max retries, or reconnect disabled) records its error and stops
  itself. The others continue.
- Validation errors (missing key, WHIP with HEVC, …) are reported per destination at start. The
  valid destinations still go live. If none is valid, starting fails.

## Settings (app)
```jsonc
"destinations": [
  { "name": "Twitch", "enabled": true, "stream": { …StreamSettings… }, "key_protected": "dpapi:…" }
]
```
The old `stream` / `stream_key_protected` fields are read once and migrated to a destination
named after the service, then no longer written.

## UI
- **Settings → Stream:**
  - destination list rows show the enabled toggle, name, detected protocol and remove;
  - "Add destination";
  - the existing editor below acts on the selected destination, plus a Name field.
- **Top bar:** Go Live shows the aggregate state: live if any destination is live, otherwise
  connecting or reconnecting. A chevron next to it opens the **live panel**, which shows one row
  per destination (status dot, name, state / bitrate / drops) with Stop or Start.
- **Status bar:** e.g. `● Twitch 6.0 Mb/s · ● YouTube reconnecting`. The health dot shows the
  worst destination.

## Testing
- Two RTMP destinations with the same settings share one encoder (encoder count = 1), and both
  servers receive the stream.
- Two destinations with different bitrates use two encoders.
- One destination pointing at a dead server (reconnect off) fails, and the other keeps streaming.
- Stopping one destination while live: the other server keeps receiving.
- Adding a destination while live: it joins the existing group and its file starts with a
  keyframe.
- Settings migration from the single-stream format.
