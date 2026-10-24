# Change: Chat on stream

## Why
Clutch combines chat from Twitch, Kick and YouTube in one panel, but viewers can't see it on the
stream. Streamers add a chat widget from each platform or a third-party service, each showing
only one platform. Clutch already has the combined chat, with badges and emotes.

## What Changes
- **Chat overlay source:** a new source type, "Chat overlay", showing the combined chat on the
  canvas: platform badge, name in its colour, message with emotes, newest at the bottom.
- **Styles:** a few built-in looks (plain text with outline, boxed lines, compact), with font,
  size, text colour, line background and corner rounding, width and maximum lines.
- **Fading:** messages fade out after a chosen time (or stay); the overlay is empty when chat is
  quiet.
- **Filters:** which platforms to show, hide commands (`!` messages) and known bots (a name list),
  and optionally show stream events (follows, subs) as highlighted lines.
- **Moderation follows the panel:** a message deleted or a user timed out on the platform
  disappears from the overlay too, where the platform reports it.
- Served by the overlay server from `add-stream-alerts` (loopback only, token), rendered by the
  browser source, so emotes and animated emotes work as on the web.

## Impact
- Affected specs: `overlays` (Chat overlay).
- Affected code: `apps/estream` (sends chat messages to the overlay server), `estream-capture`
  (Chat overlay source type on the browser source), bundled overlay page (HTML/CSS/JS).
- Depends on `add-stream-alerts` (overlay server, typed events).
