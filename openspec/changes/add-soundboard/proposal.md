# Change: Soundboard with optional on-stream effects

## Why
Streamers want to fire meme sounds and quick effects (an air horn, a "bruh", a confetti burst,
a reaction GIF) with one click while live. Today that needs a separate soundboard app routed
into Clutch as an audio source, plus a browser source for visuals. Clutch already plays
sounds and animations for stream alerts, so it can offer this directly on the Live deck.

## What Changes
- **Soundboard** panel on the Live deck (and in the studio as a dock): a grid of pads. Each pad
  has a **name**, an **icon** (emoji, image or the first frame of its visual), a **colour**, and
  plays when clicked.
- **Each pad has:**
  - **Sound** (WAV, OGG, MP3, FLAC), its own volume, and optional trim (start/end).
  - **Who hears it:** stream and me (default), stream only, or me only.
  - **Visual (optional):** an image, GIF, animated WebP or video (WebM with alpha recommended),
    shown on the canvas at a chosen place (preset corners/centre, or drag on the preview),
    size, how long it stays, and an enter/exit animation (fade, pop, slide). It can be shown
    with or without the sound.
  - **Canvases:** which canvases show the visual (all by default).
  - **When pressed again while playing:** restart, stop, or play over (layer).
  - **Hotkey (optional)**, through the existing global hotkeys.
- **Editing:** "Edit soundboard" on the panel opens the pad editor. Add a pad by clicking +,
  or by dropping sound or image files on the grid. Reorder by dragging. Pads can be grouped
  into pages (tabs) such as "Memes" and "Hype".
- **Audio path:** the soundboard is one audio source in the mixer, called "Soundboard", with its
  own knob, mute and track routing, so it can be kept off a recording track or the VOD track.
  "Me only" pads go to the monitoring output only.
- **Visual path:** a "Soundboard effects" item is added to each canvas's scenes on first use,
  at the top, so effects show over everything in every scene. It reuses the alerts renderer
  (decoding, alpha video, animations).
- **Stop all** button and a global hotkey stop every sound and visual at once.
- **Automation and remote control:** "Play soundboard pad" becomes a macro action and an
  obs-websocket vendor request, and an MCP tool once `add-mcp-server` lands, so Stream Deck and
  chat bots can trigger pads.
- **Sharing:** a soundboard can be exported and imported as one `.clutchsound` file (a zip with
  the pads' settings and media). Pads are saved with the setup, so each game setup can have its
  own board.
- No sounds or effects ship as presets in this change. A small built-in pack can follow once
  it has licensed (CC0) media.

## Out of scope
- Text-to-speech, voice-changer pads, and viewer-triggered pads (channel points/bits). Those
  can build on this later.
- Downloading sounds from online soundboard sites.

## Impact
- Affected specs: `soundboard` (new capability, including control from macros and remote API).
- Affected code: `crates/estream-capture` (soundboard source: audio playback and the effects
  renderer shared with alerts), `crates/estream-project` (pads in the setup, export/import),
  `crates/estream-automation` (macro action), `crates/estream-api` (vendor request),
  `apps/estream/ui/deck.slint` and a new `soundboard.slint` (grid and editor), `apps/estream/src`.
- Shared work: branch `add-soundboard`.
