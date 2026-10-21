# Design: Emotes as images in the chat

## Message model
`livechat::Message` gains `parts: Vec<Part>`, and `text` stays as the plain form:

```rust
pub enum Part {
    Text(String),
    Emote { name: String, url: String, animated: bool, source: EmoteSource },
}
pub enum EmoteSource { Twitch, Kick, SevenTv, Bttv, Ffz, Emoji }
```

## Where emotes come from
- **Twitch:** the IRC `emotes` tag is `id:start-end,start-end/id:…`. The positions count Unicode
  code points, not bytes, so the text is split by `chars()`. The image is
  `https://static-cdn.jtvnw.net/emoticons/v2/{id}/default/dark/2.0`. "default" gives the animated
  version when there is one; Twitch's emote format tells whether it's animated.
- **Kick:** `[emote:{id}:{name}]` becomes `https://files.kick.com/emotes/{id}/fullsize`.
  `emotes_as_names` stays for the plain text.
- **YouTube:** `displayMessage` already has the emoji characters. Member emoji (`:_name:`) stay text.
- **Third-party sets.** These load once when a channel's reader starts, are kept in memory for the
  session, and are matched word by word, only on text parts. The channel's set wins over the global
  set.
  - 7TV: `https://7tv.io/v3/users/{twitch|kick|youtube}/{id}` for the channel, where the channel has
    linked 7TV, and `https://7tv.io/v3/emote-sets/global`.
  - BetterTTV: `https://api.betterttv.net/3/cached/users/{twitch|youtube}/{id}` and
    `/3/cached/emotes/global`.
  - FrankerFaceZ (Twitch only): `https://api.frankerfacez.com/v1/room/id/{id}` and
    `/v1/set/global`.

  A failed request means no emotes from that service, with a debug log line. Chat keeps working.

## Drawing a line
Slint has no inline images in `Text` and no flow layout, so Rust lays out each message:
1. Split the text parts into words, and treat each emote as a box about 1.4 times the line height
   tall, as wide as its aspect ratio.
2. Measure words with the UI font's advance widths (`ttf-parser` on the font Slint uses), and break
   into rows at the panel's width.
3. Pass rows of pieces (`text` or `image`, with x offsets) to `chat.slint`, which draws them with
   `Text` and `Image` elements.
4. Lay the lines out again when the panel width changes, debounced to 100 ms, and only for the
   visible messages plus a margin.

Task 1.1 is a spike to confirm this. If the current Slint version supports inline images in
styled text, that's used instead and steps 2 to 4 go away.

Colour emoji: if Slint's renderer draws emoji characters without colour, emoji become `Emoji`
emote parts using Twemoji PNGs bundled with the app (about 3,600 files; packed in one archive).

## Images and animation
- **Disk cache:** `%LOCALAPPDATA%\Clutch\emotes` (Linux: `~/.cache/clutch/emotes`), keyed by a hash
  of the address. When it grows past 200 MB, the least recently used files are removed. Downloads
  run on a small pool of 4 worker threads, never on the UI thread, and each address is downloaded
  only once even if many messages ask for it.
- **Decoding:** the `image` crate, already a dependency, with png, webp and gif. Animated frames
  are decoded once into `slint::Image`s and kept in an in-memory LRU of about 64 MB.
- **Animation:** one timer at 20 fps moves each visible animated emote to the frame for its own
  timing. It stops while the chat panel is hidden or Clutch is minimised, and when "Animate emotes"
  is off, emotes show their first frame.

None of this runs on the real-time threads.

## Alternatives considered
- **Draw each message into one image in Rust.** That loses text selection and the theme's
  colours, and redraws a whole line for every animation frame. Rejected.
- **Show the chat in an embedded browser (CEF).** It's heavy for a panel that's always visible, and
  it doesn't match the rest of the UI. Rejected.
