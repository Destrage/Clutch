# Design: add-montage-editor

## Crate `estream-montage`
It depends on `estream-media` (decode/encode), `estream-gpu` (compositing) and `estream-clips`
(library, hype detector). It does not depend on the live engine: montage work never touches the
studio's video or audio threads, so editing while live can't drop stream frames. Rendering does
share the GPU, though. Exports use a lower thread priority and show a warning while live.

### Project model

```rust
pub struct Montage {
    pub version: u32,
    pub format: Format,               // width, height, fps (e.g. 1080x1920@60)
    pub segments: Vec<Segment>,       // the video track, in order
    pub transitions: Vec<Transition>, // transitions[i] sits between segments[i] and [i+1]
    pub titles: Vec<Title>,           // text overlays on the timeline
    pub music: Option<Music>,
    pub mix: Mix,                     // clip gain, music gain, ducking amount
}
pub struct Segment {
    pub source: PathBuf,              // clip or recording
    pub src_in: f64, pub src_out: f64,// seconds in the source
    pub speed: f32,                   // 0.25..=2.0
    pub crop: CropRect,               // normalized; for aspect change / pan
    pub gain_db: f32,
    pub events: Vec<f64>,             // event times in source seconds (for snapping/slow-mo)
}
pub enum Transition { Cut, Crossfade { secs: f32 }, DipToBlack { secs: f32 }, Whip { secs: f32 } }
pub struct Title { pub text: String, pub style: TitleStyle, pub start: f64, pub dur: f64, pub pos: Anchor }
pub struct Music { pub source: PathBuf, pub src_in: f64, pub gain_db: f32, pub fade_in: f32, pub fade_out: f32, pub beats: Vec<f64> }
```

- **Timeline time comes from the model**, never stored: segment *i* lasts
  `(src_out - src_in) / speed`, and a transition overlaps the two segments around it by its
  length.
- **Edits are `MontageCommand`s** (`InsertSegment`, `RemoveSegment`, `MoveSegment`,
  `SetSegment` for trim/speed/crop/gain, `SetTransition`, title, music, mix and `Batch`; a split
  is a batch). Like the engine's commands they are serializable, and applying one returns its
  inverse. The engine's `History` is tied to its own `Command` and `Project` types, so the
  montage has a small `History` of the same shape (inverse stack, redo stack, coalescing of
  repeated edits within 500 ms until sealed). A montage project is its own document, with its
  own undo history. Ids for segments and titles are never reused, so `next_id` may stay ahead
  after an undo.
- **Saved as JSON**, with a version number and migrations like project files. Source paths are
  stored absolute and also relative to the montage file. When loading, the absolute path is tried
  first, then the relative one, and if both fail the segment is flagged as missing so the user
  can relink it.

## Highlight scan
Input: a recording, plus its marker file if there is one.

1. **Markers:** each event becomes a candidate window using the clip defaults for its kind
   (e.g. a kill is 10 s before and 3 s after). Score: ace/multi-kill 1.0, objective/win 0.8,
   kill 0.6.
2. **Audio:** decode the master track to 48 kHz mono f32 on a worker thread, which runs at about
   100× real time. Then run `estream_clips::hype::scan`. Peaks become windows, with a score from
   the dB above the baseline mapped to 0.3–0.9.
3. Merge overlapping windows. The merged window keeps the highest score and adds +0.1 for every
   extra signal.
4. Return the windows sorted by time with `{ start, end, score, reasons }`. A 1-hour recording
   should scan in under 1 minute.

## Beat detection (`estream_montage::beats`)
An in-house onset-based beat tracker (no new dependency):
1. Compute spectral flux on 1024-sample frames, hop 512.
2. Find the tempo by autocorrelation in the 70–180 BPM range.
3. Pick beats by dynamic programming (Ellis 2007).

It returns beat times and downbeat estimates (every 4th beat, phase picked by flux energy). It is
tested on synthetic click tracks (with the tempo known) and a few royalty-free test loops checked
into `third_party/testdata`.

## Auto-assembly
Input: picked items (clips or highlight windows), target length *T*, format, order, and optional
music.

1. **Event anchors:** each item's main moment is its first event (or the hype peak, or the
   middle of the item).
2. **Allocate time to items:** with *n* items, each gets about `T / n` seconds. An item gets
   more if its score is above average, never less than 2 s, and never more than the item
   itself. If the items don't fit, the lowest-scoring ones are dropped.
3. **Trim around the anchor:** each segment keeps 70 % of its time before the anchor and 30 %
   after.
4. **Snap to beats (when there is music):**
   - snap each cut to the nearest beat within ±0.25 s by adjusting the segment's out point;
   - prefer downbeats for segments with a score of 0.8 or more;
   - start the music so that its first downbeat falls on the first cut.
5. **Transitions:**
   - cuts by default;
   - a 0.3 s whip between items from different games or recordings;
   - a fade to black at the very end (`Montage::fade_out`, 0.6 s by default; a transition only
     sits between two segments, so the end fade is its own setting, editable with `SetFadeOut`).
6. **Slow motion:** optionally slow the 1 s around ace/multi-kill anchors to 0.5×. On by default.
7. **Vertical crop:** for 9:16, crop to the center by default. The user can pan it per segment.
8. **Title card:** optional first title ("<Game> highlights") over the first 2 s.

It is deterministic for the same inputs, so it can be tested with snapshots of the resulting
`Montage`.

## Rendering (shared by preview and export)
- **Decoding:**
  - one `estream_media::frames::FrameReader` per source in use (at most 2 during a
    transition), kept open while its segments play;
  - software decode (multi-threaded) to NV12 with the media source's converter, uploaded with
    the GPU's existing `YuvConverter` (the same path media sources use). Hardware decode
    straight into textures can replace it later without changing the renderer;
  - frame-accurate: asking for an earlier time, or more than 3 s ahead, seeks to the keyframe
    before it and decodes forward; asking for the next frames just decodes on (one frame of
    look-ahead decides whether the next frame is due).
- **Compositing** on a wgpu render target of the montage format:
  - each segment is drawn with crop → fit (letterboxed when the crop's shape differs from the
    format) using the compositor's bilinear sampling, the same in preview and export (a sharper
    export scaler is a later improvement);
  - passes: the video (one segment, or both segments into the transition textures and the
    transition shader into the video texture), then the canvas (video + titles, with the end
    fade as opacity). Each pass has its own `CanvasRenderer` so they share one submission;
  - transitions are shaders (crossfade mix, dip, whip with a directional blur and offset);
  - titles reuse the text source renderer (`estream-capture::media::text`) to draw to textures,
    with fade/slide-in presets.
- **Audio:**
  - clip audio is resampled for speed changes (`rubato`, already used in `estream-audio`);
  - pitch is kept for 0.5×–2× by time-stretching (WSOLA, in-house), and plain resampling is
    used below that;
  - music is mixed with fades;
  - ducking: the music drops by *d* dB (default 8) when clip audio is above −35 dBFS, with
    150 ms attack and 600 ms release.
- **Preview:**
  - runs on the montage worker thread, paced to real time;
  - frames are presented through the same Slint–wgpu texture path as the studio preview;
  - while scrubbing, it seeks only and skips the look-ahead;
  - when it falls behind, the preview drops frames instead of slowing down.
- **Export:**
  - runs the same graph as fast as possible into the `estream-media` encoder;
  - progress is reported every 0.5 s;
  - cancelling deletes the partial file;
  - output goes to the recording folder's `Montages/` folder.

## UI (Montages view)
- **List:** saved montages with thumbnails, a "New montage" button, and "Scan a recording…".
- **New montage wizard:**
  1. pick clips (multi-select in the clip grid) and/or recordings to scan, then choose which
     candidates to keep;
  2. choose music, target length and format;
  3. press Create, which opens the editor.
- **Editor:**
  - preview at the top, with transport controls and a format switch;
  - the timeline below:
    - segment blocks with thumbnails and event ticks;
    - transition chips between blocks;
    - a title lane;
    - the music lane with a waveform and beat ticks;
    - zoom with Ctrl+wheel;
  - an inspector on the right for the selected segment, transition, title or music.
- **Keyboard:**
  - Space: play;
  - J/K/L: shuttle;
  - S: split;
  - Del: delete;
  - Ctrl+Z / Ctrl+Y: undo/redo;
  - arrows: step one frame;
  - all controls can be reached with the keyboard and have screen-reader labels.
- **Export dialog:** preset, codec, quality, file name, and "Add to clip library".

## Risks
- **Decoder count:** a 4K HEVC source plus a crossfade means two hardware decodes plus encode.
  The preview falls back to half resolution when decoding can't keep up.
- **Variable frame rate sources:** recordings are CFR. Clips imported from elsewhere may be VFR,
  so segments are resampled to the montage fps by timestamp.
- **Scope creep toward a full NLE** is held back by the project's narrowed non-goal.
