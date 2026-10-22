## 1. Drawing
- [x] 1.1 Spike: inline images in Slint styled text if available, else Rust row layout with font
      advance widths; check colour emoji in the renderer on Windows and Linux
- [x] 1.2 Chat line rows of text and image pieces in `chat.slint`; re-layout on width change for
      visible lines

## 2. Message parts
- [x] 2.1 `Part` and `EmoteSource` in `livechat::Message`; plain `text` kept; tests
- [x] 2.2 Twitch: parts from the `emotes` tag (code-point positions, animated format); tests with
      multi-byte text
- [x] 2.3 Kick: parts from `[emote:id:name]`; tests
- [x] 2.4 YouTube: emoji characters (and Twemoji parts if 1.1 says colour emoji need images)

## 3. Third-party emotes
- [x] 3.1 7TV, BetterTTV and FrankerFaceZ channel and global sets per reader; word matching on text
      parts; channel set wins; failures logged, never shown; tests on recorded replies

## 4. Images
- [x] 4.1 Disk cache with a 200 MB cap; download pool of 4 workers; one download per address
- [x] 4.2 Decode PNG, WebP and GIF frames; kept while a line shows them; name shown while loading or after a
      failure
- [x] 4.3 Animation timer for visible emotes; paused while hidden or minimised

## 5. Settings and docs
- [x] 5.1 Settings → Appearance → Chat: Show emotes as images, third-party emotes, Animate emotes
- [x] 5.2 Hover shows the emote's name and source
- [ ] 5.3 Licenses page (nothing to add: no Twemoji); website chat page; testing guide
- [ ] 5.4 In-app check:
  - a busy Twitch chat with 7TV emotes (done: third-party emotes inline);
  - Kick emotes (done, animated);
  - YouTube emoji (done: colour emoji render natively);
  - CPU with the chat hidden and visible.
