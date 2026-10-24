# Change: Stream alerts and event sounds

## Why
Clutch already receives follows, subs, gifts, bits, raids and paid messages from Twitch, Kick and
YouTube (`add-stream-events`), but only shows them as highlighted chat lines with one shared
chime. Streamers want viewers to see and hear these moments on stream: an animation with the
viewer's name and a sound, different for each kind of event. Today that means a third-party
alert service and a browser source per service. Clutch can do it itself, from the events it
already has, with no account or web service.

## What Changes
- **Typed stream events:** chat event lines carry a typed event (kind, platform, viewer name,
  amount, months, tier, message) instead of only display text. The seven kinds shared across
  platforms are follow, sub, resub, gift, bits, raid and paid message (Super Chat, Super Sticker,
  Kicks).
- **Event sounds in the chat:** each kind can have its own sound file (WAV, OGG, MP3), replacing
  the single chime; kinds without a sound use the built-in pack's sound. Volume setting; the bell still
  mutes them all.
- **Alerts source:** a new source type, "Alerts", added from Add source like any other. For each
  kind: on/off, an animation (WebM with alpha recommended; animated WebP, PNG, GIF also work), a
  sound, a text template (`{name} subscribed for {months} months`) with font, size and colour,
  a duration, and for amounts a minimum (e.g. only bits of 100+). Alerts play one at a time from
  a queue; a "Test" button per kind plays a sample.
- **Live text over the animation:** animations contain no text. Clutch fills the template per
  event (`{name}`, `{platform}` or a platform icon, `{amount}`, `{months}`, `{tier}`,
  `{message}`) and draws it in a text box on the animation, appearing and leaving at set times,
  shrinking to fit long names.
- **Alert packs:** a pack is a folder with, per kind, the animation, one or more sounds and an
  `alert.json` saying where the text box is, when the text appears and its style. Clutch ships a
  built-in pack (sounds made from CC0 packs now; animations from the project owner later); users can pick another pack,
  share their own, or replace single files. Several sounds for a kind play in turn or at random,
  and a kind can have a bigger variant above an amount (e.g. 10+ gifted subs).
- **Local overlay server:** Clutch serves its overlay pages and pushes events to them over a
  WebSocket on `127.0.0.1` only, with a random per-install token in the page address. The Alerts
  source is a browser page on this server, so animations, fonts and transparency come from the
  browser engine, and a broken page can't take the stream down (the browser runs out of
  process). The same address works in other apps that show web pages.

## Impact
- Affected specs: `chat` (typed events, event sounds), new `overlays` (overlay server, Alerts).
- Affected code: `apps/estream` (`livechat` typed events, chat sounds, alerts settings UI,
  overlay server start/stop), `estream-capture` (Alerts source type on top of the browser
  source), new overlay page assets (HTML/CSS/JS bundled into the app).
- Needs from the project owner: the built-in pack: per kind an alpha WebM without text (with
  where its text box sits and when the text appears) and a WAV sound. Users can change or add
  their own.
- Builds on `fix-settings-and-clip-sound` (the built-in short-sound player) and `add-stream-events`.
