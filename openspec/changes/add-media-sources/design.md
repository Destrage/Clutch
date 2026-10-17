# Design: Media sources

## Player (`estream-media::player`)
A `Player` is shared by a source's video and audio sides. It is found in a process-wide registry
keyed by source id and holds weak references. The decode thread stops when both sides are gone.

Decode thread, per playlist item:
1. Open the item with libavformat and pick the best video and audio streams.
2. Decode packets in demux order.
   - **Video:** the frame is converted with swscale to NV12 (tightly packed), or to RGBA if the
     source pixel format has alpha. The thread waits until the playback clock reaches the
     frame's pts, then publishes it as the latest frame with a sequence number.
   - **Audio:** samples are resampled with swresample to f32 stereo at 48 kHz and pushed into a
     ring buffer once the clock reaches `pts − LEAD` (LEAD = 100 ms).
3. **Still images** (a single video frame and no audio) are shown for `image_seconds`.
4. At the end of the item, go to the next item (in shuffled order if shuffle is on). After the
   last item, loop or stop; the last frame stays visible.

**Playback clock:** `position = base + (now − started)` while playing, frozen while paused.
Commands (pause/resume, restart, next/previous, seek) arrive over a channel and are checked while
waiting. A seek flushes the decoders and resets the clock base.

**Audio side (`AudioSource::read`):** it waits until LEAD worth of samples is buffered, then
consumes one block per 10 ms. Its output therefore lags the push time by LEAD, which puts it at
the samples' pts on the same clock the video uses. On underrun it outputs silence and
pre-buffers again. It uses `try_lock` only and never waits.

**Video side (`VideoSource`):** `tick` picks up a new frame by sequence number. NV12 frames go
through the existing `YuvConverter`; RGBA frames upload as an sRGB texture. Status shows the
current file name, or the error for a missing or invalid file.

## Settings
| key | type | default |
|---|---|---|
| `file` | path (video/audio/image) | "" |
| `playlist` | multiline text: more files or folders, one per line | "" |
| `loop` | bool | true |
| `shuffle` | bool | false |
| `image_seconds` | float | 5 |

`_source_id` is added by the engine to the settings it hands to source factories and `update`.
It is never stored in the project and is ignored by validation.

## Stinger transitions
`TransitionKind::Stinger { file, point_ms }`. The transition duration is the stinger length the
user sets.
- While the transition runs, the render graph adds a helper source `{kind: "media", settings:
  {file, loop: false}}` with a fixed id per canvas, and puts it in the audio graph so its sound
  plays.
- The video thread draws the outgoing scene until `point_ms` and the program scene after it,
  then composites the stinger frame stretched to the canvas on top, in the same submission.
- The helper source is created when the transition starts and dropped when it ends, so each
  stinger plays from the beginning.

## UI
- The inspector shows a MEDIA section for `media` sources:
  - play/pause, restart, previous and next buttons;
  - the current file name and position / duration.
  It polls `player::get(id)` from the UI timer.
- The transition picker gains "Stinger…". It asks for a file and adds fields for the transition
  point and duration.

## Testing
- Test media is generated with the bundled FFmpeg (`testsrc` video with tone audio, a
  `color` + `alpha` video, a PNG).
  - Playback: frames advance and the audio side delivers non-silent samples at about the real
    rate.
  - A playlist with an image then a video advances.
  - Loop restarts; non-loop stops on the last frame.
  - Pause freezes the position.
  - A missing file reports an error status.
  - An alpha video produces RGBA with transparent pixels.
- Engine: the media source renders on a canvas, both its sides share one player, and a stinger
  transition draws the stinger over the scenes and switches at the point.
- In-app: a media source with a test video and the inspector controls.
