# Change: Game and window capture bring their own audio

## Why
To get a game's sound on its own fader, users add Game capture and then a separate Application
audio source pointed at the same game. That is one more step to discover, and with Desktop audio
on as well the game plays twice. A game capture should just come with the game's sound.

## What Changes
- **Capture audio** switch on Game capture and Window capture, on by default (also for existing
  sources). The audio follows whatever the source captures: in "Any fullscreen application" mode
  it switches with the game. It appears in the mixer under the source's name, with its own
  volume, mute, filters and tracks like any other audio.
- **No double sound:**
  - Desktop audio (on the default device) automatically leaves out the game that a game or window
    capture is capturing, so the game is heard once, on its own fader. Discord, music and
    alerts stay on Desktop audio.
  - When an Application audio source already captures the same app, the capture's audio stays
    silent and says so, so existing setups keep working unchanged.
- **Clips:** clip mode's own capture uses the game's audio, and audio hype listens to it.

## Impact
- Affected specs: video-sources (capture audio and the desktop audio exclusion).
- Affected code: `estream-capture` (capture audio part, process loopback include/exclude, the
  desktop audio wrapper, window process IDs), `apps/estream` (clips).
- Windows 10 2004 or later (process loopback), as for Application audio today.
