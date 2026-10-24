# Design: Stream alerts and event sounds

## Typed events
`livechat::Message.event: bool` becomes `event: Option<StreamEvent>`:

```rust
pub struct StreamEvent {
    pub kind: EventKind,          // Follow, Sub, Resub, Gift, Bits, Raid, Paid
    pub platform: Platform,
    pub name: String,             // viewer ("Anonymous" for anonymous gifts)
    pub amount: Option<f64>,      // bits, gift count, raid viewers, Kicks, paid amount
    pub amount_text: Option<String>, // "$5.00" for Super Chats (currency as YouTube shows it)
    pub months: Option<u32>,
    pub tier: Option<String>,     // "Tier 2", "Prime"
    pub message: Option<String>,
}
```

Mapping from what each reader already parses:

| Kind | Twitch (EventSub) | Kick (relay) | YouTube (live chat) |
|---|---|---|---|
| Follow | `channel.follow` | `channel.followed` | – (not available live) |
| Sub | `channel.subscribe` | `channel.subscription.new` | `newSponsorEvent` |
| Resub | `channel.subscription.message` | `channel.subscription.renewal` | `memberMilestoneChatEvent` |
| Gift | `channel.subscription.gift` | `channel.subscription.gifts` | `membershipGiftingEvent` |
| Bits | `channel.cheer` | – | – |
| Raid | `channel.raid` | – | – |
| Paid | – | `kicks.gifted` | `superChatEvent`, `superStickerEvent` |

The display text stays as today (built from the typed event), so the chat looks the same.

## Event sounds
`Settings.event_sounds: HashMap<EventKind, PathBuf>` plus a volume. Sounds play through the
same short-sound player as the clip-saved sound, decoded once and cached; files that fail to
load are reported once and fall back to the chime.

## Overlay server
A small HTTP/WebSocket server on the UI side (`apps/estream/src/overlay/`), started when an
overlay source exists in the setup and stopped when none does:
- Binds `127.0.0.1` on a free port (remembered, so pages keep working across restarts when it is
  still free).
- Every route requires the install's random token (`?t=…`, stored in settings, regenerable).
- `GET /alerts` serves the bundled alerts page; `GET /asset/<id>` serves the user's animation,
  sound and font files by id (never arbitrary paths); `GET /ws` streams JSON messages:
  `{"type":"config", …}` on connect and when settings change, `{"type":"alert", …}` per event.
- The server runs on its own thread with the existing `tokio` runtime used for I/O; it never
  touches the video or audio threads.

## Alerts source
A source type `alerts` whose video and audio come from the browser source machinery: it creates
a browser page for `http://127.0.0.1:<port>/alerts?t=<token>&source=<id>` at the canvas size,
transparent background, audio on (the page plays the sound, so it goes through the source's own
fader and tracks like any browser source).

Settings live with the source (scene collection), per kind:
`enabled`, `animation` (path), `sound` (path), `volume`, `template`, `font`, `size`, `color`,
`duration` (seconds; default: the animation's length, at least 4 s), `min_amount`, and a layout
choice (text below, over, or above the animation).

The page keeps a queue: one alert at a time, a short gap between alerts, and the queue is capped
(oldest follows dropped first) so a raid of follows doesn't play for minutes. The name and
message are inserted as text (never HTML).

Test: the inspector's "Test" button per kind sends a sample event with a placeholder name.

## Formats
- Animation: WebM VP9 with alpha (best: small, smooth, transparent), animated WebP, APNG, GIF, or
  a still PNG. MOV/ProRes 4444 isn't playable in the browser and is listed as unsupported.
- Sound: WAV, OGG, MP3.
- Fonts: system fonts by name, or a TTF/OTF file served as an asset.

## Alternatives considered
- **Native compositor source** (decode the alpha video with the media player, draw text with the
  text source): no browser needed, but text layout, animation and theming would all be custom,
  and there's no easy way for users to share styles. The browser source already exists and is
  isolated.
- **Third-party alert services:** need accounts and a web service per platform; Clutch already
  has the events.
