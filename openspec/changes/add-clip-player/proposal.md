# Change: Play clips and recordings inside Clutch

## Why
Pressing Play (or double-clicking) on a clip or recording in the Clips view opens the file in the
system's default video player (VLC, Movies & TV). The user leaves Clutch to watch a highlight
and comes back to trim, rename or favorite it. The selected-clip pane only shows a still frame,
so trimming is done by numbers rather than by watching.

## What Changes
- **In-app player:** the selected clip or recording plays in the Clips view's preview area with
  play/pause, a seek bar with elapsed and total time, volume and mute. Space toggles play/pause,
  Left/Right seek 5 s.
- **Play opens the in-app player:** Play and double-click start playback inside Clutch. An "Open
  in default player" action stays available next to Reveal.
- **Event markers on the seek bar:** the clip's events (kills, knocks, wins, hype) and a
  recording's markers show as ticks on the seek bar; clicking one seeks a few seconds before it.
- **Trim by watching:** "Set in" and "Set out" buttons take the current playback position, and
  the player loops the trim range while trimming.
- **Audio:** clip audio plays on the monitoring device (default output when none is set), never
  into the mix, streams or recordings.

## Impact
- Affected specs: `clips`.
- Affected code: `estream-media` (`player` reused for single-file playback with no looping),
  `estream-gpu` (`YuvConverter` turns decoded NV12 frames into a texture the UI shows),
  `estream-audio` (an output stream for preview audio on the monitoring device),
  `apps/estream` (clips view player controls, keyboard shortcuts, Play action).
- No new dependencies.
