# Change: Media sources, playlists and stinger transitions

## Why
Streams need intros, B-roll, background music, sound effects and image slideshows, all played
in sync with the program. OBS has a media source, a VLC source and a slideshow, each separate.
eStream offers one media source that plays any mix of video, audio and image files as a
playlist. The same playback path enables stinger transitions (video with alpha over the scene
change), which were deferred from the transitions change.

## What Changes
- **media-sources** capability (new): a `media` source type.
  - **Playlist:** a main file plus more files or folders, one per line. Videos, audio files and
    still images can be mixed; images show for a set time, which makes a slideshow.
  - Loop, shuffle, and "restart when the file changes".
  - **Decoding** uses FFmpeg on a helper thread:
    - video becomes NV12, or RGBA when the file has alpha;
    - audio is resampled to 48 kHz stereo and goes to the mixer like any audio source;
    - audio and video are scheduled on one playback clock.
  - Media controls in the inspector: play/pause, restart, previous/next, position and duration.
- **transitions** capability (modified): a **Stinger** transition.
  - A video file (ideally with alpha) plays over the switch.
  - The scene changes at a configurable point, and the stinger's audio is mixed.
- Engine: sources receive their own id (`_source_id` in the settings handed to factories), so a
  source's video and audio sides can share one player. Transitions may carry a helper source
  (the stinger) that exists only while they run.

## Out of scope
- Network streams (URLs, SRT input).
- Hardware-accelerated decoding (software decode is fine for 1080p).
- Speed control.
- "Play only when visible" and auto-restart when the scene is shown. These need scene
  visibility events and are a follow-up.

## Impact
- Affected specs: `media-sources` (new), `transitions` (modified)
- Affected code:
  - `estream-media` (player, media source type);
  - `estream-engine` (source ids in settings, stinger helper source);
  - `estream-project` (Stinger transition kind);
  - `apps/estream` (registration, media controls, stinger settings).
