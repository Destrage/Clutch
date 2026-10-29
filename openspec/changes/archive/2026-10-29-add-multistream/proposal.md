# Change: Multistream to several destinations at once

## Why
Creators often go live on several platforms at once (e.g. Twitch + YouTube + Kick). eStream should
do it natively and share encoders between outputs where possible, so it uses fewer GPU encoder sessions and less GPU time.

## What Changes
- **streaming** capability (modified): a list of **destinations** replaces the single stream
  target. Each destination has its own:
  - name and on/off switch;
  - service, server and stream key (encrypted);
  - codec, bitrate and audio settings;
  - reconnect policy.
- Destinations with identical encoder settings **share one encoder**. Each gets its own send
  queue, connection, congestion dropping, reconnect, stats and error state.
- One destination failing or reconnecting never affects the others.
- While live, a single destination can be stopped or (re)started without interrupting the rest.
- Studio:
  - Settings → Stream becomes a destination list (add, remove, rename, enable) plus an editor for
    the selected destination.
  - Go Live starts all enabled destinations.
  - A live panel next to Go Live shows each destination's state, bitrate and drops, with
    per-destination Stop/Start.
  - The status bar shows a compact summary.
- Existing single-stream settings are migrated to one destination.

## Out of scope
- Per-destination canvas (e.g. a vertical canvas for short-form platforms): canvases are single
  in the studio today; this belongs to the multi-canvas change.
- Dynamic bitrate.

## Impact
- Affected specs: `streaming` (modified)
- Affected code: `estream-media` (`MultiStream`, per-destination stats, encoder sharing),
  `apps/estream` (settings model and migration, settings UI, live panel, status bar)
