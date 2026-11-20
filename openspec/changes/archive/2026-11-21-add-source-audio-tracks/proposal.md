# Change: Add a separate audio track for each source

## Why
Recordings and clips carry the mixed tracks only (by default just track 1). Once the game, the mic,
Discord and alerts are mixed together they can't be taken apart: a keyboard on the mic or a
friend on Discord stays in the clip for good. The six tracks can be routed by hand, but that
needs planning before the moment happens, and clip mode's own engine puts everything on track 1.

Users want every sound kept apart automatically, so they can decide afterwards what a clip or
recording should contain.

## What Changes
- **Source tracks** in recordings and the replay buffer (so in every clip): besides the mixed
  tracks, each audio source heard on the canvas gets its own audio stream, named after the source
  ("Game capture", "Microphone", "Discord"). A source track holds what that source adds to the mix
  (after its filters, volume, mute and balance), so muting the mic keeps it out of its track too.
- The mixed tracks stay first and the default, so other players and editors play the full mix.
- Source tracks that stayed silent for the whole clip are left out of the clip; likewise for a
  recording remuxed to MP4.
- A **"Separate track for each source"** switch in Settings → Recording, on by default. Streams are
  unchanged (platforms take one track).
- **Clip mode's own engine** (used when no program scene shows the game) mixes the studio's own
  microphone sources, with their filters and volume, instead of an unfiltered default mic. Game,
  desktop audio and each mic get their own source track.
- **Clips player**: a "Hear" row lists the full mix and each source track; choosing source tracks
  plays only those (for example the game without the mic). Trimming keeps every track and its name.

Not in this change: per-track volume in the montage editor (it keeps using the mix), and
per-source video ISO recording (`add-iso-recording`).

## Impact
- Affected specs: `recording` (source tracks), `clips` (clip engine audio, player tracks, trim)
- Affected code: `estream-audio` (mixer taps each source for subscribed sinks), `estream-media`
  (output, packet ring, muxer stream names, remux, trim, player track selection), `apps/estream`
  (setting, clip engine, Tracks menu)
- Files get one extra AAC stream per source (192 kbps each by default: about 1.4 MB per source per
  minute). Clips usually hold only a few sources once silent ones are left out.
