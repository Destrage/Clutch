# Design: Chat on stream

## Data flow
The chat panel's feed (`livechat::Feed`) already merges messages from every connected platform,
with emote parts and name colours. Each new message (and each typed stream event, when event
lines are on) is also sent to the overlay server, which forwards it to connected chat pages:

```json
{"type":"chat","id":"…","platform":"twitch","name":"ali","color":"#9146ff",
 "parts":[{"text":"gg "},{"emote":"https://…/emote.webp","alt":"PogChamp"}],
 "time":1792835151886}
{"type":"remove","id":"…"}            // a deleted message
{"type":"clear_user","name":"ali"}    // a timeout or ban
```

Emotes are image URLs from the platform or third-party emote CDNs, loaded by the browser page
itself. Names and text are inserted as text, never HTML.

## Chat overlay source
A source type `chat_overlay` built like the Alerts source: a browser page for
`http://127.0.0.1:<port>/chat?t=<token>&source=<id>` with a transparent background. Settings live
with the source: `style`, `font`, `size`, `text_color`, `background`, `radius`, `width`,
`max_lines`, `fade_after` (seconds, 0 = never), `platforms`, `hide_commands`, `hide_names` (bot
list), `show_events`. Changes go to the page as a config message, so it restyles without a reload.

When the overlay page connects it gets the last few messages (up to `max_lines`, only those still
within `fade_after`), so a reloaded page isn't empty mid-conversation.

## Moderation
Where the readers see removals (Twitch `CLEARMSG`/`CLEARCHAT`, Kick and YouTube deletion events)
they already update the panel; the same removals are forwarded as `remove` / `clear_user`
messages. Platforms that don't report a removal leave the line until it fades.

## Alternatives considered
- **Native compositor rendering:** emotes (including animated ones), wrapping and styling would
  need custom work; the browser source handles them already.
- **Each platform's own widget:** one platform per widget and no combined order.
