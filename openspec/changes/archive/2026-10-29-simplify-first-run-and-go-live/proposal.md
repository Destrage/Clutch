# Change: Simple first run and one-step Go Live

## Why
eStream should be easier than OBS, not just as capable. Today a new user has to:
1. pick a canvas resolution they may not understand;
2. start from an empty scene and figure out "display capture" against "game capture";
3. find Settings → Stream destinations and fill in a service, server, key, encoder and bitrate
   before they can go live.

A real user streaming to Kick also hit a server URL without `/app` and couldn't connect. Every
one of these steps is a place where people give up.

## What Changes
- **First run asks about the goal, not the format.** "What will you use eStream for?"
  - Streaming games.
  - Talking to camera.
  - Recording videos and tutorials.
  - Vertical video (TikTok, Shorts, Reels).

  eStream then builds a ready-to-use scene from what it detects:
  - the primary screen;
  - game capture (when gaming);
  - the webcam, placed by purpose;
  - desktop audio;
  - the default mic with noise suppression on.

  The wizard lists what it found and which encoder it will use. Canvas presets remain behind
  "Choose the resolution myself".
- **Going live takes two choices: where, and the key.**
  - Clicking Go Live with no destination opens a small Go Live box instead of Settings.
  - **Platforms:** Twitch, YouTube, Kick and Facebook Live as tiles. The platform is also
    recognised from the key format.
  - **Finding the key:** a button opens the platform's stream-key page.
  - **Automatic choices:** eStream picks the server, the encoder (best available hardware) and the
    platform's recommended bitrate for the canvas, and shows them in one line.
  - **Go Live** saves the destination and starts the stream. "More options" opens the full
    destination settings.
  - **Adding another platform:** the live panel gains **Add platform**, which opens the same box.
- **Guards against common mistakes.**
  - Pasting a server URL into the key field gets a clear message.
  - Keys are trimmed.
  - Amazon IVS servers get their `/app` path automatically. That fix already shipped in 0.1.1.

## Out of scope
- A measured upload-speed test. Bitrates follow each platform's published recommendations, and
  the health alerts flag a connection that can't keep up.
- Signing in to platforms (OAuth) to fetch keys automatically.
- Scene templates, the plain-language Add menu, and one-click fixes (separate changes).

## Impact
- Affected specs: `onboarding` (new), `streaming` (quick setup)
- Affected code:
  - `estream-media` settings: per-service bitrate, key page, key detection;
  - `apps/estream`: wizard, Go Live box, live panel.
