# Design: Play clips and recordings inside Clutch

## Decoding
`estream-media::player::Player` already demuxes and decodes on a helper thread, paces video
against a playback clock, supports pause and seek, publishes the latest picture (`Picture`, NV12
or RGBA) and pushes 48 kHz stereo audio into a ring. The clip player creates one `Player` with
`PlayerSettings { items: [clip], looping: false, .. }`, outside the media-source registry (it is
not a source), and drops it when the selection changes or the view closes.

## Showing frames
Each UI tick, if `Player::sequence()` moved on, the app takes the picture (`try_picture`, never
blocking) and uploads it through `estream-gpu::YuvConverter` (NV12 → RGBA on the GPU, BT.709/601
and range from the picture) into a preview texture created like the engine's own preview
textures, which Slint already accepts (`slint::Image::try_from(wgpu::Texture)`). The texture is
reused while the size stays the same, so a new frame costs an upload and one draw. The UI
requests redraws only while playing.

## Audio
The player's audio ring is drained by a cpal output stream on the monitoring device (default
output if none), opened when playback starts and closed on pause or stop. A volume and mute
setting scale the samples there. The stream lives in `estream-audio` next to the existing
monitoring output and never feeds the mixer, so preview audio can't reach streams, recordings
or the replay buffer. `AUDIO_LEAD` delays video by the same amount the audio is pushed ahead, so
they stay in sync.

## Controls
- Play/pause, a seek bar (drag to seek; position from `PlayerStatus`), elapsed / total time,
  volume slider and mute.
- Event ticks: clip metadata already lists events with their times; recordings have
  `<recording>.markers.json`. Clicking a tick seeks to 3 s before it.
- Trimming: "Set in" / "Set out" copy the current position into the existing trim fields;
  while the trim panel is open, reaching the out point seeks back to the in point.
- Keyboard (when the Clips view has focus): Space, Left/Right (±5 s), M (mute).

## Play action
`"play"` starts in-app playback of the selected item. The old behaviour moves to a new
`"open-external"` action ("Open in default player"), for formats the player can't decode or for
users who prefer their own player.

## Performance
Decoding uses FFmpeg's software decoders on the player thread (as media sources do). The player
only runs while the Clips view is open with a clip selected and playing, so it doesn't compete
with a live show. If the engine reports a live output (streaming or recording), playback
resolution is capped at 1080p.

## Alternatives considered
- **Embedding a native player (Media Foundation, libVLC):** another dependency, a native child
  window that Slint can't draw over, and no cross-platform path.
- **Keep opening the default player:** loses the trim-by-watching workflow.
