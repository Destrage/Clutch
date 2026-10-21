## 1. Drawing
- [ ] 1.1 Spike: inline images in Slint styled text if available, else Rust row layout with font
      advance widths; check colour emoji in the renderer on Windows and Linux
- [ ] 1.2 Chat line rows of text and image pieces in `chat.slint`; re-layout on width change for
      visible lines

## 2. Message parts
- [ ] 2.1 `Part` and `EmoteSource` in `livechat::Message`; plain `text` kept; tests
- [ ] 2.2 Twitch: parts from the `emotes` tag (code-point positions, animated format); tests with
      multi-byte text
- [ ] 2.3 Kick: parts from `[emote:id:name]`; tests
- [ ] 2.4 YouTube: emoji characters (and Twemoji parts if 1.1 says colour emoji need images)

## 3. Third-party emotes
- [ ] 3.1 7TV, BetterTTV and FrankerFaceZ channel and global sets per reader; word matching on text
      parts; channel set wins; failures logged, never shown; tests on recorded replies

## 4. Images
- [ ] 4.1 Disk cache with a 200 MB cap; download pool of 4 workers; one download per address
- [ ] 4.2 Decode PNG, WebP and GIF frames; in-memory LRU; name shown while loading or after a
      failure
- [ ] 4.3 Animation timer for visible emotes; paused while hidden or minimised

## 5. Settings and docs
- [ ] 5.1 Settings → Chat: Show emotes as images, third-party emotes, Animate emotes
- [ ] 5.2 Hover shows the emote's name and source
- [ ] 5.3 Licenses page (Twemoji if used); website chat page; testing guide
- [ ] 5.4 In-app check:
  - a busy Twitch chat with 7TV emotes;
  - Kick emotes;
  - YouTube emoji;
  - CPU with the chat hidden and visible.
