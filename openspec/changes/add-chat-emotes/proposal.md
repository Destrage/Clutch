# Change: Emotes as images in the chat

## Why
In the combined chat, emotes show up as text:
- Twitch sends the emote's name ("Kappa"), and Clutch ignores the emote positions that come with it.
- Kick sends `[emote:37234:Prayge]`, which Clutch turns into "Prayge".
- The animated emotes most channels use come from 7TV, BetterTTV and FrankerFaceZ. They reach every
  chat as plain words.

Testers found the chat hard to follow next to the platforms' own chat, where these are pictures,
many of them animated.

## What Changes
- **Messages in parts.** A chat message becomes a list of pieces: text, and emotes with a name, an
  image address and whether they're animated. The text form stays for the reply box, copying, and
  anywhere an image can't be shown.
- **Platform emotes:**
  - Twitch: from the `emotes` tag of each message (positions and ids), including animated ones;
    images from Twitch's emote server.
  - Kick: from the `[emote:id:name]` codes; images from Kick's emote files.
  - YouTube: standard emoji arrive as emoji characters and are shown in colour. YouTube's API
    doesn't give the images for channel-member emoji (`:_name:`), so those stay text.
- **7TV, BetterTTV and FrankerFaceZ emotes.** When a channel's chat starts being read, Clutch loads
  that channel's emote sets and the global ones from these services' public APIs. Words in messages
  that match are shown as the emote. This is on by default, with a switch in Settings → Chat.
- **Shown inline.** Emotes sit in the text at about line height and wrap with it. Animated ones
  play, and pause when the chat isn't visible. Hovering an emote shows its name and where it comes
  from.
- **Cached.** Each image is downloaded once into a disk cache, with a size cap. While an image
  loads, or if it fails, the emote's name is shown instead.
- **Settings → Chat:**
  - Show emotes as images (on).
  - 7TV, BetterTTV and FrankerFaceZ emotes (on).
  - Animate emotes (on).

## Out of scope
- An emote picker in the reply box.
- A chat overlay for the stream (no chat overlay exists yet; it can reuse this when it's added).
- Twitch cheermotes, badges, and YouTube member emoji images.

## Impact
- Affected specs: chat (emotes).
- Affected code: `apps/estream`:
  - `livechat` (message pieces; Twitch, Kick and YouTube parsing; third-party emote sets);
  - an emote cache and animator;
  - `chat.slint` (line layout with images);
  - Settings → Chat.
- Network: one request per channel and service when its chat starts, plus each emote image once.
  The third-party services are public APIs with no keys.
- Licences: if colour emoji need images (see design), Twemoji graphics are CC-BY 4.0 and are
  credited on the Licenses page.
