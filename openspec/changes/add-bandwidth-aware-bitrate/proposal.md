# Change: Stream bitrate from the measured upload speed

## Why
The stream bitrate comes from a fixed per-platform table (`Service::recommended_kbps`). The
table looks only at the canvas size and frame rate, never at the connection. On a slow upload
that bitrate drops frames and buffers for viewers. On a fast one it wastes quality: YouTube's
current guidance for 1080p60 H.264 is 6–17 Mb/s, and Clutch always picks 9. The game being
played also needs part of the upload (game traffic, voice chat), and several destinations share
it. Testers asked for the optimizer to check the connection and set the bitrate from it.

## What Changes
- **Upload speed test.** Clutch measures the upload by sending random, incompressible data to
  Cloudflare's speed test endpoint over four parallel connections for about eight seconds. It
  ignores the first second, while TCP ramps up, and reports the steady rate. The result and its
  time are saved in settings. The test only runs when the user starts it (below), never in the
  background or on first run.
- **Bitrate budget.**
  - usable upload = the lower of 75% of the measured speed and the measured speed minus
    1.5 Mb/s. That leaves room for the game, voice chat and normal swings in the connection.
  - Each destination sends its own copy, so the usable upload is split evenly between the
    enabled destinations, and each destination's audio bitrate comes off its share.
  - Each destination's video bitrate is the lower of its share and the platform's ceiling for
    its canvas. It is never set below a floor of 1.5 Mb/s.
- **Platform ceilings** replace the single recommended value when a test result exists, so a
  fast connection gets the platform's upper range (current published values; checked again
  during implementation):

  | Platform | Ceiling |
  |---|---|
  | Twitch | 6 Mb/s (its limit) |
  | Kick | 8 Mb/s (its limit) |
  | YouTube, H.264 | 2160p60 50 · 2160p30 42 · 1440p60 34 · 1440p30 21 · 1080p60 17 · 1080p30 14 · 720p 8 Mb/s |
  | Facebook | 9 Mb/s at 1080p, 6 Mb/s below |
  | Custom server | unchanged: lowered only when the upload is the limit |

  Without a test result the current table stays as it is.
- **Not enough upload.** When a destination's share is below what its canvas needs for a clear
  picture (the platform's minimum, e.g. YouTube 6 Mb/s for 1080p60, or 4.5 Mb/s for 1080p60
  elsewhere), the optimizer says so and suggests 720p or 30 fps. It does not change the canvas.
- **Where it runs.**
  - **Settings → Performance → Optimize for this PC** runs the test first. The "found" line
    adds "Upload 23.4 Mb/s". The bitrate lines in the change list come from the budget, with the
    reason ("limited by your upload", "platform maximum").
  - **Go Live box**, when a destination is being added: the test runs while the user pastes
    the key ("Checking your upload speed…"), and the saved destination uses its result.
  - **Health alert** "The network can't keep up" (only raised while live, when a test would take
    the stream's upload): its action offers a bitrate that fits what the stream actually got
    through (85% of the rate sent, less audio), for the next time the user goes live.
  - No test runs while live. Going live stops a test that is still running.
  - Settings → Performance shows the last result and its time, with a **Test again** button.
- The test can't reach the server (offline, firewall): the optimizer says so and keeps the table
  values; nothing else fails.

## Out of scope
- Changing the bitrate during a live stream when the connection gets worse (adaptive bitrate).
  It needs encoder reconfiguration mid-stream and should be its own change. The test and budget
  here are its starting point.
- HEVC/AV1 ceilings for YouTube over Enhanced RTMP (lower bitrates for the same quality). They
  can be added when those codecs become a profile choice for streams.

## Impact
- Affected specs: `performance` (ADDED: Upload Speed Test, Bandwidth-Aware Stream Bitrate)
- Affected code: new `apps/estream/src/speedtest.rs`; `crates/estream-media/src/settings.rs`
  (ceilings and minimums per platform); `apps/estream/src/performance.rs` (budget,
  `with_profile` bitrates, change reasons); `apps/estream/src/app.rs` (Optimize, Go Live box,
  health action); `apps/estream/src/settings.rs` (saved result); `apps/estream/ui/overlays.slint`
  and `golive.slint`
- Network: one user-started request set to `speed.cloudflare.com`. It carries no account,
  stream key or identifying data beyond the connection itself.
- Shared work: branch `add-bandwidth-aware-bitrate`
